use super::*;

impl Store {
    pub fn extraction_input(
        &self,
        article_id: i64,
        force: bool,
    ) -> anyhow::Result<Option<ExtractionInput>> {
        let (existing_html, existing_hash, cached_page_html, cached_page_url, cached_page_content_type, source_revision): (
            Option<String>,
            Option<String>,
            Option<Vec<u8>>,
            Option<String>,
            Option<String>,
            i64,
        ) = self.connection.query_row(
            "SELECT a.extracted_html,a.extraction_pipeline_hash,a.source_page_html,a.source_page_url,
                    a.source_page_content_type,a.content_revision
             FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.id=?1 AND f.workspace=?2",
            params![article_id, self.workspace],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )?;
        if !force
            && existing_html
                .as_deref()
                .is_some_and(|html| !html.trim().is_empty())
            && existing_hash.is_none()
            && cached_page_html.is_none()
        {
            return Ok(None);
        }
        let (url, title): (String, String) = self.connection.query_row(
            "SELECT a.url,a.title FROM articles a JOIN feeds f ON f.id=a.feed_id
             WHERE a.id=?1 AND f.workspace=?2",
            params![article_id, self.workspace],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let cached_page_html = if !force {
            cached_page_html
                .map(|html| decompress_html(&html))
                .transpose()?
        } else {
            None
        };
        Ok(Some(ExtractionInput {
            workspace: self.workspace.clone(),
            article_id,
            source_revision,
            url,
            title,
            existing_html,
            existing_hash,
            cached_page_html,
            cached_page_url,
            cached_page_content_type,
        }))
    }

    pub fn prepare_extraction_from_page(
        &self,
        input: ExtractionInput,
        resolved_url: String,
        source_page_url: String,
        source_page_content_type: String,
        raw: String,
        extractor: ContentExtractor,
        pipeline_hash: &str,
        prepare: impl FnOnce(&str, &str, &str) -> anyhow::Result<Option<(String, bool)>>,
    ) -> anyhow::Result<Option<PreparedExtraction>> {
        anyhow::ensure!(
            input.workspace == self.workspace,
            "extraction input belongs to a different workspace"
        );
        let mut cache_hasher = Sha256::new();
        cache_hasher.update(pipeline_hash.as_bytes());
        cache_hasher.update(source_page_url.as_bytes());
        cache_hasher.update(Sha256::digest(raw.as_bytes()));
        let pipeline_hash = hex::encode(cache_hasher.finalize());
        if input
            .existing_html
            .as_deref()
            .is_some_and(|html| !html.trim().is_empty())
            && input.existing_hash.as_deref() == Some(pipeline_hash.as_str())
        {
            return Ok(None);
        }
        let compressed_raw = compress_html(&raw)?;
        let prepared = prepare(&raw, &source_page_url, &input.title)?;
        let (sanitized, body_selected) = match prepared {
            Some((html, true)) => (
                panda_content::sanitize_html(&html, Some(&source_page_url)),
                true,
            ),
            Some((html, false)) => (
                extract_article_html(&html, &source_page_url, &input.title, extractor),
                false,
            ),
            None => (
                extract_article_html(&raw, &source_page_url, &input.title, extractor),
                false,
            ),
        };
        let sanitized = if body_selected && plain_text(&sanitized).trim().len() < 80 {
            extract_article_html(&raw, &source_page_url, &input.title, extractor)
        } else {
            sanitized
        };
        if plain_text(&sanitized).trim().len() < 80 {
            anyhow::bail!("No extractable article content found on this page");
        }
        Ok(Some(PreparedExtraction {
            workspace: input.workspace,
            article_id: input.article_id,
            source_revision: input.source_revision,
            resolved_url,
            source_page_url,
            source_page_content_type,
            source_page_html: compressed_raw,
            extracted_html: sanitized,
            pipeline_hash,
        }))
    }

    #[cfg(test)]
    pub async fn extract(
        &mut self,
        article_id: i64,
        force: bool,
        extractor: ContentExtractor,
    ) -> anyhow::Result<()> {
        self.extract_using(article_id, force, extractor, "legacy", |_, _, _| Ok(None))
            .await
    }

    #[cfg(test)]
    pub async fn extract_using(
        &mut self,
        article_id: i64,
        force: bool,
        extractor: ContentExtractor,
        pipeline_hash: &str,
        prepare: impl FnOnce(&str, &str, &str) -> anyhow::Result<Option<(String, bool)>>,
    ) -> anyhow::Result<()> {
        if let Some(prepared) = self
            .prepare_extraction_using(
                article_id,
                force,
                extractor,
                pipeline_hash,
                |url| Ok(url.to_owned()),
                prepare,
            )
            .await?
        {
            self.persist_extraction(prepared)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub async fn prepare_extraction_using(
        &self,
        article_id: i64,
        force: bool,
        extractor: ContentExtractor,
        pipeline_hash: &str,
        resolve_url: impl FnOnce(&str) -> anyhow::Result<String>,
        prepare: impl FnOnce(&str, &str, &str) -> anyhow::Result<Option<(String, bool)>>,
    ) -> anyhow::Result<Option<PreparedExtraction>> {
        let Some(input) = self.extraction_input(article_id, force)? else {
            return Ok(None);
        };
        let raw = input
            .cached_page_html
            .clone()
            .ok_or_else(|| anyhow::anyhow!("test extraction requires a cached source page"))?;
        let resolved = if input.cached_page_html.is_some() {
            input.url.clone()
        } else {
            resolve_url(&input.url)?
        };
        let source_page_url = input
            .cached_page_url
            .clone()
            .unwrap_or_else(|| resolved.clone());
        let content_type = input.cached_page_content_type.clone().unwrap_or_default();
        self.prepare_extraction_from_page(
            input,
            resolved,
            source_page_url,
            content_type,
            raw,
            extractor,
            pipeline_hash,
            prepare,
        )
    }

    pub fn persist_extraction(&self, prepared: PreparedExtraction) -> anyhow::Result<bool> {
        let changed = self.connection.execute(
            "UPDATE articles SET url=?1,source_page_html=?2,source_page_url=?3,source_page_content_type=?4,
                extracted_html=?5, extraction_pipeline_hash=?6,
                processed_html=NULL, processed_source_hash=NULL, processed_pipeline_hash=NULL,
                translated_html=NULL, translated_title=NULL, translated_lang=NULL, translation_source_hash=NULL,
                content_revision=content_revision+1 WHERE id=?7 AND content_revision=?8
                AND EXISTS(SELECT 1 FROM feeds f WHERE f.id=articles.feed_id AND f.workspace=?9)",
            params![prepared.resolved_url, prepared.source_page_html, prepared.source_page_url,
                prepared.source_page_content_type, prepared.extracted_html, prepared.pipeline_hash,
                prepared.article_id, prepared.source_revision, prepared.workspace],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        sync_article_fts(&self.connection, prepared.article_id)?;
        Ok(true)
    }
}
