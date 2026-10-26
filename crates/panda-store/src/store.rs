use anyhow::Context as _;
mod articles;
mod backup_config;
pub use backup_config::BackupFeed;
mod extraction;
mod feeds;
mod fts;
mod opml;
mod provider;
mod schema;
mod translation;
use ::opml::{Head, OPML, Outline};
use feed_rs::model::{Entry, Feed as ParsedFeed};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use fts::{fts_query, sync_article_fts};
use panda_content::{extract_article_html, plain_text, sanitize_html};
use panda_core::{
    Article, ArticleCursor, ArticleSummary, ContentExtractor, Feed, MarkField, ParsedArticle,
    ReaderSnapshot, Scope,
};
#[cfg(test)]
use panda_providers::{ProviderClient, SyncMode};
use panda_providers::{ProviderKind, RemoteEntry, RemoteFeed, SyncCursor};
#[cfg(test)]
use reqwest::Client;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
#[cfg(test)]
use std::sync::OnceLock;
use std::time::Duration;

pub const CURRENT_SCHEMA_VERSION: i64 = 7;
use url::Url;

pub struct Store {
    connection: Connection,
    #[cfg(test)]
    client: Client,
    workspace: String,
}

pub struct FetchedFeed {
    parsed: Option<ParsedFeed>,
    etag: Option<String>,
    modified: Option<String>,
}

pub struct PreparedFeedResponse {
    parsed: Option<PreparedFeed>,
    etag: Option<String>,
    modified: Option<String>,
}

impl PreparedFeedResponse {
    pub fn is_not_modified(&self) -> bool {
        self.parsed.is_none()
    }
}

struct PreparedFeed {
    title: String,
    site_url: Option<String>,
    articles: Vec<(ParsedArticle, Vec<u8>, String)>,
}

impl FetchedFeed {
    pub fn is_not_modified(&self) -> bool {
        self.parsed.is_none()
    }
}

#[derive(Clone, Debug)]
pub struct FeedRefreshInput {
    pub id: i64,
    pub url: String,
    pub etag: Option<String>,
    pub modified: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PendingRemoteMark {
    pub remote_id: i64,
    pub field: String,
    pub value: bool,
    pub revision: i64,
}

pub struct PreparedExtraction {
    pub workspace: String,
    pub article_id: i64,
    pub source_revision: i64,
    pub resolved_url: String,
    pub source_page_url: String,
    pub source_page_content_type: String,
    pub source_page_html: Vec<u8>,
    pub extracted_html: String,
    pub pipeline_hash: String,
}

pub struct ExtractionInput {
    pub workspace: String,
    pub article_id: i64,
    pub source_revision: i64,
    pub url: String,
    pub title: String,
    pub existing_html: Option<String>,
    pub existing_hash: Option<String>,
    pub cached_page_html: Option<String>,
    pub cached_page_url: Option<String>,
    pub cached_page_content_type: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProviderSyncState {
    pub provider: String,
    pub account: String,
    pub cursor: Option<SyncCursor>,
    pub last_full_sync_at: Option<String>,
}

pub struct PreparedRemoteEntry {
    entry: RemoteEntry,
    guid: String,
    remote_hash: String,
    html: String,
    snippet: String,
    compressed_source: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct SavedRemoteEntries {
    pub count: usize,
    pub changed_article_ids: Vec<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct PersistedFeedUpdate {
    pub feed_changed: bool,
    pub changed_article_ids: Vec<i64>,
}

#[cfg(test)]
fn shared_http_client() -> Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent("PandaReader/0.1 (+https://github.com)")
                .timeout(Duration::from_secs(25))
                .build()
                .expect("store HTTP client configuration is valid")
        })
        .clone()
}

impl Store {
    #[cfg(test)]
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Self::open_writer(path, "local")
    }

    /// Apply versioned schema changes once before starting the writable connection.
    /// Open the one writable connection owned by the application's DB writer.
    pub fn open_writer(path: &Path, workspace: &str) -> anyhow::Result<Self> {
        #[cfg(test)]
        Self::migrate(path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            connection.pragma_update(None, "journal_mode", "WAL")?;
        }
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let schema_version: i64 =
            connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if schema_version < CURRENT_SCHEMA_VERSION {
            anyhow::bail!("database schema migration is required before opening the writer");
        }
        Ok(Self {
            connection,
            #[cfg(test)]
            client: shared_http_client(),
            workspace: workspace.to_owned(),
        })
    }

