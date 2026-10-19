use super::*;

impl Store {
    #[cfg(test)]
    pub fn persist_fetched_feed(
        &mut self,
        url: &str,
        id: Option<i64>,
        fetched: FetchedFeed,
    ) -> anyhow::Result<()> {
        if let Some(feed) = fetched.parsed {
            self.save_feed(url, feed, id, fetched.etag, fetched.modified)
        } else if let Some(id) = id {
            self.connection.execute(
                "UPDATE feeds SET last_error=NULL WHERE id=?1 AND workspace=?2",
                params![id, self.workspace],
            )?;
            Ok(())
        } else {
            anyhow::bail!("Feed has no content yet")
        }
    }

    pub fn prepare_fetched_feed(
        url: &str,
        fetched: FetchedFeed,
    ) -> anyhow::Result<PreparedFeedResponse> {
        Self::prepare_fetched_feed_with_revisions(url, fetched, &HashMap::new())
    }

    pub fn prepare_fetched_feed_with_revisions(
        url: &str,
        fetched: FetchedFeed,
        known_revisions: &HashMap<String, String>,
    ) -> anyhow::Result<PreparedFeedResponse> {
        let parsed = fetched
            .parsed
            .map(|feed| prepare_feed(url, feed, known_revisions))
            .transpose()?;
        Ok(PreparedFeedResponse {
            parsed,
            etag: fetched.etag,
            modified: fetched.modified,
        })
    }

    pub fn persist_prepared_feed(
        &mut self,
        url: &str,
        id: Option<i64>,
        response: PreparedFeedResponse,
    ) -> anyhow::Result<PersistedFeedUpdate> {
        if let Some(feed) = response.parsed {
            let changed_article_ids =
                self.write_prepared_feed(url, feed, id, response.etag, response.modified)?;
            Ok(PersistedFeedUpdate {
                feed_changed: true,
                changed_article_ids,
            })
        } else if let Some(id) = id {
            self.connection.execute(
                "UPDATE feeds SET last_error=NULL WHERE id=?1 AND workspace=?2",
                params![id, self.workspace],
            )?;
            Ok(PersistedFeedUpdate {
                feed_changed: false,
                changed_article_ids: Vec::new(),
            })
        } else {
            anyhow::bail!("Feed has no content yet")
        }
    }