    /// Open an independent read-only connection. All mutations must be sent
    /// through the application's serialized database writer.
    pub fn open_read_workspace(path: &Path, workspace: &str) -> anyhow::Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(Self {
            connection,
            #[cfg(test)]
            client: shared_http_client(),
            workspace: workspace.to_owned(),
        })
    }

    /// Change the workspace used by the writer-owned connection.
    pub fn set_workspace(&mut self, workspace: &str) {
        self.workspace.clear();
        self.workspace.push_str(workspace);
    }

    pub fn local_feeds_to_refresh(&self) -> anyhow::Result<Vec<FeedRefreshInput>> {
        let mut statement = self.connection.prepare(
            "SELECT id,feed_url,etag,last_modified FROM feeds WHERE source='local' AND workspace=?1 ORDER BY id",
        )?;
        Ok(statement
            .query_map([&self.workspace], |row| {
                Ok(FeedRefreshInput {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    etag: row.get(2)?,
                    modified: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn local_feed_refresh_input(&self, id: i64) -> anyhow::Result<FeedRefreshInput> {
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
        Ok(FeedRefreshInput {
            id,
            url,
            etag,
            modified,
        })
    }

    pub fn article_source_revisions(
        &self,
        feed_id: i64,
    ) -> anyhow::Result<HashMap<String, String>> {
        let mut statement = self.connection.prepare(
            "SELECT guid,source_revision FROM articles WHERE feed_id=?1 AND source_revision IS NOT NULL",
        )?;
        Ok(statement
            .query_map([feed_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<HashMap<_, _>, _>>()?)
    }

    pub fn remote_content_revisions(&self) -> anyhow::Result<HashMap<i64, String>> {
        let mut statement = self.connection.prepare(
            "SELECT a.remote_id,COALESCE(a.source_revision,a.remote_content_hash) FROM articles a JOIN feeds f ON f.id=a.feed_id
             WHERE f.workspace=?1 AND a.remote_id IS NOT NULL AND a.remote_content_hash IS NOT NULL",
        )?;
        Ok(statement
            .query_map([&self.workspace], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<HashMap<_, _>, _>>()?)
    }

    pub fn remote_content_revisions_for_ids(
        &self,
        remote_ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, String>> {
        if remote_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut revisions = HashMap::with_capacity(remote_ids.len());
        for chunk in remote_ids.chunks(500) {
            let mut sql = String::from(
                "SELECT a.remote_id,COALESCE(a.source_revision,a.remote_content_hash) FROM articles a JOIN feeds f ON f.id=a.feed_id
                 WHERE f.workspace=?1 AND a.remote_id IS NOT NULL AND a.remote_content_hash IS NOT NULL
                 AND a.remote_id IN (",
            );
            for index in 0..chunk.len() {
                if index > 0 {
                    sql.push(',');
                }
                sql.push('?');
            }
            sql.push(')');
            let mut statement = self.connection.prepare(&sql)?;
            let mut params: Vec<rusqlite::types::Value> =
                Vec::with_capacity(chunk.len().saturating_add(1));
            params.push(self.workspace.clone().into());
            params.extend(chunk.iter().map(|id| rusqlite::types::Value::Integer(*id)));
            let rows = statement.query_map(rusqlite::params_from_iter(params), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
            for row in rows {
                let (remote_id, revision) = row?;
                revisions.insert(remote_id, revision);
            }
        }
        Ok(revisions)
    }

    pub fn has_feed_url(&self, url: &str) -> anyhow::Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM feeds WHERE feed_url=?1 AND workspace=?2)",
            params![url, self.workspace],
            |row| row.get(0),
        )?)
    }

    pub fn snapshot(
        &self,
        scope: Scope,
        search: &str,
        limit: i64,
        after: Option<&ArticleCursor>,
        include_feeds: bool,
    ) -> anyhow::Result<ReaderSnapshot> {
        let feeds = if include_feeds {
            let mut feeds_statement = self.connection.prepare(
                "SELECT f.id, COALESCE(f.custom_title,f.title), f.feed_url, f.site_url, f.folder,
                        COUNT(a.id), f.last_error, f.auto_translate_titles
                 FROM feeds f LEFT JOIN articles a ON a.feed_id=f.id AND a.is_read=0
                 WHERE f.workspace=?1
                 GROUP BY f.id ORDER BY COALESCE(f.folder,''), COALESCE(f.custom_title,f.title) COLLATE NOCASE",
            )?;
            feeds_statement
                .query_map([&self.workspace], |row| {
                    Ok(Feed {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        feed_url: row.get(2)?,
                        site_url: row.get(3)?,
                        folder: row.get(4)?,
                        unread: row.get(5)?,
                        last_error: row.get(6)?,
                        auto_translate_titles: row.get::<_, i64>(7)? != 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };

        enum ScopeBind {
            None,
            FeedId(i64),
            Folder(String),
        }
        let (filter, bind) = match scope {
            Scope::All => ("1=1", ScopeBind::None),
            Scope::Unread => ("a.is_read=0", ScopeBind::None),
            Scope::Starred => ("a.is_starred=1", ScopeBind::None),
            Scope::Later => ("a.read_later=1", ScopeBind::None),
            Scope::Feed(id) => ("a.feed_id=?", ScopeBind::FeedId(id)),
            Scope::Folder(name) => ("f.folder=?", ScopeBind::Folder(name)),
        };
        let search_query = fts_query(search);
        let limit = limit.max(1);
        let cursor_pub = after
            .map(|cursor| cursor.published_at.clone().unwrap_or_default())
            .unwrap_or_default();
        let cursor_id = after.map(|cursor| cursor.id).unwrap_or(0);
        let use_cursor = after.is_some();
        // Keyset: rows strictly after (published_at DESC, id DESC) cursor.
        let cursor_filter = if use_cursor {
            "AND (COALESCE(a.published_at,'') < ? OR (COALESCE(a.published_at,'') = ? AND a.id < ?))"
        } else {
            ""
        };
        // FTS stores sanitized plain text, keeping HTML parsing out of the read path.
        let search_filter = if search_query.is_empty() {
            ""
        } else {
            "AND a.id IN (SELECT rowid FROM article_fts WHERE article_fts MATCH ?)"
        };
        let sql = format!(
            "SELECT a.id,COALESCE(f.custom_title,f.title),a.title,a.url,a.author,a.snippet,a.published_at,a.is_read,a.is_starred,a.read_later,a.auto_translated_title,a.auto_translated_title_lang,a.auto_translated_title_source_hash,CASE WHEN f.source='miniflux' THEN f.language ELSE NULL END,f.auto_translate_titles,a.feed_id,a.id>COALESCE(f.auto_translate_titles_after_article_id,0)
             FROM articles a JOIN feeds f ON f.id=a.feed_id
                 WHERE f.workspace=? AND {filter} {search_filter}
             {cursor_filter}
             ORDER BY COALESCE(a.published_at,'') DESC,a.id DESC LIMIT {limit}"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(ArticleSummary {
                id: row.get(0)?,
                feed_title: row.get(1)?,
                feed_language: row.get(13)?,
                feed_auto_translate_titles: row.get::<_, i64>(14)? != 0,
                title_is_future: row.get(16)?,
                title: row.get(2)?,
                url: row.get(3)?,
                author: row.get(4)?,
                snippet: row.get(5)?,
                published_at: row.get(6)?,
                is_read: row.get(7)?,
                is_starred: row.get(8)?,
                read_later: row.get(9)?,
                auto_translated_title: row.get(10)?,
                auto_translated_title_lang: row.get(11)?,
                auto_translated_title_source_hash: row.get(12)?,
                feed_id: row.get(15)?,
            })
        };
        let mut params: Vec<rusqlite::types::Value> = Vec::new();
        params.push(self.workspace.clone().into());
        match &bind {
            ScopeBind::FeedId(feed_id) => params.push((*feed_id).into()),
            ScopeBind::Folder(folder) => params.push(folder.clone().into()),
            ScopeBind::None => {}
        }
        if !search_query.is_empty() {
            params.push(search_query.into());
        }
        if use_cursor {
            params.push(cursor_pub.clone().into());
            params.push(cursor_pub.into());
            params.push(cursor_id.into());
        }
        let articles = statement
            .query_map(rusqlite::params_from_iter(params), map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = articles.len() as i64 >= limit;
        Ok(ReaderSnapshot {
            feeds,
            articles,
            has_more,
        })
    }

    /// Index a bounded batch so an older library can become searchable without
    /// holding up application startup or monopolizing the SQLite connection.
    pub fn index_search_batch(&mut self, batch_size: usize) -> anyhow::Result<bool> {
        let transaction = self.connection.transaction()?;
        let (last_id, complete): (i64, bool) = transaction.query_row(
            "SELECT last_article_id,complete FROM article_fts_state WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
        )?;
        if complete {
            return Ok(true);
        }
        let rows = {
            let mut statement =
                transaction.prepare("SELECT id FROM articles WHERE id>?1 ORDER BY id LIMIT ?2")?;
            statement
                .query_map(params![last_id, batch_size.max(1) as i64], |row| {
                    row.get::<_, i64>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        if rows.is_empty() {
            transaction.execute("UPDATE article_fts_state SET complete=1 WHERE id=1", [])?;
            transaction.commit()?;
            return Ok(true);
        }
        for article_id in &rows {
            sync_article_fts(&transaction, *article_id)?;
        }
        transaction.execute(
            "UPDATE article_fts_state SET last_article_id=?1 WHERE id=1",
            [rows.last().copied().unwrap_or(last_id)],
        )?;
        transaction.commit()?;
        Ok(false)
    }

    pub fn article(&self, id: i64) -> anyhow::Result<Article> {
        let (summary, content_revision, url, content_html, source_html, extracted_html,
            translated_html, translated_title, translated_lang, translation_source_hash) = self.connection.query_row(
            "SELECT a.id,COALESCE(f.custom_title,f.title),a.title,a.url,a.author,a.snippet,a.published_at,
                    a.is_read,a.is_starred,a.read_later,a.auto_translated_title,a.auto_translated_title_lang,
                    a.auto_translated_title_source_hash,CASE WHEN f.source='miniflux' THEN f.language ELSE NULL END,
                    f.auto_translate_titles,a.feed_id,a.content_revision,a.content_html,
                    CASE WHEN a.extracted_html IS NULL THEN a.source_html ELSE NULL END,a.extracted_html,
                    a.translated_html,a.translated_title,a.translated_lang,a.translation_source_hash,
                    a.id>COALESCE(f.auto_translate_titles_after_article_id,0)
             FROM articles a JOIN feeds f ON f.id=a.feed_id
             WHERE a.id=?1 AND f.workspace=?2",
            params![id, self.workspace],
            |row| Ok((
                ArticleSummary {
                    id: row.get(0)?, feed_title: row.get(1)?, title: row.get(2)?, url: row.get(3)?,
                    author: row.get(4)?, snippet: row.get(5)?, published_at: row.get(6)?,
                    is_read: row.get(7)?, is_starred: row.get(8)?, read_later: row.get(9)?,
                    auto_translated_title: row.get(10)?, auto_translated_title_lang: row.get(11)?,
                    auto_translated_title_source_hash: row.get(12)?, feed_language: row.get(13)?,
                    feed_auto_translate_titles: row.get::<_, i64>(14)? != 0, feed_id: row.get(15)?,
                    title_is_future: row.get(24)?,
                },
                row.get::<_, i64>(16)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(17)?,
                row.get::<_, Option<Vec<u8>>>(18)?,
                row.get::<_, Option<String>>(19)?,
                row.get::<_, Option<String>>(20)?,
                row.get::<_, Option<String>>(21)?,
                row.get::<_, Option<String>>(22)?,
                row.get::<_, Option<String>>(23)?,
            )),
        )?;
        Ok(Article {
            summary,
            content_revision,
            url,
            content_html,
            source_html: source_html
                .map(|bytes| decompress_html(&bytes))
                .transpose()?,
            source_page_html: None,
            extracted_html,
            canonical: None,
            translated_html,
            translated_title,
            translated_lang,
            translation_source_hash,
        })
    }

    pub fn reading_progress(&self, article_id: i64) -> anyhow::Result<f32> {
        self.connection
            .query_row(
                "SELECT progress FROM article_reading_positions WHERE workspace=?1 AND article_id=?2",
                params![self.workspace, article_id],
                |row| row.get::<_, f32>(0),
            )
            .optional()
            .map(|progress| progress.unwrap_or(0.).clamp(0., 1.))
            .map_err(Into::into)
    }

    pub fn save_reading_progress(&self, article_id: i64, progress: f32) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO article_reading_positions(workspace,article_id,progress,updated_at)
             VALUES(?1,?2,?3,CURRENT_TIMESTAMP)
             ON CONFLICT(workspace,article_id) DO UPDATE SET
               progress=excluded.progress, updated_at=CURRENT_TIMESTAMP",
            params![self.workspace, article_id, progress.clamp(0., 1.)],
        )?;
        Ok(())
    }

    pub fn processed_content(
        &self,
        article_id: i64,
        source: &str,
        url: &str,
        title: &str,
        pipeline: &str,
    ) -> anyhow::Result<Option<String>> {
        let source_hash = processed_source_hash(source, url, title);
        self.connection
            .query_row(
                "SELECT processed_html FROM articles WHERE id=?1 AND processed_source_hash=?2 AND processed_pipeline_hash=?3",
                params![article_id, source_hash, pipeline],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Stable identity for a canonical body, derived from its source and the
    /// pipeline that produced it rather than re-hashing HTML on every read.
    pub fn canonical_revision(
        source: &str,
        url: &str,
        title: &str,
        pipeline: &str,
    ) -> panda_core::ContentRevision {
        let source_hash = processed_source_hash(source, url, title);
        let mut hasher = Sha256::new();
        hasher.update(b"panda-canonical-v1\0");
        hasher.update(source_hash.as_bytes());
        hasher.update([0]);
        hasher.update(pipeline.as_bytes());
        panda_core::ContentRevision(std::sync::Arc::from(hex::encode(hasher.finalize())))
    }

    pub fn save_processed_content(
        &self,
        article_id: i64,
        source: &str,
        url: &str,
        title: &str,
        pipeline: &str,
        processed: &str,
    ) -> anyhow::Result<()> {
        let source_hash = processed_source_hash(source, url, title);
        let changed = self.connection.execute(
            "UPDATE articles SET processed_html=?1,processed_source_hash=?2,processed_pipeline_hash=?3,
                translated_html=NULL,translated_title=NULL,translated_lang=NULL,translation_source_hash=NULL,
                content_revision=content_revision+1 WHERE id=?4 AND
                (processed_html IS NOT ?1 OR processed_source_hash IS NOT ?2 OR processed_pipeline_hash IS NOT ?3)",
            params![processed, source_hash, pipeline, article_id],
        )?;
        if changed > 0 {
            sync_article_fts(&self.connection, article_id)?;
        }
        Ok(())
    }
}

fn prepare_feed(
    url: &str,
    feed: ParsedFeed,
    known_revisions: &HashMap<String, String>,
) -> anyhow::Result<PreparedFeed> {
    let site_url = feed
        .links
        .iter()
        .find(|link| link.rel.as_deref() == Some("alternate"))
        .or_else(|| feed.links.first())
        .map(|link| resolve_url(url, &link.href));
    let title = feed
        .title
        .as_ref()
        .map(|title| title.content.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(url)
        .to_owned();
    let mut articles = Vec::new();
    for entry in feed.entries {
        let Some((guid, source_revision)) = entry_source_revision(&entry, url, site_url.as_deref())
        else {
            continue;
        };
        if known_revisions.get(&guid) == Some(&source_revision) {
            continue;
        }
        if let Some(article) = map_entry(entry, url, site_url.as_deref()) {
            let compressed_source = compress_html(&article.source_html)?;
            articles.push((article, compressed_source, source_revision));
        }
    }
    Ok(PreparedFeed {
        title,
        site_url,
        articles,
    })
}

fn entry_source_revision(
    entry: &Entry,
    feed_url: &str,
    site_url: Option<&str>,
) -> Option<(String, String)> {
    let title = entry
        .title
        .as_ref()
        .map(|title| title.content.trim())
        .filter(|title| !title.is_empty())?;
    let published_at = entry
        .published
        .as_ref()
        .or(entry.updated.as_ref())
        .map(|date| date.to_rfc3339());
    let guid = if entry.id.trim().is_empty() {
        format!("{}:{}", title, published_at.as_deref().unwrap_or(""))
    } else {
        entry.id.clone()
    };
    let base = site_url.unwrap_or(feed_url);
    let url = entry
        .links
        .iter()
        .find(|link| link.rel.as_deref() == Some("alternate"))
        .or_else(|| entry.links.first())
        .map(|link| resolve_url(base, &link.href))
        .unwrap_or_default();
    let raw_html = entry
        .content
        .as_ref()
        .and_then(|content| content.body.as_deref())
        .or_else(|| {
            entry
                .summary
                .as_ref()
                .map(|summary| summary.content.as_str())
        })
        .unwrap_or_default();
    let author = entry
        .authors
        .first()
        .map(|person| person.name.as_str())
        .unwrap_or_default();
    let summary = entry
        .summary
        .as_ref()
        .map(|summary| summary.content.as_str())
        .unwrap_or_default();
    let mut hasher = Sha256::new();
    for value in [
        title,
        url.as_str(),
        author,
        raw_html,
        summary,
        published_at.as_deref().unwrap_or(""),
    ] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    Some((guid, hex::encode(hasher.finalize())))
}

fn compress_html(html: &str) -> anyhow::Result<Vec<u8>> {
    if html.len() > 8 * 1024 * 1024 {
        anyhow::bail!("article source exceeds the 8 MiB limit");
    }
    use std::io::Write as _;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(html.as_bytes())?;
    Ok(encoder.finish()?)
}

fn processed_source_hash(source: &str, url: &str, title: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    hasher.update([0]);
    hasher.update(url.as_bytes());
    hasher.update([0]);
    hasher.update(title.as_bytes());
    hex::encode(hasher.finalize())
}

fn decompress_html(compressed: &[u8]) -> anyhow::Result<String> {
    use std::io::Read as _;
    let mut decoder = ZlibDecoder::new(compressed);
    let mut bytes = Vec::new();
    decoder
        .by_ref()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 8 * 1024 * 1024 {
        anyhow::bail!("stored article source exceeds the 8 MiB limit");
    }
    String::from_utf8(bytes).context("stored article source is not UTF-8")
}

fn map_entry(entry: Entry, feed_url: &str, site_url: Option<&str>) -> Option<ParsedArticle> {
    let title = entry
        .title
        .map(|t| t.content)
        .filter(|s| !s.trim().is_empty())?;
    let base = site_url.unwrap_or(feed_url);
    let href = entry
        .links
        .iter()
        .find(|link| link.rel.as_deref() == Some("alternate"))
        .or_else(|| entry.links.first())
        .map(|link| resolve_url(base, &link.href));
    let raw_html = entry
        .content
        .as_ref()
        .and_then(|content| content.body.clone())
        .or_else(|| {
            entry
                .summary
                .as_ref()
                .map(|summary| summary.content.clone())
        })
        .unwrap_or_default();
    let source_html = raw_html.clone();
    let safe_html = sanitize_html(&raw_html, Some(base));
    let snippet = entry
        .summary
        .map(|summary| plain_text(&summary.content))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| plain_text(&raw_html));
    let published_at = entry
        .published
        .or(entry.updated)
        .map(|date| date.to_rfc3339());
    let guid = if entry.id.trim().is_empty() {
        format!("{}:{}", title, published_at.as_deref().unwrap_or(""))
    } else {
        entry.id
    };
    Some(ParsedArticle {
        guid,
        title,
        url: href,
        author: entry.authors.first().map(|person| person.name.clone()),
        published_at,
        snippet,
        content_html: safe_html,
        source_html,
    })
}

pub fn normalize_http_url(raw: &str) -> anyhow::Result<String> {
    let url = Url::parse(raw.trim())?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("Feed URL must use HTTP or HTTPS");
    }
    Ok(url.to_string())
}

fn resolve_url(base: &str, value: &str) -> String {
    Url::parse(value)
        .map(|url| url.to_string())
        .or_else(|_| Url::parse(base)?.join(value).map(|url| url.to_string()))
        .unwrap_or_else(|_| value.to_owned())
}

fn collect_outlines(
    outline: &Outline,
    folder: Option<&str>,
    result: &mut Vec<(String, Option<String>, Option<String>)>,
) {
    if let Some(url) = &outline.xml_url {
        let title = if outline.text.trim().is_empty() {
            outline.title.clone()
        } else {
            Some(outline.text.clone())
        };
        result.push((url.clone(), title, folder.map(str::to_owned)));
    }
    let current_folder = if outline.xml_url.is_none() && !outline.outlines.is_empty() {
        if outline.text.trim().is_empty() {
            outline.title.as_deref().or(folder)
        } else {
            Some(outline.text.as_str())
        }
    } else {
        folder
    };
    for child in &outline.outlines {
        collect_outlines(child, current_folder, result);
    }
}

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;