    pub fn set_feed_refresh_error(&self, id: i64, error: Option<&str>) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE feeds SET last_error=?1 WHERE id=?2 AND workspace=?3",
            params![error, id, self.workspace],
        )?;
        Ok(())
    }

    pub fn parse_opml_feeds(
        source: &str,
    ) -> anyhow::Result<Vec<(String, Option<String>, Option<String>)>> {
        let document = OPML::from_str(source)?;
        let mut entries = Vec::new();
        for outline in &document.body.outlines {
            collect_outlines(outline, None, &mut entries);
        }
        Ok(entries)
    }

    #[cfg(test)]
    pub fn persist_imported_feed(
        &mut self,
        url: &str,
        fetched: FetchedFeed,
        title: Option<String>,
        folder: Option<String>,
    ) -> anyhow::Result<bool> {
        if self.has_feed_url(url)? {
            return Ok(false);
        }
        let Some(feed) = fetched.parsed else {
            return Ok(false);
        };
        self.save_feed(url, feed, None, fetched.etag, fetched.modified)?;
        if let Some(folder) = folder.filter(|value| !value.trim().is_empty()) {
            self.connection.execute(
                "UPDATE feeds SET folder=?1 WHERE feed_url=?2 AND workspace=?3",
                params![folder, url, self.workspace],
            )?;
        }
        if let Some(title) = title.filter(|value| !value.trim().is_empty()) {
            self.connection.execute(
                "UPDATE feeds SET title=?1 WHERE feed_url=?2 AND workspace=?3",
                params![title, url, self.workspace],
            )?;
        }
        Ok(true)
    }

    pub fn persist_imported_feed_response(
        &mut self,
        url: &str,
        response: PreparedFeedResponse,
        title: Option<String>,
        folder: Option<String>,
    ) -> anyhow::Result<bool> {
        if self.has_feed_url(url)? {
            return Ok(false);
        }
        let Some(feed) = response.parsed else {
            return Ok(false);
        };
        self.write_prepared_feed(url, feed, None, response.etag, response.modified)
            .map(|_| ())?;
        if let Some(folder) = folder.filter(|value| !value.trim().is_empty()) {
            self.connection.execute(
                "UPDATE feeds SET folder=?1 WHERE feed_url=?2 AND workspace=?3",
                params![folder, url, self.workspace],
            )?;
        }
        if let Some(title) = title.filter(|value| !value.trim().is_empty()) {
            self.connection.execute(
                "UPDATE feeds SET title=?1 WHERE feed_url=?2 AND workspace=?3",
                params![title, url, self.workspace],
            )?;
        }
        Ok(true)
    }

    pub fn update_feed(
        &self,
        id: i64,
        title: &str,
        folder: Option<&str>,
        feed_url: &str,
        auto_translate_titles: bool,
    ) -> anyhow::Result<()> {
        let title = title.trim();
        let feed_url = feed_url.trim();
        anyhow::ensure!(!title.is_empty(), "Feed title cannot be empty");
        anyhow::ensure!(!feed_url.is_empty(), "Feed URL cannot be empty");
        let folder = folder
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        self.connection.execute(
            "UPDATE feeds SET custom_title=?1, folder=?2, feed_url=?3, auto_translate_titles=?4,
                auto_translate_titles_after_article_id=CASE
                    WHEN ?4=1 AND auto_translate_titles=0 THEN COALESCE((SELECT MAX(id) FROM articles WHERE feed_id=feeds.id),0)
                    WHEN ?4=0 THEN NULL ELSE auto_translate_titles_after_article_id END
             WHERE id=?5 AND workspace=?6",
            params![title, folder, feed_url, auto_translate_titles, id, self.workspace],
        )?;
        Ok(())
    }

    pub fn set_feed_auto_translate_titles(&self, id: i64, enabled: bool) -> anyhow::Result<()> {
        let changed = self.connection.execute(
            "UPDATE feeds SET auto_translate_titles=?1,
                auto_translate_titles_after_article_id=CASE
                    WHEN ?1=1 AND auto_translate_titles=0 THEN COALESCE((SELECT MAX(id) FROM articles WHERE feed_id=feeds.id),0)
                    WHEN ?1=0 THEN NULL ELSE auto_translate_titles_after_article_id END
             WHERE id=?2 AND workspace=?3",
            params![enabled, id, self.workspace],
        )?;
        anyhow::ensure!(changed == 1, "Feed not found in this workspace");
        Ok(())
    }

    pub fn reset_auto_title_translation_cutoffs(&self) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE feeds SET auto_translate_titles_after_article_id=COALESCE((SELECT MAX(id) FROM articles WHERE feed_id=feeds.id),0) WHERE workspace=?1 AND auto_translate_titles=1",
            [&self.workspace],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn refresh_feed(&mut self, id: i64) -> anyhow::Result<usize> {
        let (url, etag, modified, source): (String, Option<String>, Option<String>, String) =
            self.connection.query_row(
                "SELECT feed_url,etag,last_modified,source FROM feeds WHERE id=?1 AND workspace=?2",
                params![id, self.workspace],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        anyhow::ensure!(
            source == "local",
            "Remote feeds refresh through provider sync"
        );
        match self
            .fetch_feed(&url, etag.as_deref(), modified.as_deref())
            .await
        {
            Ok((Some(feed), new_etag, new_modified)) => {
                self.save_feed(&url, feed, Some(id), new_etag, new_modified)?;
                Ok(1)
            }
            Ok((None, _, _)) => {
                let _ = self.connection.execute(
                    "UPDATE feeds SET last_error=NULL WHERE id=?1 AND workspace=?2",
                    params![id, self.workspace],
                );
                Ok(0)
            }
            Err(error) => {
                let _ = self.connection.execute(
                    "UPDATE feeds SET last_error=?1 WHERE id=?2 AND workspace=?3",
                    params![error.to_string(), id, self.workspace],
                );
                Err(error)
            }
        }
    }

    #[cfg(test)]
    pub async fn add_feed(&mut self, feed_url: &str) -> anyhow::Result<()> {
        let url = normalize_http_url(feed_url)?;
        if self
            .connection
            .query_row(
                "SELECT 1 FROM feeds WHERE feed_url=?1 AND workspace=?2",
                params![url, self.workspace],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Ok(());
        }
        let (parsed, _, _) = self.fetch_feed(&url, None, None).await?;
        let parsed = parsed.ok_or_else(|| anyhow::anyhow!("Feed has no content yet"))?;
        self.save_feed(&url, parsed, None, None, None)?;
        Ok(())
    }

    pub fn remove_feed(&mut self, feed_id: i64) -> anyhow::Result<()> {
        self.clear_pending_for_feed(feed_id)?;
        self.connection.execute(
            "DELETE FROM feeds WHERE id=?1 AND workspace=?2",
            params![feed_id, self.workspace],
        )?;
        Ok(())
    }

    pub(super) fn clear_pending_for_feed(&self, feed_id: i64) -> anyhow::Result<()> {
        self.connection.execute(
            "DELETE FROM pending_remote_marks WHERE workspace=?1 AND remote_id IN
             (SELECT remote_id FROM articles WHERE feed_id=?2 AND remote_id IS NOT NULL)",
            params![self.workspace, feed_id],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn refresh_all(&mut self) -> anyhow::Result<usize> {
        let feeds = {
            let mut statement = self.connection.prepare(
                "SELECT id,feed_url,etag,last_modified FROM feeds WHERE source='local' AND workspace=?1 ORDER BY id",
            )?;
            statement
                .query_map([&self.workspace], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut total = 0;
        let mut failures = Vec::new();
        for (id, url, etag, modified) in feeds {
            match self
                .fetch_feed(&url, etag.as_deref(), modified.as_deref())
                .await
            {
                Ok((Some(feed), new_etag, new_modified)) => {
                    if let Err(error) = self.save_feed(&url, feed, Some(id), new_etag, new_modified)
                    {
                        failures.push(format!("{url}: {error}"));
                    } else {
                        total += 1;
                    }
                }
                Ok((None, _, _)) => {
                    let _ = self.connection.execute(
                        "UPDATE feeds SET last_error=NULL WHERE id=?1 AND workspace=?2",
                        params![id, self.workspace],
                    );
                }
                Err(error) => {
                    let _ = self.connection.execute(
                        "UPDATE feeds SET last_error=?1 WHERE id=?2 AND workspace=?3",
                        params![error.to_string(), id, self.workspace],
                    );
                    failures.push(format!("{url}: {error}"));
                }
            }
        }
        if total == 0 && !failures.is_empty() {
            anyhow::bail!("All feeds failed to refresh: {}", failures.join("; "));
        }
        Ok(total)
    }

    #[cfg(test)]
    pub(super) async fn fetch_feed(
        &self,
        url: &str,
        etag: Option<&str>,
        modified: Option<&str>,
    ) -> anyhow::Result<(Option<ParsedFeed>, Option<String>, Option<String>)> {
        let mut request = self.client.get(url);
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(modified) = modified {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, modified);
        }
        let response = request.send().await?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok((None, etag.map(str::to_owned), modified.map(str::to_owned)));
        }
        let response = response.error_for_status()?;
        let next_etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let next_modified = response
            .headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await?;
        let feed = feed_rs::parser::parse(bytes.as_ref())?;
        Ok((Some(feed), next_etag, next_modified))
    }

    #[cfg(test)]
    pub(super) fn save_feed(
        &mut self,
        url: &str,
        feed: ParsedFeed,
        existing_id: Option<i64>,
        etag: Option<String>,
        modified: Option<String>,
    ) -> anyhow::Result<()> {
        self.write_prepared_feed(
            url,
            prepare_feed(url, feed, &HashMap::new())?,
            existing_id,
            etag,
            modified,
        )
        .map(|_| ())
    }

    fn write_prepared_feed(
        &mut self,
        url: &str,
        feed: PreparedFeed,
        existing_id: Option<i64>,
        etag: Option<String>,
        modified: Option<String>,
    ) -> anyhow::Result<Vec<i64>> {
        let PreparedFeed {
            title,
            site_url,
            articles,
        } = feed;
        let id = if let Some(id) = existing_id {
            self.connection.execute("UPDATE feeds SET title=?1,site_url=COALESCE(?2,site_url),etag=COALESCE(?3,etag),last_modified=COALESCE(?4,last_modified),last_error=NULL WHERE id=?5 AND workspace=?6", params![title,site_url,etag,modified,id,self.workspace])?;
            id
        } else {
            self.connection.execute("INSERT INTO feeds(feed_url,title,site_url,etag,last_modified,workspace) VALUES(?1,?2,?3,?4,?5,?6)", params![url,title,site_url,etag,modified,self.workspace])?;
            self.connection.last_insert_rowid()
        };
        let mut changed_article_ids = Vec::new();
        let mut entries = articles.into_iter().peekable();
        while entries.peek().is_some() {
            let transaction = self.connection.transaction()?;
            for _ in 0..100 {
                let Some((parsed, compressed_source, source_revision)) = entries.next() else {
                    break;
                };
                transaction.execute(
                    "INSERT INTO articles(feed_id,guid,title,url,author,published_at,snippet,content_html,source_html,source_revision)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(feed_id,guid) DO UPDATE SET title=excluded.title,url=excluded.url,author=excluded.author,
                 published_at=COALESCE(excluded.published_at,articles.published_at),snippet=excluded.snippet,
                 content_html=CASE WHEN excluded.content_html='' THEN articles.content_html ELSE excluded.content_html END,
                 source_html=CASE WHEN excluded.source_html IS NULL OR excluded.source_html=X'' THEN articles.source_html ELSE excluded.source_html END,
                 source_revision=excluded.source_revision,
                 content_revision=articles.content_revision+1
                 WHERE articles.source_revision IS NOT excluded.source_revision OR articles.title IS NOT excluded.title OR articles.url IS NOT excluded.url
                    OR articles.author IS NOT excluded.author OR articles.published_at IS NOT COALESCE(excluded.published_at,articles.published_at)
                    OR articles.snippet IS NOT excluded.snippet
                    OR (excluded.content_html<>'' AND articles.content_html IS NOT excluded.content_html)
                    OR (excluded.source_html IS NOT NULL AND excluded.source_html<>X'' AND articles.source_html IS NOT excluded.source_html)",
                    params![id,parsed.guid,parsed.title,parsed.url,parsed.author,parsed.published_at,parsed.snippet,parsed.content_html,compressed_source,source_revision],
                )?;
                let article_id: i64 = transaction.query_row(
                    "SELECT id FROM articles WHERE feed_id=?1 AND guid=?2",
                    params![id, parsed.guid],
                    |row| row.get(0),
                )?;
                if transaction.changes() > 0 {
                    sync_article_fts(&transaction, article_id)?;
                    changed_article_ids.push(article_id);
                }
            }
            transaction.commit()?;
        }
        Ok(changed_article_ids)
    }
}

impl FetchedFeed {
    pub fn from_parsed(parsed: ParsedFeed, etag: Option<String>, modified: Option<String>) -> Self {
        Self {
            parsed: Some(parsed),
            etag,
            modified,
        }
    }

    pub fn not_modified(etag: Option<String>, modified: Option<String>) -> Self {
        Self {
            parsed: None,
            etag,
            modified,
        }
    }
}
