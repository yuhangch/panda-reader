use anyhow::Context as _;
use feed_rs::model::{Entry, Feed as ParsedFeed};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use opml::{Head, OPML, Outline};
use panda_content::{extract_article_html, plain_text, sanitize_html};
use panda_core::{
    Article, ArticleCursor, ArticleSummary, ContentExtractor, Feed, MarkField, ParsedArticle,
    ReaderSnapshot, Scope,
};
#[cfg(test)]
use panda_providers::{ProviderClient, SyncMode};
use panda_providers::{ProviderKind, RemoteEntry, RemoteFeed, SyncCursor};
use reqwest::Client;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

const MAX_FEED_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const MAX_ARTICLE_PAGE_BYTES: usize = 20 * 1024 * 1024;
use url::Url;

pub struct Store {
    connection: Connection,
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
    pub article_id: i64,
    pub resolved_url: String,
    pub source_page_html: Vec<u8>,
    pub extracted_html: String,
    pub pipeline_hash: String,
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
    pub fn migrate(path: &Path) -> anyhow::Result<()> {
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
        let has_existing_schema: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%')",
            [],
            |row| row.get(0),
        )?;
        let has_legacy_feeds_table: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='feeds')",
            [],
            |row| row.get(0),
        )?;
        let feed_columns = if has_legacy_feeds_table {
            let mut statement = connection.prepare("PRAGMA table_info(feeds)")?;
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let needs_foreign_key_toggle =
            has_legacy_feeds_table && !feed_columns.iter().any(|column| column == "workspace");
        if schema_version < 5 && has_existing_schema {
            connection.query_row("PRAGMA wal_checkpoint(FULL)", [], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            let timestamp = chrono::Utc::now().timestamp_millis();
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            let backup_path =
                path.with_file_name(format!("{file_name}.pre-migration-{timestamp}.bak"));
            std::fs::copy(path, &backup_path)?;
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&backup_path)?
                .sync_all()?;
        }
        if needs_foreign_key_toggle {
            connection.pragma_update(None, "foreign_keys", "OFF")?;
        }
        // SQLite DDL and user_version changes are transactional. If any step below
        // fails, dropping the connection rolls back the entire migration.
        connection.execute_batch("BEGIN IMMEDIATE")?;
        if schema_version < 4 {
            connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS feeds (
                id INTEGER PRIMARY KEY,
                feed_url TEXT NOT NULL,
                title TEXT NOT NULL,
                custom_title TEXT,
                site_url TEXT,
                folder TEXT,
                etag TEXT,
                last_modified TEXT,
                last_error TEXT,
                added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                auto_translate_titles_after_article_id INTEGER,
                workspace TEXT NOT NULL DEFAULT 'local',
                UNIQUE(workspace,feed_url)
            );
            CREATE TABLE IF NOT EXISTS articles (
                id INTEGER PRIMARY KEY,
                feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
                guid TEXT NOT NULL,
                title TEXT NOT NULL,
                url TEXT,
                author TEXT,
                published_at TEXT,
                snippet TEXT NOT NULL DEFAULT '',
                content_html TEXT NOT NULL DEFAULT '',
                source_html BLOB,
                source_page_html BLOB,
                extraction_pipeline_hash TEXT,
                extracted_html TEXT,
                processed_html TEXT,
                processed_source_hash TEXT,
                processed_pipeline_hash TEXT,
                is_read INTEGER NOT NULL DEFAULT 0,
                is_starred INTEGER NOT NULL DEFAULT 0,
                read_later INTEGER NOT NULL DEFAULT 0,
                UNIQUE(feed_id, guid)
            );
            CREATE INDEX IF NOT EXISTS idx_articles_feed_date ON articles(feed_id, published_at DESC);
            DROP INDEX IF EXISTS idx_articles_state_date;
            DROP INDEX IF EXISTS idx_articles_feed_date;
            CREATE INDEX IF NOT EXISTS idx_articles_sort ON articles(COALESCE(published_at,''),id);
            CREATE INDEX IF NOT EXISTS idx_articles_unread_sort ON articles(COALESCE(published_at,''),id) WHERE is_read=0;
            CREATE INDEX IF NOT EXISTS idx_articles_starred_sort ON articles(COALESCE(published_at,''),id) WHERE is_starred=1;
            CREATE INDEX IF NOT EXISTS idx_articles_later_sort ON articles(COALESCE(published_at,''),id) WHERE read_later=1;
            CREATE INDEX IF NOT EXISTS idx_articles_feed_sort ON articles(feed_id,COALESCE(published_at,''),id);
            CREATE INDEX IF NOT EXISTS idx_articles_feed_unread ON articles(feed_id) WHERE is_read=0;
            CREATE TABLE IF NOT EXISTS remote_state (
                workspace TEXT PRIMARY KEY,
                provider TEXT NOT NULL DEFAULT '',
                account TEXT NOT NULL,
                sync_cursor TEXT,
                last_sync_at TEXT,
                last_full_sync_at TEXT
            );
            CREATE TABLE IF NOT EXISTS pending_remote_marks (
                workspace TEXT NOT NULL,
                remote_id INTEGER NOT NULL,
                field TEXT NOT NULL,
                value INTEGER NOT NULL,
                PRIMARY KEY(workspace,remote_id,field)
            );
            CREATE TABLE IF NOT EXISTS article_reading_positions (
                workspace TEXT NOT NULL,
                article_id INTEGER NOT NULL,
                progress REAL NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY(workspace,article_id)
            );
            PRAGMA foreign_keys = ON;",
        )?;
            connection.execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS article_fts USING fts5(
                title, author, content_text, translated_text, tokenize='unicode61'
             );
             CREATE TABLE IF NOT EXISTS article_fts_state (
                id INTEGER PRIMARY KEY CHECK(id=1), last_article_id INTEGER NOT NULL DEFAULT 0,
                complete INTEGER NOT NULL DEFAULT 0
             );
             CREATE TRIGGER IF NOT EXISTS articles_fts_delete AFTER DELETE ON articles BEGIN
                DELETE FROM article_fts WHERE rowid=old.id;
             END;",
            )?;
            // A missing state row means the index may be newly created or left
            // incomplete by a prior crash, so conservatively schedule a rebuild.
            connection.execute(
            "INSERT OR IGNORE INTO article_fts_state(id,last_article_id,complete) VALUES(1,0,0)",
            [],
        )?;
            ensure_column(
                &connection,
                "feeds",
                "source",
                "TEXT NOT NULL DEFAULT 'local'",
            )?;
            ensure_column(&connection, "feeds", "remote_id", "INTEGER")?;
            ensure_column(&connection, "feeds", "custom_title", "TEXT")?;
            ensure_column(&connection, "feeds", "language", "TEXT")?;
            ensure_column(
                &connection,
                "feeds",
                "auto_translate_titles",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
            ensure_column(
                &connection,
                "feeds",
                "auto_translate_titles_after_article_id",
                "INTEGER",
            )?;
            connection.execute(
                "UPDATE feeds SET auto_translate_titles_after_article_id=COALESCE((SELECT MAX(id) FROM articles WHERE articles.feed_id=feeds.id),0) WHERE auto_translate_titles=1 AND auto_translate_titles_after_article_id IS NULL",
                [],
            )?;
            ensure_column(&connection, "articles", "remote_id", "INTEGER")?;
            ensure_column(&connection, "articles", "remote_content_hash", "TEXT")?;
            ensure_column(&connection, "articles", "source_revision", "TEXT")?;
            ensure_column(
                &connection,
                "articles",
                "content_revision",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
            ensure_column(&connection, "articles", "source_html", "BLOB")?;
            ensure_column(&connection, "articles", "source_page_html", "BLOB")?;
            ensure_column(&connection, "articles", "extraction_pipeline_hash", "TEXT")?;
            ensure_column(&connection, "articles", "processed_html", "TEXT")?;
            ensure_column(&connection, "articles", "processed_source_hash", "TEXT")?;
            ensure_column(&connection, "articles", "processed_pipeline_hash", "TEXT")?;
            ensure_column(&connection, "articles", "translated_html", "TEXT")?;
            ensure_column(&connection, "articles", "translated_title", "TEXT")?;
            ensure_column(&connection, "articles", "translated_lang", "TEXT")?;
            ensure_column(&connection, "articles", "translation_source_hash", "TEXT")?;
            ensure_column(&connection, "articles", "auto_translated_title", "TEXT")?;
            ensure_column(
                &connection,
                "articles",
                "auto_translated_title_lang",
                "TEXT",
            )?;
            ensure_column(
                &connection,
                "articles",
                "auto_translated_title_source_hash",
                "TEXT",
            )?;
            connection.execute_batch("CREATE TABLE IF NOT EXISTS translation_usage(day TEXT NOT NULL, provider TEXT NOT NULL, requests INTEGER NOT NULL DEFAULT 0, characters INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(day,provider));")?;
            ensure_column(
                &connection,
                "translation_usage",
                "input_tokens",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
            ensure_column(
                &connection,
                "translation_usage",
                "output_tokens",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
            connection.execute_batch("CREATE TABLE IF NOT EXISTS article_translation_segments(workspace TEXT NOT NULL, article_id INTEGER NOT NULL, target_lang TEXT NOT NULL, backend_id TEXT NOT NULL, prompt_revision TEXT NOT NULL, context_hash TEXT NOT NULL DEFAULT '', segment_id TEXT NOT NULL, source_hash TEXT NOT NULL, translated_html TEXT NOT NULL, PRIMARY KEY(workspace,article_id,target_lang,backend_id,prompt_revision,segment_id,source_hash));")?;
            ensure_column(
                &connection,
                "article_translation_segments",
                "context_hash",
                "TEXT NOT NULL DEFAULT ''",
            )?;
            migrate_workspace_schema(&connection)?;
            ensure_column(&connection, "feeds", "custom_title", "TEXT")?;
            ensure_column(&connection, "feeds", "language", "TEXT")?;
            ensure_column(
                &connection,
                "feeds",
                "auto_translate_titles",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
            connection.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_feeds_workspace_folder ON feeds(workspace,folder);",
            )?;
            ensure_column(
                &connection,
                "remote_state",
                "provider",
                "TEXT NOT NULL DEFAULT ''",
            )?;
            ensure_column(&connection, "remote_state", "sync_cursor", "TEXT")?;
            ensure_column(&connection, "remote_state", "last_sync_at", "TEXT")?;
            ensure_column(&connection, "remote_state", "last_full_sync_at", "TEXT")?;
            ensure_column(&connection, "feeds", "custom_title", "TEXT")?;
            ensure_column(
                &connection,
                "pending_remote_marks",
                "revision",
                "INTEGER NOT NULL DEFAULT 0",
            )?;
            connection.execute_batch(
            "DROP INDEX IF EXISTS idx_feeds_remote;
             DROP INDEX IF EXISTS idx_articles_remote;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_feeds_remote ON feeds(workspace,remote_id) WHERE remote_id IS NOT NULL;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_articles_remote ON articles(feed_id,remote_id) WHERE remote_id IS NOT NULL;",
        )?;
            connection.pragma_update(None, "user_version", 4)?;
        }
        // Keep this idempotent for databases whose schema version predates the
        // title-translation watermark. Existing enabled feeds start after their
        // current newest article so their history is never queued as new.
        ensure_column(
            &connection,
            "feeds",
            "auto_translate_titles_after_article_id",
            "INTEGER",
        )?;
        connection.execute(
            "UPDATE feeds SET auto_translate_titles_after_article_id=COALESCE((SELECT MAX(id) FROM articles WHERE articles.feed_id=feeds.id),0) WHERE auto_translate_titles=1 AND auto_translate_titles_after_article_id IS NULL",
            [],
        )?;
        let fts_exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='article_fts')",
            [],
            |row| row.get(0),
        )?;
        let fts_state_exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='article_fts_state')",
            [],
            |row| row.get(0),
        )?;
        let fts_trigger_exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='articles_fts_delete')",
            [],
            |row| row.get(0),
        )?;
        if !fts_exists || !fts_state_exists || !fts_trigger_exists {
            connection.execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS article_fts USING fts5(
                    title, author, content_text, translated_text, tokenize='unicode61'
                 );
                 CREATE TABLE IF NOT EXISTS article_fts_state (
                    id INTEGER PRIMARY KEY CHECK(id=1), last_article_id INTEGER NOT NULL DEFAULT 0,
                    complete INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TRIGGER IF NOT EXISTS articles_fts_delete AFTER DELETE ON articles BEGIN
                    DELETE FROM article_fts WHERE rowid=old.id;
                 END;
                 INSERT OR IGNORE INTO article_fts_state(id,last_article_id,complete) VALUES(1,0,0);",
            )?;
            if !fts_exists {
                connection.execute(
                    "UPDATE article_fts_state SET last_article_id=0,complete=0 WHERE id=1",
                    [],
                )?;
            }
        }
        if schema_version < 5 {
            // The canonical-body contract changes the FTS source from extracted
            // HTML to processed canonical HTML. Rebuild incrementally so old
            // rows converge without blocking startup.
            connection.execute(
                "UPDATE article_fts_state SET last_article_id=0,complete=0 WHERE id=1",
                [],
            )?;
            connection.pragma_update(None, "user_version", 5)?;
        }
        if schema_version < 5 {
            let integrity: String =
                connection.query_row("PRAGMA integrity_check(1)", [], |row| row.get(0))?;
            if integrity != "ok" {
                anyhow::bail!("database integrity check failed after migration: {integrity}");
            }
            let foreign_key_errors: usize = connection.query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_check",
                [],
                |row| row.get(0),
            )?;
            if foreign_key_errors != 0 {
                anyhow::bail!(
                    "database migration left {foreign_key_errors} foreign-key violations"
                );
            }
        }
        connection.execute_batch("COMMIT")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    }

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
        if schema_version < 5 {
            anyhow::bail!("database schema migration is required before opening the writer");
        }
        let client = shared_http_client();
        Ok(Self {
            connection,
            client,
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

    pub fn has_feed_url(&self, url: &str) -> anyhow::Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM feeds WHERE feed_url=?1 AND workspace=?2)",
            params![url, self.workspace],
            |row| row.get(0),
        )?)
    }

    pub async fn fetch_feed_data(
        url: &str,
        etag: Option<&str>,
        modified: Option<&str>,
    ) -> anyhow::Result<FetchedFeed> {
        let mut request = Client::builder()
            .user_agent("PandaReader/0.1 (+https://github.com)")
            .timeout(Duration::from_secs(25))
            .build()?
            .get(url);
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(modified) = modified {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, modified);
        }
        let response = request.send().await?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(FetchedFeed {
                parsed: None,
                etag: etag.map(str::to_owned),
                modified: modified.map(str::to_owned),
            });
        }
        let response = response.error_for_status()?;
        let next_etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let next_modified = response
            .headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = read_response_limited(response, MAX_FEED_RESPONSE_BYTES, "RSS feed").await?;
        Ok(FetchedFeed {
            parsed: Some(feed_rs::parser::parse(bytes.as_slice())?),
            etag: next_etag,
            modified: next_modified,
        })
    }

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
    ) -> anyhow::Result<bool> {
        if let Some(feed) = response.parsed {
            self.write_prepared_feed(url, feed, id, response.etag, response.modified)?;
            Ok(true)
        } else if let Some(id) = id {
            self.connection.execute(
                "UPDATE feeds SET last_error=NULL WHERE id=?1 AND workspace=?2",
                params![id, self.workspace],
            )?;
            Ok(false)
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
        self.write_prepared_feed(url, feed, None, response.etag, response.modified)?;
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

    #[cfg(test)]
    pub async fn translate(
        &mut self,
        article_id: i64,
        target_lang: &str,
        translator: &panda_translate::AnyTranslator,
    ) -> anyhow::Result<Article> {
        let mut article = self.article(article_id)?;
        let source = article
            .extracted_html
            .as_deref()
            .filter(|html| !html.trim().is_empty())
            .or(article.source_html.as_deref())
            .unwrap_or(article.content_html.as_str())
            .to_owned();
        let url = article.url.as_deref().unwrap_or_default();
        let pipeline = "store-test-canonical-v1";
        article.canonical = Some(panda_core::CanonicalArticle {
            html: panda_core::CanonicalHtml(std::sync::Arc::from(source.as_str())),
            revision: Self::canonical_revision(&source, url, &article.summary.title, pipeline),
            pipeline_revision: std::sync::Arc::from(pipeline),
        });
        self.translate_with_source(article_id, target_lang, &source, translator)
            .await
    }

    #[cfg(test)]
    pub async fn translate_with_source(
        &mut self,
        article_id: i64,
        target_lang: &str,
        source: &str,
        translator: &panda_translate::AnyTranslator,
    ) -> anyhow::Result<Article> {
        let article = self.article(article_id)?;
        let title = article.summary.title.trim();
        let source_hash = panda_translate::translation_cache_hash(source, title, translator.id());
        let has_body = article
            .translated_html
            .as_deref()
            .is_some_and(|html| !html.trim().is_empty());
        let lang_ok = article
            .translated_lang
            .as_deref()
            .is_some_and(|lang| lang == target_lang);
        let hash_ok = article
            .translation_source_hash
            .as_deref()
            .is_some_and(|hash| hash == source_hash);
        if lang_ok && has_body && hash_ok {
            if let Some(translated_title) = article
                .translated_title
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                && article.summary.feed_auto_translate_titles
            {
                self.save_auto_translated_title(
                    article_id,
                    &article.summary.title,
                    translated_title,
                    target_lang,
                    &panda_translate::title_source_hash(&article.summary.title),
                )?;
                return self.article(article_id);
            }
            return Ok(article);
        }
        if source.trim().is_empty() && title.is_empty() {
            anyhow::bail!("Nothing to translate");
        }
        let result = translator
            .translate(panda_translate::TranslateRequest {
                html: source.to_owned(),
                title: (!title.is_empty()).then(|| title.to_owned()),
                target_lang: target_lang.to_owned(),
            })
            .await?;
        self.connection.execute(
            "UPDATE articles SET translated_html=?1, translated_title=?2, translated_lang=?3, translation_source_hash=?4,
                content_revision=content_revision+1 WHERE id=?5",
            params![result.html, result.title, target_lang, source_hash, article_id],
        )?;
        sync_article_fts(&self.connection, article_id)?;
        if let Some(translated_title) = result
            .title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            && article.summary.feed_auto_translate_titles
        {
            self.save_auto_translated_title(
                article_id,
                &article.summary.title,
                translated_title,
                target_lang,
                &panda_translate::title_source_hash(&article.summary.title),
            )?;
        }
        self.article(article_id)
    }

    pub fn persist_translation(
        &self,
        article_id: i64,
        expected_content_revision: i64,
        target_lang: &str,
        source_hash: &str,
        translated_html: &str,
        translated_title: Option<&str>,
    ) -> anyhow::Result<()> {
        let updated = self.connection.execute(
            "UPDATE articles SET translated_html=?1, translated_title=?2, translated_lang=?3, translation_source_hash=?4,
                content_revision=content_revision+1 WHERE id=?5 AND content_revision=?6",
            params![translated_html, translated_title, target_lang, source_hash, article_id, expected_content_revision],
        )?;
        if updated == 0 {
            anyhow::bail!("Article content changed while translation was running");
        }
        sync_article_fts(&self.connection, article_id)?;
        Ok(())
    }

    pub fn translation_segments(
        &self,
        article_id: i64,
        target_lang: &str,
        backend_id: &str,
        prompt_revision: &str,
        context_hash: &str,
    ) -> anyhow::Result<HashMap<String, String>> {
        let mut statement = self.connection.prepare(
            "SELECT segment_id,translated_html FROM article_translation_segments
             WHERE workspace=?1 AND article_id=?2 AND target_lang=?3 AND backend_id=?4 AND prompt_revision=?5 AND context_hash=?6",
        )?;
        let rows = statement.query_map(
            params![
                self.workspace,
                article_id,
                target_lang,
                backend_id,
                prompt_revision,
                context_hash
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(Into::into)
    }

    pub fn save_translation_segment(
        &self,
        article_id: i64,
        target_lang: &str,
        backend_id: &str,
        prompt_revision: &str,
        context_hash: &str,
        segment_id: &str,
        source_hash: &str,
        translated_html: &str,
    ) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO article_translation_segments(workspace,article_id,target_lang,backend_id,prompt_revision,context_hash,segment_id,source_hash,translated_html)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(workspace,article_id,target_lang,backend_id,prompt_revision,segment_id,source_hash)
             DO UPDATE SET context_hash=excluded.context_hash,translated_html=excluded.translated_html",
            params![
                self.workspace,
                article_id,
                target_lang,
                backend_id,
                prompt_revision,
                context_hash,
                segment_id,
                source_hash,
                translated_html
            ],
        )?;
        Ok(())
    }

    pub fn prune_translation_segments(
        &self,
        article_id: i64,
        target_lang: &str,
        backend_id: &str,
        prompt_revision: &str,
        context_hash: &str,
    ) -> anyhow::Result<()> {
        self.connection.execute(
            "DELETE FROM article_translation_segments
             WHERE workspace=?1 AND article_id=?2
               AND NOT (target_lang=?3 AND backend_id=?4 AND prompt_revision=?5 AND context_hash=?6)",
            params![
                self.workspace,
                article_id,
                target_lang,
                backend_id,
                prompt_revision,
                context_hash
            ],
        )?;
        Ok(())
    }

    pub fn save_auto_translated_title(
        &self,
        id: i64,
        source_title: &str,
        title: &str,
        lang: &str,
        source_hash: &str,
    ) -> anyhow::Result<()> {
        self.connection.execute("UPDATE articles SET auto_translated_title=?1,auto_translated_title_lang=?2,auto_translated_title_source_hash=?3 WHERE id=?4 AND articles.title=?5 AND EXISTS(SELECT 1 FROM feeds WHERE feeds.id=articles.feed_id AND feeds.workspace=?6)",params![title,lang,source_hash,id,source_title,self.workspace])?;
        Ok(())
    }

    pub fn record_translation_usage(
        &self,
        day: &str,
        provider: &str,
        requests: u64,
        characters: u64,
    ) -> anyhow::Result<()> {
        self.record_translation_usage_with_tokens(day, provider, requests, characters, 0, 0)
    }

    pub fn record_translation_usage_with_tokens(
        &self,
        day: &str,
        provider: &str,
        requests: u64,
        characters: u64,
        input_tokens: u64,
        output_tokens: u64,
    ) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO translation_usage(day,provider,requests,characters,input_tokens,output_tokens) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(day,provider) DO UPDATE SET requests=requests+excluded.requests,characters=characters+excluded.characters,input_tokens=input_tokens+excluded.input_tokens,output_tokens=output_tokens+excluded.output_tokens",
            params![day,provider,requests,characters,input_tokens,output_tokens],
        )?;
        Ok(())
    }

    pub fn translation_usage(&self) -> anyhow::Result<Vec<panda_core::TranslationUsage>> {
        let mut stmt=self.connection.prepare("SELECT day,provider,requests,characters,input_tokens,output_tokens FROM translation_usage ORDER BY day DESC,provider")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(panda_core::TranslationUsage {
                    day: r.get(0)?,
                    provider: r.get(1)?,
                    requests: r.get::<_, u64>(2)?,
                    characters: r.get::<_, u64>(3)?,
                    input_tokens: r.get::<_, u64>(4)?,
                    output_tokens: r.get::<_, u64>(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn mark(&self, id: i64, field: MarkField, value: bool) -> anyhow::Result<()> {
        let column = match field {
            MarkField::Read => "is_read",
            MarkField::Starred => "is_starred",
            MarkField::Later => "read_later",
        };
        self.connection.execute(
            &format!("UPDATE articles SET {column}=?1 WHERE id=?2"),
            params![value, id],
        )?;
        if !matches!(field, MarkField::Later)
            && let Some(remote_id) = self.remote_entry_id(id)?
        {
            self.connection.execute(
                "INSERT INTO pending_remote_marks(workspace,remote_id,field,value,revision) VALUES(?1,?2,?3,?4,1)
                 ON CONFLICT(workspace,remote_id,field) DO UPDATE SET value=excluded.value,revision=pending_remote_marks.revision+1",
                params![self.workspace, remote_id, column, value],
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub async fn flush_provider_marks(&mut self, remote: &ProviderClient) -> anyhow::Result<usize> {
        let pending = {
            let mut statement = self.connection.prepare(
                "SELECT remote_id,field,value,revision FROM pending_remote_marks WHERE workspace=?1 ORDER BY remote_id,field",
            )?;
            statement
                .query_map([&self.workspace], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut read_ids = Vec::new();
        let mut unread_ids = Vec::new();
        let mut starred = Vec::new();
        for (remote_id, field, value, _) in &pending {
            match field.as_str() {
                "is_read" if *value => read_ids.push(*remote_id),
                "is_read" => unread_ids.push(*remote_id),
                "is_starred" => starred.push((*remote_id, *value)),
                _ => {}
            }
        }
        remote.mark_entries_status(&read_ids, true).await?;
        remote.mark_entries_status(&unread_ids, false).await?;
        for (remote_id, value) in starred {
            remote.set_starred(remote_id, value).await?;
        }
        let transaction = self.connection.transaction()?;
        let mut completed = 0;
        for (remote_id, field, _, revision) in pending {
            completed += transaction.execute(
                "DELETE FROM pending_remote_marks WHERE workspace=?1 AND remote_id=?2 AND field=?3 AND revision=?4",
                params![self.workspace, remote_id, field, revision],
            )?;
        }
        transaction.commit()?;
        Ok(completed)
    }

    pub fn mark_all_read(&self, scope: Scope) -> anyhow::Result<usize> {
        enum ScopeBind {
            None,
            FeedId(i64),
            Folder(String),
        }
        let (filter, bind) = match scope {
            Scope::All | Scope::Unread => ("1=1", ScopeBind::None),
            Scope::Starred => ("a.is_starred=1", ScopeBind::None),
            Scope::Later => ("a.read_later=1", ScopeBind::None),
            Scope::Feed(id) => ("a.feed_id=?", ScopeBind::FeedId(id)),
            Scope::Folder(name) => ("f.folder=?", ScopeBind::Folder(name)),
        };
        let sql = format!(
            "SELECT a.id, a.remote_id FROM articles a JOIN feeds f ON f.id=a.feed_id
             WHERE f.workspace=? AND a.is_read=0 AND {filter}"
        );
        // Keep the article updates and their pending provider marks in one transaction.
        // A folder can contain thousands of rows, and autocommitting each row is expensive.
        let transaction = self.connection.unchecked_transaction()?;
        let rows: Vec<(i64, Option<i64>)> = {
            let mut statement = transaction.prepare(&sql)?;
            match bind {
                ScopeBind::FeedId(feed_id) => statement
                    .query_map(params![self.workspace, feed_id], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?,
                ScopeBind::Folder(folder) => statement
                    .query_map(params![self.workspace, folder], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?,
                ScopeBind::None => statement
                    .query_map([&self.workspace], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?,
            }
        };
        let count = rows.len();
        for (id, remote_id) in rows {
            transaction.execute("UPDATE articles SET is_read=1 WHERE id=?1", [id])?;
            if let Some(remote_id) = remote_id {
                transaction.execute(
                    "INSERT INTO pending_remote_marks(workspace,remote_id,field,value,revision) VALUES(?1,?2,'is_read',1,1)
                     ON CONFLICT(workspace,remote_id,field) DO UPDATE SET value=excluded.value,revision=pending_remote_marks.revision+1",
                    params![self.workspace, remote_id],
                )?;
            }
        }
        transaction.commit()?;
        Ok(count)
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

    pub fn remote_entry_id(&self, id: i64) -> anyhow::Result<Option<i64>> {
        Ok(self.connection.query_row(
            "SELECT a.remote_id FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.id=?1 AND f.workspace=?2",
            params![id, self.workspace],
            |row| row.get(0),
        )?)
    }

    pub fn remote_feed_id(&self, id: i64) -> anyhow::Result<Option<i64>> {
        Ok(self.connection.query_row(
            "SELECT remote_id FROM feeds WHERE id=?1 AND workspace=?2",
            params![id, self.workspace],
            |row| row.get(0),
        )?)
    }

    /// Feed fields needed to remove it from the active provider as well.
    pub fn feed_removal_info(&self, id: i64) -> anyhow::Result<(Option<i64>, String, String)> {
        Ok(self.connection.query_row(
            "SELECT remote_id, feed_url, COALESCE(source, 'local') FROM feeds WHERE id=?1 AND workspace=?2",
            params![id, self.workspace],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?)
    }

    pub fn pending_remote_marks(&self) -> anyhow::Result<Vec<PendingRemoteMark>> {
        let mut statement = self.connection.prepare(
            "SELECT remote_id,field,value,revision FROM pending_remote_marks WHERE workspace=?1 ORDER BY remote_id,field",
        )?;
        Ok(statement
            .query_map([&self.workspace], |row| {
                Ok(PendingRemoteMark {
                    remote_id: row.get(0)?,
                    field: row.get(1)?,
                    value: row.get(2)?,
                    revision: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn provider_sync_state(&self) -> anyhow::Result<Option<ProviderSyncState>> {
        let state = self.connection.query_row(
            "SELECT provider,account,sync_cursor,last_full_sync_at FROM remote_state WHERE workspace=?1",
            [&self.workspace],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?)),
        ).optional()?;
        Ok(state.map(
            |(provider, account, cursor, last_full_sync_at)| ProviderSyncState {
                provider,
                account,
                cursor: cursor.map(|value| SyncCursor { value }),
                last_full_sync_at,
            },
        ))
    }

    pub fn begin_provider_sync(
        &self,
        kind: ProviderKind,
        account: &str,
        cursor: Option<&SyncCursor>,
        last_full_sync_at: Option<&str>,
        reset: bool,
    ) -> anyhow::Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        if reset {
            transaction.execute(
                "DELETE FROM feeds WHERE workspace=?1 AND source IS NOT NULL AND source<>'local'",
                [&self.workspace],
            )?;
            transaction.execute(
                "DELETE FROM pending_remote_marks WHERE workspace=?1",
                [&self.workspace],
            )?;
        }
        transaction.execute(
            "INSERT INTO remote_state(workspace,provider,account,sync_cursor,last_sync_at,last_full_sync_at)
             VALUES(?1,?2,?3,?4,NULL,?5)
             ON CONFLICT(workspace) DO UPDATE SET provider=excluded.provider,account=excluded.account,
                sync_cursor=excluded.sync_cursor,last_full_sync_at=excluded.last_full_sync_at",
            params![self.workspace, kind.key(), account, cursor.map(|value| value.value.as_str()), last_full_sync_at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Replace an active provider workspace with a fully synchronized staging workspace.
    /// Keeping the old workspace visible until this transaction commits prevents an account
    /// switch from discarding its cached subscriptions when network synchronization fails.
    pub fn promote_provider_workspace(
        &mut self,
        provider: ProviderKind,
        staging_workspace: &str,
    ) -> anyhow::Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM feeds WHERE workspace=?1 AND source IS NOT NULL AND source<>'local'",
            [&self.workspace],
        )?;
        transaction.execute(
            "DELETE FROM pending_remote_marks WHERE workspace=?1",
            [&self.workspace],
        )?;
        transaction.execute(
            "DELETE FROM remote_state WHERE workspace=?1",
            [&self.workspace],
        )?;
        transaction.execute(
            "UPDATE feeds SET workspace=?1 WHERE workspace=?2 AND source=?3",
            params![self.workspace, staging_workspace, provider.key()],
        )?;
        transaction.execute(
            "UPDATE pending_remote_marks SET workspace=?1 WHERE workspace=?2",
            params![self.workspace, staging_workspace],
        )?;
        transaction.execute(
            "UPDATE remote_state SET workspace=?1 WHERE workspace=?2",
            params![self.workspace, staging_workspace],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn update_provider_sync_cursor(
        &self,
        cursor: Option<&SyncCursor>,
        full_sync_at: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.connection.execute(
            "UPDATE remote_state SET sync_cursor=?1,last_sync_at=?2,last_full_sync_at=COALESCE(?3,last_full_sync_at) WHERE workspace=?4",
            params![cursor.map(|value| value.value.as_str()), now, full_sync_at, self.workspace],
        )?;
        Ok(())
    }

    pub fn acknowledge_remote_marks(
        &mut self,
        marks: &[PendingRemoteMark],
    ) -> anyhow::Result<usize> {
        let transaction = self.connection.transaction()?;
        let mut completed = 0;
        for mark in marks {
            completed += transaction.execute(
                "DELETE FROM pending_remote_marks WHERE workspace=?1 AND remote_id=?2 AND field=?3 AND revision=?4",
                params![self.workspace, mark.remote_id, mark.field, mark.revision],
            )?;
        }
        transaction.commit()?;
        Ok(completed)
    }

    #[cfg(test)]
    pub async fn sync_provider(
        &mut self,
        remote: &ProviderClient,
        kind: ProviderKind,
    ) -> anyhow::Result<usize> {
        let identity = remote.identity().await?;
        let feeds = remote.feeds().await?;
        let account = identity.account;
        let previous: Option<(String, String, Option<String>, Option<String>)> = self
            .connection
            .query_row(
                "SELECT provider,account,sync_cursor,last_full_sync_at FROM remote_state WHERE workspace=?1",
                [&self.workspace],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let account_changed = previous
            .as_ref()
            .is_some_and(|(_, old_account, _, _)| old_account != &account);
        let provider_changed = previous.as_ref().is_some_and(|(old_provider, _, _, _)| {
            !old_provider.is_empty() && old_provider != kind.key()
        });
        let reset = account_changed || provider_changed;
        if reset {
            self.connection.execute(
                "DELETE FROM feeds WHERE source=?1 AND workspace=?2",
                params![kind.key(), self.workspace],
            )?;
            self.connection.execute(
                "DELETE FROM pending_remote_marks WHERE workspace=?1",
                [&self.workspace],
            )?;
        }
        let mut cursor = if reset {
            None
        } else {
            previous
                .as_ref()
                .and_then(|(_, _, cursor, _)| cursor.as_ref())
                .map(|value| SyncCursor {
                    value: value.clone(),
                })
        };
        let last_full_sync_at = if reset {
            None
        } else {
            previous
                .as_ref()
                .and_then(|(_, _, _, timestamp)| timestamp.as_deref())
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&chrono::Utc))
        };
        let mode = if reset || previous.is_none() || cursor.is_none() {
            SyncMode::Initial
        } else if !remote.capabilities().incremental_sync {
            SyncMode::Reconcile
        } else if last_full_sync_at
            .is_none_or(|last| chrono::Utc::now().signed_duration_since(last).num_days() >= 7)
        {
            SyncMode::Reconcile
        } else {
            SyncMode::Incremental
        };
        self.connection.execute(
            "INSERT INTO remote_state(workspace,provider,account,sync_cursor,last_sync_at,last_full_sync_at)
             VALUES(?1,?2,?3,?4,NULL,?5)
             ON CONFLICT(workspace) DO UPDATE SET provider=excluded.provider,account=excluded.account,
                sync_cursor=excluded.sync_cursor,last_full_sync_at=excluded.last_full_sync_at",
            params![
                self.workspace,
                kind.key(),
                account,
                cursor.as_ref().map(|value| value.value.as_str()),
                last_full_sync_at.map(|value| value.to_rfc3339())
            ],
        )?;
        self.flush_provider_marks(remote).await?;
        let feed_ids = self.save_remote_feeds(&feeds, kind)?;
        let mut saved = 0;
        let mut full_sync = matches!(mode, SyncMode::Initial | SyncMode::Reconcile);
        loop {
            let page = remote.entries_page(mode, cursor.as_ref()).await?;
            full_sync |= page.full_sync;
            saved += self.save_remote_entries(&page.entries, &feed_ids, kind)?;
            cursor = page.next_cursor;
            let now = chrono::Utc::now().to_rfc3339();
            let full_sync_at = (!page.has_more && full_sync).then_some(now.as_str());
            self.connection.execute(
                "UPDATE remote_state SET sync_cursor=?1,last_sync_at=?2,
                    last_full_sync_at=COALESCE(?3,last_full_sync_at) WHERE workspace=?4",
                params![
                    cursor.as_ref().map(|value| value.value.as_str()),
                    now,
                    full_sync_at,
                    self.workspace
                ],
            )?;
            if !page.has_more {
                break;
            }
            if cursor.is_none() {
                anyhow::bail!("Provider returned another sync page without a continuation cursor");
            }
        }
        Ok(saved)
    }

    pub fn save_remote_feeds(
        &mut self,
        feeds: &[RemoteFeed],
        kind: ProviderKind,
    ) -> anyhow::Result<HashMap<i64, i64>> {
        let mut seen = HashSet::new();
        for feed in feeds {
            seen.insert(feed.id);
            let language = if kind == ProviderKind::Miniflux {
                feed.language.as_deref()
            } else {
                None
            };
            let folder = feed
                .category
                .as_ref()
                .map(|category| category.title.as_str());
            self.connection.execute(
                "UPDATE articles SET auto_translated_title=NULL,auto_translated_title_lang=NULL,auto_translated_title_source_hash=NULL
                 WHERE feed_id IN (
                    SELECT id FROM feeds WHERE remote_id=?1 AND workspace=?2
                    AND COALESCE(language,'') != COALESCE(?3,'')
                 )",
                params![feed.id, self.workspace, language],
            )?;
            let updated = self.connection.execute(
                "UPDATE feeds SET feed_url=?1,title=?2,site_url=?3,folder=?4,source=?5,language=?6
                 WHERE remote_id=?7 AND workspace=?8",
                params![
                    feed.feed_url,
                    feed.title,
                    feed.site_url,
                    folder,
                    kind.key(),
                    language,
                    feed.id,
                    self.workspace
                ],
            )?;
            if updated == 0 {
                self.connection.execute(
                    "INSERT INTO feeds(feed_url,title,site_url,folder,source,remote_id,workspace,language)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(workspace,feed_url) DO UPDATE SET
                    title=excluded.title,site_url=excluded.site_url,folder=excluded.folder,
                    source=excluded.source,remote_id=excluded.remote_id,language=excluded.language",
                    params![
                        feed.feed_url,
                        feed.title,
                        feed.site_url,
                        folder,
                        kind.key(),
                        feed.id,
                        self.workspace,
                        language
                    ],
                )?;
            }
        }
        let existing = {
            let mut statement = self
                .connection
                .prepare("SELECT id,remote_id FROM feeds WHERE source=?1 AND workspace=?2")?;
            statement
                .query_map(params![kind.key(), self.workspace], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut result = HashMap::new();
        for (local_id, remote_id) in existing {
            if seen.contains(&remote_id) {
                result.insert(remote_id, local_id);
            } else {
                self.clear_pending_for_feed(local_id)?;
                self.connection.execute(
                    "DELETE FROM feeds WHERE id=?1 AND workspace=?2",
                    params![local_id, self.workspace],
                )?;
            }
        }
        Ok(result)
    }

    #[cfg(test)]
    pub fn save_remote_entries(
        &mut self,
        entries: &[RemoteEntry],
        feed_ids: &HashMap<i64, i64>,
        kind: ProviderKind,
    ) -> anyhow::Result<usize> {
        let prepared = Self::prepare_remote_entries(entries.to_vec(), kind)?;
        self.save_prepared_remote_entries(&prepared, feed_ids)
    }

    pub fn prepare_remote_entries(
        entries: Vec<RemoteEntry>,
        kind: ProviderKind,
    ) -> anyhow::Result<Vec<PreparedRemoteEntry>> {
        Self::prepare_remote_entries_with_revisions(entries, kind, &HashMap::new())
    }

    pub fn prepare_remote_entries_with_revisions(
        entries: Vec<RemoteEntry>,
        kind: ProviderKind,
        known_revisions: &HashMap<i64, String>,
    ) -> anyhow::Result<Vec<PreparedRemoteEntry>> {
        entries
            .into_iter()
            .map(|entry| {
                let remote_hash = hex::encode(Sha256::digest(entry.content.as_bytes()));
                if entry.status == "removed" {
                    return Ok(PreparedRemoteEntry {
                        guid: format!("{}:{}", kind.key(), entry.id),
                        entry,
                        remote_hash,
                        html: String::new(),
                        snippet: String::new(),
                        compressed_source: Vec::new(),
                    });
                }
                if known_revisions.get(&entry.id) == Some(&remote_hash) {
                    return Ok(PreparedRemoteEntry {
                        guid: format!("{}:{}", kind.key(), entry.id),
                        entry,
                        remote_hash,
                        html: String::new(),
                        snippet: String::new(),
                        compressed_source: Vec::new(),
                    });
                }
                let html = sanitize_html(&entry.content, entry.url.as_deref());
                let snippet = plain_text(&html).chars().take(280).collect();
                let compressed_source = compress_html(&entry.content)?;
                Ok(PreparedRemoteEntry {
                    guid: format!("{}:{}", kind.key(), entry.id),
                    entry,
                    remote_hash,
                    html,
                    snippet,
                    compressed_source,
                })
            })
            .collect()
    }

    pub fn save_prepared_remote_entries(
        &mut self,
        entries: &[PreparedRemoteEntry],
        feed_ids: &HashMap<i64, i64>,
    ) -> anyhow::Result<usize> {
        let mut saved = 0;
        for batch in entries.chunks(100) {
            let transaction = self.connection.transaction()?;
            for prepared in batch {
                let entry = &prepared.entry;
                let Some(feed_id) = feed_ids.get(&entry.feed_id) else {
                    continue;
                };
                if entry.status == "removed" {
                    transaction.execute(
                        "DELETE FROM articles WHERE remote_id=?1 AND feed_id=?2",
                        params![entry.id, feed_id],
                    )?;
                    continue;
                }
                let remote_hash = &prepared.remote_hash;
                let is_read = entry.status == "read";
                let existing: Option<(
                    i64,
                    Option<String>,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    bool,
                    bool,
                )> = transaction
                    .query_row(
                        "SELECT id,remote_content_hash,title,url,author,published_at,is_read,is_starred
                         FROM articles WHERE feed_id=?1 AND guid=?2",
                        params![feed_id, prepared.guid],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                                row.get(5)?,
                                row.get::<_, i64>(6)? != 0,
                                row.get::<_, i64>(7)? != 0,
                            ))
                        },
                    )
                    .optional()?;

                if let Some((
                    article_id,
                    stored_hash,
                    old_title,
                    old_url,
                    old_author,
                    old_published_at,
                    old_is_read,
                    old_is_starred,
                )) = existing
                {
                    let content_changed = stored_hash.as_deref() != Some(remote_hash.as_str());
                    let title_changed = old_title != entry.title;
                    let author_changed = old_author != entry.author;
                    let changed = content_changed
                        || title_changed
                        || old_url != entry.url
                        || author_changed
                        || old_published_at != entry.published_at
                        || old_is_read != is_read
                        || old_is_starred != entry.starred;
                    if !changed {
                        continue;
                    }

                    if content_changed {
                        transaction.execute(
                            "UPDATE articles SET title=?1,url=?2,author=?3,published_at=?4,
                                snippet=?5,content_html=?6,
                                source_html=CASE WHEN ?6='' THEN source_html ELSE ?7 END,
                                content_revision=content_revision+1,
                                remote_content_hash=?8,source_revision=?8,is_read=?9,is_starred=?10,
                                extraction_pipeline_hash=NULL,extracted_html=NULL,
                                processed_html=NULL,processed_source_hash=NULL,processed_pipeline_hash=NULL,
                                translated_html=NULL,translated_title=NULL,translated_lang=NULL,
                                translation_source_hash=NULL,auto_translated_title=NULL,
                                auto_translated_title_lang=NULL,auto_translated_title_source_hash=NULL
                             WHERE id=?11",
                            params![
                                entry.title,
                                entry.url,
                                entry.author,
                                entry.published_at,
                                prepared.snippet,
                                prepared.html,
                                prepared.compressed_source,
                                remote_hash,
                                is_read,
                                entry.starred,
                                article_id,
                            ],
                        )?;
                    } else {
                        transaction.execute(
                            "UPDATE articles SET title=?1,url=?2,author=?3,published_at=?4,
                                is_read=?5,is_starred=?6,
                                content_revision=content_revision+CASE WHEN title IS NOT ?1 THEN 1 ELSE 0 END
                             WHERE id=?7",
                            params![
                                entry.title,
                                entry.url,
                                entry.author,
                                entry.published_at,
                                is_read,
                                entry.starred,
                                article_id,
                            ],
                        )?;
                    }
                    if content_changed || title_changed || author_changed {
                        sync_article_fts(&transaction, article_id)?;
                    }
                    saved += 1;
                } else {
                    transaction.execute(
                            "INSERT INTO articles(feed_id,guid,title,url,author,published_at,snippet,
                            content_html,source_html,is_read,is_starred,remote_id,remote_content_hash,source_revision)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)",
                        params![
                            feed_id,
                            prepared.guid,
                            entry.title,
                            entry.url,
                            entry.author,
                            entry.published_at,
                            prepared.snippet,
                            prepared.html,
                            prepared.compressed_source,
                            is_read,
                            entry.starred,
                            entry.id,
                            remote_hash,
                        ],
                    )?;
                    let article_id = transaction.last_insert_rowid();
                    sync_article_fts(&transaction, article_id)?;
                    saved += 1;
                }
            }
            transaction.commit()?;
        }
        Ok(saved)
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

    fn clear_pending_for_feed(&self, feed_id: i64) -> anyhow::Result<()> {
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
    async fn fetch_feed(
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
    fn save_feed(
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
    }

    fn write_prepared_feed(
        &mut self,
        url: &str,
        feed: PreparedFeed,
        existing_id: Option<i64>,
        etag: Option<String>,
        modified: Option<String>,
    ) -> anyhow::Result<()> {
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
                }
            }
            transaction.commit()?;
        }
        Ok(())
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

    pub async fn prepare_extraction_using(
        &self,
        article_id: i64,
        force: bool,
        extractor: ContentExtractor,
        pipeline_hash: &str,
        resolve_url: impl FnOnce(&str) -> anyhow::Result<String>,
        prepare: impl FnOnce(&str, &str, &str) -> anyhow::Result<Option<(String, bool)>>,
    ) -> anyhow::Result<Option<PreparedExtraction>> {
        let (existing, existing_hash, cached_page): (Option<String>, Option<String>, Option<Vec<u8>>) = self.connection.query_row(
            "SELECT extracted_html,extraction_pipeline_hash,source_page_html FROM articles WHERE id=?1",
            [article_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if !force
            && existing
                .as_deref()
                .is_some_and(|html| !html.trim().is_empty())
            && existing_hash.is_none()
            && cached_page.is_none()
        {
            return Ok(None);
        }
        let (url, title): (String, String) = self.connection.query_row(
            "SELECT url,title FROM articles WHERE id=?1",
            [article_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (resolved, raw) = if !force && let Some(cached) = cached_page {
            (url.clone(), decompress_html(&cached)?)
        } else {
            let resolved = resolve_url(&url)?;
            let referer = (resolved != url).then_some(url.as_str());
            let raw = fetch_html(&self.client, &resolved, referer).await?;
            (resolved, raw)
        };
        let mut cache_hasher = Sha256::new();
        cache_hasher.update(pipeline_hash.as_bytes());
        cache_hasher.update(Sha256::digest(raw.as_bytes()));
        let pipeline_hash = hex::encode(cache_hasher.finalize());
        if !force
            && existing
                .as_deref()
                .is_some_and(|html| !html.trim().is_empty())
            && existing_hash.as_deref() == Some(pipeline_hash.as_str())
        {
            return Ok(None);
        }
        let compressed_raw = compress_html(&raw)?;
        let prepared = prepare(&raw, &resolved, &title)?;
        let (sanitized, body_selected) = match prepared {
            Some((html, true)) => (panda_content::sanitize_html(&html, Some(&resolved)), true),
            Some((html, false)) => (
                extract_article_html(&html, &resolved, &title, extractor),
                false,
            ),
            None => (
                extract_article_html(&raw, &resolved, &title, extractor),
                false,
            ),
        };
        // A selected body still needs a basic content-quality check before it can
        // bypass the regular extractor; short or empty plugin output falls back.
        let sanitized = if body_selected && plain_text(&sanitized).trim().len() < 80 {
            extract_article_html(&raw, &resolved, &title, extractor)
        } else {
            sanitized
        };
        if plain_text(&sanitized).trim().len() < 80 {
            anyhow::bail!("No extractable article content found on this page");
        }
        Ok(Some(PreparedExtraction {
            article_id,
            resolved_url: resolved,
            source_page_html: compressed_raw,
            extracted_html: sanitized,
            pipeline_hash,
        }))
    }

    pub fn persist_extraction(&self, prepared: PreparedExtraction) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE articles SET url=?1,source_page_html=?2, extracted_html=?3, extraction_pipeline_hash=?4,
                processed_html=NULL, processed_source_hash=NULL, processed_pipeline_hash=NULL,
                translated_html=NULL, translated_title=NULL, translated_lang=NULL, translation_source_hash=NULL,
                content_revision=content_revision+1 WHERE id=?5",
            params![prepared.resolved_url, prepared.source_page_html, prepared.extracted_html, prepared.pipeline_hash, prepared.article_id],
        )?;
        sync_article_fts(&self.connection, prepared.article_id)?;
        Ok(())
    }

    #[cfg(test)]
    pub async fn import_opml(&mut self, source: &str) -> anyhow::Result<usize> {
        let document = OPML::from_str(source)?;
        let mut entries = Vec::new();
        for outline in &document.body.outlines {
            collect_outlines(outline, None, &mut entries);
        }
        let mut added = 0;
        for (url, title, folder) in entries {
            let url = match normalize_http_url(&url) {
                Ok(url) => url,
                Err(_) => continue,
            };
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
                continue;
            }
            match self.fetch_feed(&url, None, None).await {
                Ok((Some(feed), etag, modified)) => {
                    self.save_feed(&url, feed, None, etag, modified)?;
                    if let Some(folder) = folder {
                        self.connection.execute(
                            "UPDATE feeds SET folder=?1 WHERE feed_url=?2 AND workspace=?3",
                            params![folder, url, self.workspace],
                        )?;
                    }
                    if let Some(title) = title.filter(|name| !name.trim().is_empty()) {
                        self.connection.execute(
                            "UPDATE feeds SET title=?1 WHERE feed_url=?2 AND workspace=?3",
                            params![title, url, self.workspace],
                        )?;
                    }
                    added += 1;
                }
                Ok((None, _, _)) => {}
                Err(error) => eprintln!("skipping OPML feed {url}: {error}"),
            }
        }
        Ok(added)
    }

    pub fn export_opml(&self) -> anyhow::Result<String> {
        let mut statement = self.connection.prepare("SELECT COALESCE(custom_title,title),feed_url,folder FROM feeds WHERE workspace=?1 ORDER BY COALESCE(folder,''),COALESCE(custom_title,title) COLLATE NOCASE")?;
        let rows = statement
            .query_map([&self.workspace], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut groups: BTreeMap<Option<String>, Vec<Outline>> = BTreeMap::new();
        for (title, url, folder) in rows {
            groups.entry(folder).or_default().push(Outline {
                text: title,
                xml_url: Some(url),
                r#type: Some("rss".into()),
                ..Outline::default()
            });
        }
        let mut document = OPML {
            head: Some(Head {
                title: Some("Panda Reader subscriptions".into()),
                ..Head::default()
            }),
            ..OPML::default()
        };
        for (folder, outlines) in groups {
            if let Some(folder) = folder.filter(|s| !s.trim().is_empty()) {
                document.body.outlines.push(Outline {
                    text: folder,
                    outlines,
                    ..Outline::default()
                });
            } else {
                document.body.outlines.extend(outlines);
            }
        }
        Ok(document.to_string()?)
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

fn sync_article_fts(connection: &Connection, article_id: i64) -> anyhow::Result<()> {
    let article = connection
        .query_row(
            "SELECT title,author,content_html,processed_html,translated_html FROM articles WHERE id=?1",
            [article_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((title, author, content_html, canonical_html, translated_html)) = article else {
        connection.execute("DELETE FROM article_fts WHERE rowid=?1", [article_id])?;
        return Ok(());
    };
    let source_html = canonical_html.unwrap_or(content_html);
    let content_text = plain_text(&source_html);
    let translated_text = translated_html
        .as_deref()
        .filter(|html| !html.trim().is_empty())
        .map(plain_text)
        .unwrap_or_default();
    connection.execute("DELETE FROM article_fts WHERE rowid=?1", [article_id])?;
    connection.execute(
        "INSERT INTO article_fts(rowid,title,author,content_text,translated_text) VALUES(?1,?2,?3,?4,?5)",
        params![
            article_id,
            cjk_token_text(&title),
            cjk_token_text(author.as_deref().unwrap_or_default()),
            cjk_token_text(&content_text),
            cjk_token_text(&translated_text),
        ],
    )?;
    Ok(())
}

fn fts_query(search: &str) -> String {
    let normalized = cjk_token_text(search.trim());
    if normalized.is_empty() {
        String::new()
    } else {
        format!("\"{}\"", normalized.replace('"', "\"\""))
    }
}

// Separate adjacent Han characters for unicode61 so CJK searches can match
// short phrases instead of requiring the entire uninterrupted run to match.
fn cjk_token_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous_was_han = false;
    for character in value.chars() {
        let is_han = matches!(
            character as u32,
            0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xF900..=0xFAFF
                | 0x20000..=0x2FA1F
                | 0x30000..=0x323AF
        );
        if is_han && previous_was_han {
            output.push(' ');
        }
        output.push(character);
        previous_was_han = is_han;
    }
    output
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !names.iter().any(|name| name == column) {
        connection.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

fn migrate_workspace_schema(connection: &Connection) -> anyhow::Result<()> {
    let has_workspace = {
        let mut statement = connection.prepare("PRAGMA table_info(feeds)")?;
        statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == "workspace")
    };
    if !has_workspace {
        connection.execute_batch(
            "SAVEPOINT workspace_migration;
             CREATE TABLE feeds_workspace (
                id INTEGER PRIMARY KEY,
                feed_url TEXT NOT NULL,
                title TEXT NOT NULL,
                site_url TEXT,
                folder TEXT,
                etag TEXT,
                last_modified TEXT,
                last_error TEXT,
                added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                source TEXT NOT NULL DEFAULT 'local',
                remote_id INTEGER,
                auto_translate_titles INTEGER NOT NULL DEFAULT 0,
                auto_translate_titles_after_article_id INTEGER,
                workspace TEXT NOT NULL DEFAULT 'local',
                UNIQUE(workspace,feed_url)
             );",
        )?;
        let account_workspace = "provider:miniflux";
        connection.execute(
            "INSERT INTO feeds_workspace(id,feed_url,title,site_url,folder,etag,last_modified,last_error,added_at,source,remote_id,auto_translate_titles,auto_translate_titles_after_article_id,workspace)
             SELECT id,feed_url,title,site_url,folder,etag,last_modified,last_error,added_at,source,remote_id,auto_translate_titles,auto_translate_titles_after_article_id,
                    CASE WHEN source='miniflux' THEN ?1 ELSE 'local' END FROM feeds",
            [&account_workspace],
        )?;
        connection.execute_batch(
            "DROP TABLE feeds;
             ALTER TABLE feeds_workspace RENAME TO feeds;
             RELEASE SAVEPOINT workspace_migration;",
        )?;
    }

    let remote_state_has_workspace = {
        let mut statement = connection.prepare("PRAGMA table_info(remote_state)")?;
        statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == "workspace")
    };
    if !remote_state_has_workspace {
        connection.execute_batch(
            "ALTER TABLE remote_state RENAME TO remote_state_legacy;
             CREATE TABLE remote_state (workspace TEXT PRIMARY KEY, account TEXT NOT NULL);
             INSERT INTO remote_state(workspace,account)
                 SELECT 'provider:miniflux', account FROM remote_state_legacy;
             DROP TABLE remote_state_legacy;",
        )?;
    }

    let pending_has_workspace = {
        let mut statement = connection.prepare("PRAGMA table_info(pending_remote_marks)")?;
        statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == "workspace")
    };
    if !pending_has_workspace {
        connection.execute_batch(
            "ALTER TABLE pending_remote_marks RENAME TO pending_remote_marks_legacy;
             CREATE TABLE pending_remote_marks (
                workspace TEXT NOT NULL,
                remote_id INTEGER NOT NULL,
                field TEXT NOT NULL,
                value INTEGER NOT NULL,
                PRIMARY KEY(workspace,remote_id,field)
             );
             INSERT OR REPLACE INTO pending_remote_marks(workspace,remote_id,field,value)
                 SELECT f.workspace,p.remote_id,p.field,p.value
                 FROM pending_remote_marks_legacy p
                 JOIN articles a ON a.remote_id=p.remote_id
                 JOIN feeds f ON f.id=a.feed_id;
             DROP TABLE pending_remote_marks_legacy;",
        )?;
    }
    Ok(())
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

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36";

async fn fetch_html(client: &Client, url: &str, referer: Option<&str>) -> anyhow::Result<String> {
    let mut request = client
        .get(url)
        .header(reqwest::header::USER_AGENT, BROWSER_UA)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(
            reqwest::header::ACCEPT_LANGUAGE,
            "zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7",
        );
    if let Some(referer) = referer {
        request = request.header(reqwest::header::REFERER, referer);
    }
    let response = request.send().await?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!(
            "Publisher blocked automated article downloads ({status}). Open the original page instead."
        );
    }
    let response = response.error_for_status()?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = read_response_limited(response, MAX_ARTICLE_PAGE_BYTES, "article page").await?;
    let charset = content_type
        .split(';')
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("charset")
                .then_some(value.trim())
        })
        .unwrap_or("utf-8")
        .trim_matches([' ', '\'', '"']);
    let encoding =
        encoding_rs::Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    Ok(encoding.decode(&bytes).0.into_owned())
}

async fn read_response_limited(
    mut response: reqwest::Response,
    limit: usize,
    label: &str,
) -> anyhow::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        anyhow::bail!("{label} response exceeds the {limit}-byte limit");
    }
    let mut bytes =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(limit as u64) as usize);
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            anyhow::bail!("{label} response exceeds the {limit}-byte limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
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
mod tests {
    use super::*;
    use panda_core::Scope;

    #[test]
    fn migration_is_versioned_and_read_connections_do_not_change_schema() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("migration.sqlite3");
        Store::migrate(&path).unwrap();
        let connection = Connection::open(&path).unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 5);
        connection
            .execute(
                "UPDATE article_fts_state SET last_article_id=99,complete=1 WHERE id=1",
                [],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 4).unwrap();
        drop(connection);
        Store::migrate(&path).unwrap();
        let connection = Connection::open(&path).unwrap();
        let (last_id, complete): (i64, i64) = connection
            .query_row(
                "SELECT last_article_id,complete FROM article_fts_state WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((last_id, complete), (0, 0));
        assert!(
            std::fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .contains("pre-migration-"))
        );
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 5);
        let before: i64 = connection
            .query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| row.get(0))
            .unwrap();
        drop(connection);
        let reader = Store::open_read_workspace(&path, "local").unwrap();
        reader.snapshot(Scope::All, "", 20, None, false).unwrap();
        drop(reader);
        let connection = Connection::open(&path).unwrap();
        let after: i64 = connection
            .query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, after);
    }

    #[tokio::test]
    async fn feed_download_rejects_oversized_declared_and_streamed_bodies() {
        use std::{io::Write as _, net::TcpListener};

        for response in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n4\r\nefgh\r\n0\r\n\r\n".as_slice(),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/feed", listener.local_addr().unwrap());
            let response = response.to_vec();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut request = [0u8; 2048];
                let _ = std::io::Read::read(&mut stream, &mut request);
                let _ = stream.write_all(&response);
            });
            let response = Client::new().get(url).send().await.unwrap();
            let error = read_response_limited(response, 5, "test feed")
                .await
                .unwrap_err();
            assert!(error.to_string().contains("5-byte limit"));
            server.join().unwrap();
        }
    }

    #[test]
    fn scope_indexes_match_their_filters_and_sort_order() {
        let (_directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(id,feed_url,title,folder) VALUES(1,'https://example.org/feed','feed','folder')",
                [],
            )
            .unwrap();
        for index in 0..2_000 {
            store.connection.execute(
                "INSERT INTO articles(feed_id,guid,title,published_at,is_read,is_starred,read_later)
                 VALUES(1,?1,?2,?3,?4,?5,?6)",
                params![
                    index.to_string(),
                    format!("article {index}"),
                    format!("2026-01-{:02}-{:02}", index % 12 + 1, index % 27 + 1),
                    index % 20 != 0,
                    index % 5 == 0,
                    index % 7 == 0,
                ],
            ).unwrap();
        }
        store.connection.execute_batch("ANALYZE").unwrap();
        let plan = |sql: &str| -> String {
            let mut statement = store.connection.prepare(sql).unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join("\n")
        };
        let all_plan = plan(
            "EXPLAIN QUERY PLAN SELECT id FROM articles ORDER BY COALESCE(published_at,'') DESC,id DESC LIMIT 20",
        );
        assert!(all_plan.contains("idx_articles_sort"), "{all_plan}");
        let unread_plan = plan(
            "EXPLAIN QUERY PLAN SELECT id FROM articles WHERE is_read=0 ORDER BY COALESCE(published_at,'') DESC,id DESC LIMIT 20",
        );
        assert!(
            unread_plan.contains("idx_articles_unread_sort"),
            "{unread_plan}"
        );
        assert!(plan("EXPLAIN QUERY PLAN SELECT id FROM articles WHERE is_starred=1 ORDER BY COALESCE(published_at,'') DESC,id DESC LIMIT 20").contains("idx_articles_starred_sort"));
        assert!(plan("EXPLAIN QUERY PLAN SELECT id FROM articles WHERE read_later=1 ORDER BY COALESCE(published_at,'') DESC,id DESC LIMIT 20").contains("idx_articles_later_sort"));
        assert!(plan("EXPLAIN QUERY PLAN SELECT id FROM articles WHERE feed_id=1 ORDER BY COALESCE(published_at,'') DESC,id DESC LIMIT 20").contains("idx_articles_feed_sort"));
        let folder_plan = plan(
            "EXPLAIN QUERY PLAN SELECT a.id FROM feeds f JOIN articles a ON a.feed_id=f.id WHERE f.workspace='local' AND f.folder='folder' ORDER BY COALESCE(a.published_at,'') DESC,a.id DESC LIMIT 20",
        );
        assert!(
            folder_plan.contains("idx_feeds_workspace_folder"),
            "{folder_plan}"
        );
        let unread_counts = plan(
            "EXPLAIN QUERY PLAN SELECT feed_id,COUNT(*) FROM articles WHERE is_read=0 GROUP BY feed_id",
        );
        assert!(
            unread_counts.contains("idx_articles_feed_unread"),
            "{unread_counts}"
        );
        assert!(
            plan(
                "EXPLAIN QUERY PLAN SELECT rowid FROM article_fts WHERE article_fts MATCH 'article'"
            )
            .contains("VIRTUAL TABLE INDEX")
        );
    }

    fn test_store() -> (tempfile::TempDir, Store) {
        test_store_for("local")
    }

    fn test_store_for(workspace: &str) -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().expect("temporary database directory");
        let store = Store::open_writer(&directory.path().join("reader.sqlite3"), workspace)
            .expect("open test database");
        (directory, store)
    }

    #[test]
    fn full_text_search_covers_metadata_body_translation_and_cjk_then_tracks_changes() {
        let (_directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(id,feed_url,title) VALUES(1,'https://example.org/feed','feed')",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO articles(id,feed_id,guid,title,author,content_html,translated_html) VALUES(1,1,'one','湖南大学找到植物生长开关','Ada Lovelace','<p>the original article body contains quantum foam</p>','<p>译文里有月球基地</p>')",
                [],
            )
            .unwrap();
        sync_article_fts(&store.connection, 1).unwrap();

        for query in ["湖南大学", "quantum foam", "月球基地", "Ada Lovelace"] {
            assert_eq!(
                store
                    .snapshot(Scope::All, query, 20, None, false)
                    .unwrap()
                    .articles
                    .len(),
                1,
                "query should match: {query}"
            );
        }

        store
            .save_processed_content(
                1,
                "<p>raw body</p>",
                "https://example.org/article",
                "湖南大学找到植物生长开关",
                "canonical-v1",
                "<p>replacement body about ocean currents</p>",
            )
            .unwrap();
        assert_eq!(
            store
                .snapshot(Scope::All, "quantum foam", 20, None, false)
                .unwrap()
                .articles
                .len(),
            0
        );
        assert_eq!(
            store
                .snapshot(Scope::All, "ocean currents", 20, None, false)
                .unwrap()
                .articles
                .len(),
            1
        );
        assert_eq!(
            store
                .snapshot(Scope::All, "original article body", 20, None, false)
                .unwrap()
                .articles
                .len(),
            0
        );
        assert_eq!(
            store
                .snapshot(Scope::All, "月球基地", 20, None, false)
                .unwrap()
                .articles
                .len(),
            0,
            "a canonical revision change must discard the stale translation"
        );
        store
            .connection
            .execute("DELETE FROM articles WHERE id=1", [])
            .unwrap();
        assert_eq!(
            store
                .snapshot(Scope::All, "ocean currents", 20, None, false)
                .unwrap()
                .articles
                .len(),
            0
        );
    }

    #[test]
    fn local_feed_revision_skips_unchanged_entry_preparation() {
        let bytes = include_bytes!("../tests/fixtures/sample_feed.xml");
        let feed = feed_rs::parser::parse(bytes.as_slice()).unwrap();
        let first = Store::prepare_fetched_feed(
            "https://example.com/feed.xml",
            FetchedFeed {
                parsed: Some(feed),
                etag: None,
                modified: None,
            },
        )
        .unwrap();
        let prepared = first.parsed.unwrap();
        let known = prepared
            .articles
            .iter()
            .map(|(article, _, revision)| (article.guid.clone(), revision.clone()))
            .collect::<HashMap<_, _>>();
        let feed = feed_rs::parser::parse(bytes.as_slice()).unwrap();
        let second = Store::prepare_fetched_feed_with_revisions(
            "https://example.com/feed.xml",
            FetchedFeed {
                parsed: Some(feed),
                etag: None,
                modified: None,
            },
            &known,
        )
        .unwrap();
        assert!(second.parsed.unwrap().articles.is_empty());
    }

    #[test]
    fn known_provider_revision_skips_sanitize_and_compression() {
        let entry = RemoteEntry {
            id: 44,
            feed_id: 4,
            title: "Same article".into(),
            url: Some("https://example.com/story".into()),
            author: None,
            published_at: None,
            content: "<script>old</script><p>unchanged</p>".into(),
            status: "unread".into(),
            starred: false,
            changed_at: None,
            revision: None,
        };
        let hash = hex::encode(Sha256::digest(entry.content.as_bytes()));
        let known = HashMap::from([(entry.id, hash)]);
        let prepared = Store::prepare_remote_entries_with_revisions(
            vec![entry],
            ProviderKind::Miniflux,
            &known,
        )
        .unwrap();
        assert_eq!(prepared.len(), 1);
        assert!(prepared[0].html.is_empty());
        assert!(prepared[0].compressed_source.is_empty());
    }

    #[test]
    fn historical_search_index_is_built_in_batches() {
        let (directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(id,feed_url,title) VALUES(1,'https://example.org/feed','feed')",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO articles(id,feed_id,guid,title,content_html) VALUES(1,1,'one','A title','<p>historical material</p>')",
                [],
            )
            .unwrap();
        store
            .connection
            .execute_batch("DROP TABLE article_fts; DROP TABLE article_fts_state;")
            .unwrap();
        drop(store);
        let mut store =
            Store::open_writer(&directory.path().join("reader.sqlite3"), "local").unwrap();
        assert!(!store.index_search_batch(1).unwrap());
        assert_eq!(
            store
                .snapshot(Scope::All, "historical material", 20, None, false)
                .unwrap()
                .articles
                .len(),
            1
        );
        assert!(store.index_search_batch(1).unwrap());
    }

    #[test]
    fn title_translation_cache_is_separate_and_usage_is_persistent_by_day_provider() {
        let (_directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(id,feed_url,title) VALUES(1,'https://example.org/feed','feed')",
                [],
            )
            .unwrap();
        store.connection.execute("INSERT INTO articles(id,feed_id,guid,title) VALUES(7,1,'guid','The government announces a new policy for the country')",[]).unwrap();
        let original = "The government announces a new policy for the country";
        let hash = panda_translate::title_source_hash(original);
        store
            .save_auto_translated_title(7, original, "政府宣布新政策", "zh-Hans", &hash)
            .unwrap();
        let article = store.article(7).unwrap();
        assert_eq!(
            article.summary.auto_translated_title.as_deref(),
            Some("政府宣布新政策")
        );
        assert_eq!(article.translated_html, None);
        store
            .record_translation_usage("2026-10-01", "azure", 2, 99)
            .unwrap();
        store
            .record_translation_usage_with_tokens("2026-10-01", "azure", 1, 40, 23, 11)
            .unwrap();
        store
            .record_translation_usage("2026-09-30", "volcengine", 1, 10)
            .unwrap();
        let usage = store.translation_usage().unwrap();
        assert_eq!(usage.len(), 2);
        assert_eq!(
            (
                usage[0].day.as_str(),
                usage[0].provider.as_str(),
                usage[0].requests,
                usage[0].characters
            ),
            ("2026-10-01", "azure", 3, 139)
        );
        assert_eq!(
            (
                usage[1].day.as_str(),
                usage[1].provider.as_str(),
                usage[1].requests,
                usage[1].characters
            ),
            ("2026-09-30", "volcengine", 1, 10)
        );
        assert_eq!((usage[0].input_tokens, usage[0].output_tokens), (23, 11));
        store.connection.execute("UPDATE articles SET title='The government announces a different policy for the country' WHERE id=7",[]).unwrap();
        let updated = store.article(7).unwrap();
        assert_ne!(
            panda_translate::title_source_hash(&updated.summary.title),
            updated.summary.auto_translated_title_source_hash.unwrap()
        );
    }

    #[test]
    fn translation_segment_cache_is_scoped_by_article_language_backend_and_prompt() {
        let (_directory, store) = test_store();
        store
            .save_translation_segment(
                7,
                "zh-Hans",
                "openai:model-a",
                "prompt-v2",
                "title-hash",
                "segment-a",
                "source-a",
                "<p>译文</p>",
            )
            .unwrap();
        assert_eq!(
            store
                .translation_segments(7, "zh-Hans", "openai:model-a", "prompt-v2", "title-hash")
                .unwrap()
                .get("segment-a")
                .map(String::as_str),
            Some("<p>译文</p>")
        );
        assert!(
            store
                .translation_segments(7, "ja", "openai:model-a", "prompt-v2", "title-hash")
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .translation_segments(7, "zh-Hans", "openai:model-b", "prompt-v2", "title-hash")
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .translation_segments(7, "zh-Hans", "openai:model-a", "prompt-v3", "title-hash")
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .translation_segments(
                    7,
                    "zh-Hans",
                    "openai:model-a",
                    "prompt-v2",
                    "different-title"
                )
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn feed_title_translation_is_manual_and_persisted() {
        let (_directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(id,feed_url,title) VALUES(1,'https://example.org/feed','feed')",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO articles(id,feed_id,guid,title) VALUES(7,1,'guid','A title')",
                [],
            )
            .unwrap();

        let initial = store.snapshot(Scope::All, "", 20, None, true).unwrap();
        assert!(!initial.feeds[0].auto_translate_titles);
        assert!(!initial.articles[0].feed_auto_translate_titles);

        store.set_feed_auto_translate_titles(1, true).unwrap();
        let enabled = store.snapshot(Scope::All, "", 20, None, true).unwrap();
        assert!(enabled.feeds[0].auto_translate_titles);
        assert!(enabled.articles[0].feed_auto_translate_titles);
        assert!(store.article(7).unwrap().summary.feed_auto_translate_titles);

        store.set_feed_auto_translate_titles(1, false).unwrap();
        assert!(!store.article(7).unwrap().summary.feed_auto_translate_titles);
    }

    #[test]
    fn same_feed_url_is_isolated_between_local_and_provider_workspaces() {
        let (directory, local) = test_store_for("local");
        let path = directory.path().join("reader.sqlite3");
        local
            .connection
            .execute(
                "INSERT INTO feeds(feed_url,title,workspace) VALUES(?1,'Local feed','local')",
                ["https://example.com/feed.xml"],
            )
            .unwrap();
        local.connection.execute(
            "INSERT INTO feeds(feed_url,title,source,remote_id,workspace) VALUES(?1,'Remote feed','miniflux',17,'provider:miniflux')",
            ["https://example.com/feed.xml"],
        ).unwrap();
        let provider = Store::open_writer(&path, "provider:miniflux").unwrap();

        assert_eq!(
            local
                .snapshot(Scope::All, "", 50, None, true)
                .unwrap()
                .feeds[0]
                .title,
            "Local feed"
        );
        assert_eq!(
            provider
                .snapshot(Scope::All, "", 50, None, true)
                .unwrap()
                .feeds[0]
                .title,
            "Remote feed"
        );
    }

    #[test]
    fn provider_workspace_promotion_keeps_active_data_until_commit() {
        let (directory, mut active) = test_store_for("provider:miniflux");
        let path = directory.path().join("reader.sqlite3");
        active.connection.execute(
            "INSERT INTO feeds(id,feed_url,title,source,remote_id,workspace) VALUES(1,'https://old.example/feed','Old account','miniflux',1,'provider:miniflux')",
            [],
        ).unwrap();
        active
            .connection
            .execute(
                "INSERT INTO articles(id,feed_id,guid,title) VALUES(11,1,'old-guid','Old article')",
                [],
            )
            .unwrap();
        active.connection.execute(
            "INSERT INTO feeds(id,feed_url,title,source,remote_id,workspace) VALUES(2,'https://local.example/feed','Local feed','local',NULL,'local')",
            [],
        ).unwrap();

        let staging = Store::open_writer(&path, "provider:miniflux:staging").unwrap();
        staging.connection.execute(
            "INSERT INTO feeds(id,feed_url,title,source,remote_id,workspace) VALUES(3,'https://new.example/feed','New account','miniflux',1,'provider:miniflux:staging')",
            [],
        ).unwrap();
        staging
            .connection
            .execute(
                "INSERT INTO articles(id,feed_id,guid,title) VALUES(33,3,'new-guid','New article')",
                [],
            )
            .unwrap();
        staging.connection.execute(
            "INSERT INTO remote_state(workspace,provider,account) VALUES('provider:miniflux:staging','miniflux','new-account')",
            [],
        ).unwrap();

        assert_eq!(
            active
                .snapshot(Scope::All, "", 20, None, true)
                .unwrap()
                .feeds[0]
                .title,
            "Old account"
        );
        active
            .promote_provider_workspace(ProviderKind::Miniflux, "provider:miniflux:staging")
            .unwrap();

        assert_eq!(
            active
                .snapshot(Scope::All, "", 20, None, true)
                .unwrap()
                .feeds[0]
                .title,
            "New account"
        );
        assert_eq!(active.article(33).unwrap().summary.title, "New article");
        assert!(active.article(11).is_err());
        assert_eq!(
            active.provider_sync_state().unwrap().unwrap().account,
            "new-account"
        );
        let local = Store::open_writer(&path, "local").unwrap();
        assert_eq!(
            local
                .snapshot(Scope::All, "", 20, None, true)
                .unwrap()
                .feeds[0]
                .title,
            "Local feed"
        );
    }

    #[test]
    fn migrates_existing_local_and_miniflux_rows_into_separate_workspaces() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.sqlite3");
        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE feeds (
                id INTEGER PRIMARY KEY, feed_url TEXT NOT NULL UNIQUE, title TEXT NOT NULL,
                site_url TEXT, folder TEXT, etag TEXT, last_modified TEXT, last_error TEXT,
                added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, source TEXT NOT NULL DEFAULT 'local', remote_id INTEGER
             );
             CREATE TABLE articles (
                id INTEGER PRIMARY KEY, feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
                guid TEXT NOT NULL, title TEXT NOT NULL, url TEXT, author TEXT, published_at TEXT,
                snippet TEXT NOT NULL DEFAULT '', content_html TEXT NOT NULL DEFAULT '', extracted_html TEXT,
                is_read INTEGER NOT NULL DEFAULT 0, is_starred INTEGER NOT NULL DEFAULT 0, read_later INTEGER NOT NULL DEFAULT 0,
                remote_id INTEGER, translated_html TEXT, translated_title TEXT, translated_lang TEXT,
                translation_source_hash TEXT, UNIQUE(feed_id,guid)
             );
             CREATE TABLE remote_state (id INTEGER PRIMARY KEY CHECK(id=1), account TEXT NOT NULL);
             CREATE TABLE pending_remote_marks (remote_id INTEGER NOT NULL, field TEXT NOT NULL, value INTEGER NOT NULL, PRIMARY KEY(remote_id,field));
             INSERT INTO feeds(id,feed_url,title,source,remote_id) VALUES
                (1,'https://local.example/rss','Local','local',NULL),
                (2,'https://remote.example/rss','Remote','miniflux',42);
             INSERT INTO articles(id,feed_id,guid,title,remote_id) VALUES
                (11,1,'local-guid','Local article',NULL),
                (12,2,'remote-guid','Remote article',99);
             INSERT INTO remote_state(id,account) VALUES(1,'https://miniflux.example:7');
             INSERT INTO pending_remote_marks(remote_id,field,value) VALUES(99,'is_read',1);"
        ).unwrap();
        drop(legacy);

        let local = Store::open_writer(&path, "local").unwrap();
        let provider = Store::open_writer(&path, "provider:miniflux").unwrap();
        assert_eq!(
            local
                .snapshot(Scope::All, "", 20, None, true)
                .unwrap()
                .feeds
                .len(),
            1
        );
        assert_eq!(
            provider
                .snapshot(Scope::All, "", 20, None, true)
                .unwrap()
                .feeds
                .len(),
            1
        );
        assert_eq!(local.article(11).unwrap().summary.title, "Local article");
        assert_eq!(
            provider.article(12).unwrap().summary.title,
            "Remote article"
        );
        let account: String = provider
            .connection
            .query_row(
                "SELECT account FROM remote_state WHERE workspace='provider:miniflux'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(account, "https://miniflux.example:7");
        let queued: i64 = provider.connection.query_row(
            "SELECT COUNT(*) FROM pending_remote_marks WHERE workspace='provider:miniflux' AND remote_id=99", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(queued, 1);
    }

    #[test]
    fn miniflux_entries_keep_remote_state_and_local_later_flag() {
        let (_directory, mut store) = test_store_for("provider:miniflux");
        let remote_feed = RemoteFeed {
            id: 42,
            title: "远程订阅".into(),
            feed_url: "https://example.com/feed.xml".into(),
            site_url: "https://example.com".into(),
            language: Some("en".into()),
            category: Some(panda_providers::RemoteCategory {
                id: 3,
                title: "技术".into(),
            }),
        };
        let ids = store
            .save_remote_feeds(&[remote_feed.clone()], ProviderKind::Miniflux)
            .unwrap();
        let entry = RemoteEntry {
            id: 99,
            feed_id: 42,
            title: "远程文章".into(),
            url: Some("https://example.com/post".into()),
            author: None,
            published_at: Some("2026-09-28T00:00:00Z".into()),
            content: "<p>正文</p><script>alert(1)</script>".into(),
            status: "unread".into(),
            starred: false,
            changed_at: None,
            revision: None,
        };
        store
            .save_remote_entries(&[entry.clone()], &ids, ProviderKind::Miniflux)
            .unwrap();
        let snapshot = store.snapshot(Scope::All, "", 500, None, true).unwrap();
        assert_eq!(snapshot.feeds[0].folder.as_deref(), Some("技术"));
        let article_id = snapshot.articles[0].id;
        assert_eq!(snapshot.articles[0].feed_language.as_deref(), Some("en"));
        assert_eq!(store.remote_entry_id(article_id).unwrap(), Some(99));
        let article = store.article(article_id).unwrap();
        assert_eq!(article.summary.feed_language.as_deref(), Some("en"));
        assert!(!article.content_html.contains("<script"));
        store.mark(article_id, MarkField::Later, true).unwrap();

        let mut changed = entry;
        changed.status = "read".into();
        changed.starred = true;
        store
            .save_remote_entries(&[changed], &ids, ProviderKind::Miniflux)
            .unwrap();
        let article = store.article(article_id).unwrap();
        assert!(article.summary.is_read);
        assert!(article.summary.is_starred);
        assert!(article.summary.read_later);
        store.mark(article_id, MarkField::Starred, false).unwrap();
        store.mark(article_id, MarkField::Starred, true).unwrap();
        let source_title = store.article(article_id).unwrap().summary.title;
        store
            .save_auto_translated_title(
                article_id,
                &source_title,
                "Translated title",
                "zh-Hans",
                &panda_translate::title_source_hash(&source_title),
            )
            .unwrap();
        let mut language_changed_feed = remote_feed.clone();
        language_changed_feed.language = Some("fr".into());
        store
            .save_remote_feeds(&[language_changed_feed], ProviderKind::Miniflux)
            .unwrap();
        let updated = store.article(article_id).unwrap();
        assert_eq!(updated.summary.feed_language.as_deref(), Some("fr"));
        assert!(updated.summary.auto_translated_title.is_none());
        let queued: (i64, bool) = store.connection.query_row(
            "SELECT COUNT(*),MAX(value) FROM pending_remote_marks WHERE workspace='provider:miniflux' AND remote_id=99 AND field='is_starred'",
            [], |row| Ok((row.get(0)?, row.get(1)?))
        ).unwrap();
        assert_eq!(queued, (1, true));

        let mut removed_feed = remote_feed;
        removed_feed.title = "更名订阅".into();
        removed_feed.feed_url = "https://example.com/new-feed.xml".into();
        store
            .save_remote_feeds(&[removed_feed], ProviderKind::Miniflux)
            .unwrap();
        assert_eq!(store.remote_feed_id(ids[&42]).unwrap(), Some(42));
    }

    #[test]
    fn flush_keeps_a_newer_mark_queued_while_the_previous_value_is_in_flight() {
        use std::{
            io::{Read as _, Write as _},
            sync::mpsc,
            time::Duration,
        };

        let (directory, mut store) = test_store_for("provider:miniflux");
        store.connection.execute(
            "INSERT INTO feeds(id,feed_url,title,workspace,source,remote_id) VALUES(1,'https://example.com/rss','Feed','provider:miniflux','miniflux',1)",
            [],
        ).unwrap();
        store.connection.execute(
            "INSERT INTO articles(id,feed_id,guid,title,remote_id) VALUES(1,1,'1','Article',42)",
            [],
        ).unwrap();
        store.mark(1, MarkField::Read, true).unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (received_tx, received_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0u8; 4096];
            let count = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).contains("PUT /v1/entries"));
            received_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        let client = ProviderClient::new(
            ProviderKind::Miniflux,
            &panda_providers::ProviderSettings {
                endpoint,
                username: String::new(),
                secret: "token".into(),
            },
        )
        .unwrap();
        let path = directory.path().join("reader.sqlite3");
        let flush = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime
                .block_on(store.flush_provider_marks(&client))
                .unwrap()
        });
        received_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        let newer = Store::open_writer(&path, "provider:miniflux").unwrap();
        newer.mark(1, MarkField::Read, false).unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(flush.join().unwrap(), 0);
        server.join().unwrap();

        let pending: (i64, bool) = newer
            .connection
            .query_row(
                "SELECT COUNT(*),MAX(value) FROM pending_remote_marks WHERE workspace='provider:miniflux' AND remote_id=42 AND field='is_read'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(pending, (1, false));
    }

    #[test]
    fn edited_feed_fields_and_translation_preference_survive_refresh() {
        use std::io::{Read as _, Write as _};

        let (directory, mut store) = test_store();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let feed_url = format!("http://{}/rss", listener.local_addr().unwrap());
        store
            .connection
            .execute(
                "INSERT INTO feeds(id,feed_url,title) VALUES(1,?1,'Source title')",
                [&feed_url],
            )
            .unwrap();
        store
            .update_feed(1, "My title", None, &feed_url, true)
            .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            stream.read(&mut request).unwrap();
            let body = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Source title</title><link>https://example.com</link><description>Feed</description><item><guid>entry-1</guid><title>Entry</title><link>https://example.com/post</link><description>Body text</description></item></channel></rss>"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(store.refresh_feed(1)).unwrap();
        server.join().unwrap();

        let snapshot = store.snapshot(Scope::All, "", 20, None, true).unwrap();
        assert_eq!(snapshot.feeds[0].title, "My title");
        assert!(snapshot.feeds[0].auto_translate_titles);
        let article = store.article(snapshot.articles[0].id).unwrap();
        assert_eq!(article.summary.feed_title, "My title");
        assert!(article.summary.feed_auto_translate_titles);
        drop(directory);
    }

    #[test]
    fn provider_sync_downloads_feeds_and_articles() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let replies = [
                r#"{"id":5,"username":"reader"}"#,
                r#"[{"id":42,"title":"远程源","feed_url":"https://example.com/rss","site_url":"https://example.com","category":{"id":1,"title":"科技"}}]"#,
                r#"{"total":1,"entries":[{"id":99,"feed_id":42,"title":"远程文章","url":"https://example.com/article","content":"<p>正文</p>","status":"unread","starred":true}]}"#,
            ];
            for body in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                stream.read(&mut request).unwrap();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let (_directory, mut store) = test_store_for("provider:miniflux");
        let remote = ProviderClient::new(
            ProviderKind::Miniflux,
            &panda_providers::ProviderSettings {
                endpoint: format!("http://{address}"),
                username: String::new(),
                secret: "secret".into(),
            },
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            runtime
                .block_on(store.sync_provider(&remote, ProviderKind::Miniflux))
                .unwrap(),
            1
        );
        let snapshot = store.snapshot(Scope::Starred, "", 500, None, true).unwrap();
        assert_eq!(snapshot.feeds[0].folder.as_deref(), Some("科技"));
        assert_eq!(snapshot.articles[0].title, "远程文章");
        assert_eq!(
            store.remote_entry_id(snapshot.articles[0].id).unwrap(),
            Some(99)
        );
        server.join().unwrap();
    }

    #[test]
    fn provider_incremental_sync_fetches_changes_and_skips_unchanged_content() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let feeds = r#"[{"id":42,"title":"Feed","feed_url":"https://example.com/rss","site_url":"https://example.com","category":{"id":1,"title":"News"}}]"#;
            let initial = r#"{"total":1,"entries":[{"id":99,"feed_id":42,"title":"Story","url":"https://example.com/post","content":"<p>Body</p>","status":"unread","starred":false}]}"#;
            let changed = r#"{"total":1,"entries":[{"id":99,"feed_id":42,"title":"Story","url":"https://example.com/post","content":"<p>Body</p>","status":"read","starred":true}]}"#;
            let replies = [
                (r#"{"id":5,"username":"reader"}"#, ""),
                (feeds, ""),
                (initial, ""),
                (r#"{"id":5,"username":"reader"}"#, ""),
                (feeds, ""),
                (initial, "changed_after"),
                (r#"{"id":5,"username":"reader"}"#, ""),
                (feeds, ""),
                (changed, "changed_after"),
            ];
            let mut requests = Vec::new();
            for (body, expected_query) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]).to_string();
                if !expected_query.is_empty() {
                    assert!(request.lines().next().unwrap().contains(expected_query));
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                requests.push(request);
            }
            requests
        });

        let (_directory, mut store) = test_store_for("provider:miniflux");
        let remote = ProviderClient::new(
            ProviderKind::Miniflux,
            &panda_providers::ProviderSettings {
                endpoint: format!("http://{address}"),
                username: String::new(),
                secret: "secret".into(),
            },
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        assert_eq!(
            runtime
                .block_on(store.sync_provider(&remote, ProviderKind::Miniflux))
                .unwrap(),
            1
        );
        assert_eq!(
            runtime
                .block_on(store.sync_provider(&remote, ProviderKind::Miniflux))
                .unwrap(),
            0
        );
        assert_eq!(
            runtime
                .block_on(store.sync_provider(&remote, ProviderKind::Miniflux))
                .unwrap(),
            1
        );

        let snapshot = store.snapshot(Scope::All, "", 20, None, true).unwrap();
        assert!(snapshot.articles[0].is_read);
        assert!(snapshot.articles[0].is_starred);
        let requests = server.join().unwrap();
        assert!(
            requests[5]
                .lines()
                .next()
                .unwrap()
                .contains("changed_after")
        );
        assert!(
            requests[8]
                .lines()
                .next()
                .unwrap()
                .contains("changed_after")
        );
        let synced_state: (String, Option<String>, Option<String>) = store
            .connection
            .query_row(
                "SELECT provider,sync_cursor,last_full_sync_at FROM remote_state WHERE workspace='provider:miniflux'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(synced_state.0, "miniflux");
        assert!(synced_state.1.is_some());
        assert!(synced_state.2.is_some());
    }

    #[test]
    fn sanitized_article_keeps_reader_structures_and_drops_active_content() {
        let source = include_str!("../tests/fixtures/article_sample.html");
        let safe = sanitize_html(source, Some("https://example.com/posts/one"));
        assert!(safe.contains("<table"));
        assert!(safe.contains("<code>离线</code>"));
        assert!(safe.contains("https://example.com/guide"));
        assert!(safe.contains("https://example.com/images/cover.png"));
        assert!(!safe.contains("<script"));
        assert!(!safe.contains("<iframe"));
        assert!(safe.contains("Embedded media is unavailable"));
    }

    #[test]
    fn article_search_and_smart_views_follow_saved_state() {
        let (_directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(feed_url,title) VALUES('https://example.com/feed.xml','示例订阅')",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO articles(feed_id,guid,title,snippet,content_html,is_starred,read_later) VALUES(1,'a','Rust 阅读器','GPUI 和文章搜索','<p>GPUI 和文章搜索</p>',1,0)",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO articles(feed_id,guid,title,snippet,is_read,read_later) VALUES(1,'b','稍后阅读','离线内容',1,1)",
                [],
            )
            .unwrap();
        let ids = store
            .connection
            .prepare("SELECT id FROM articles")
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for id in ids {
            sync_article_fts(&store.connection, id).unwrap();
        }

        let all = store.snapshot(Scope::All, "搜索", 500, None, true).unwrap();
        assert_eq!(all.articles.len(), 1);
        assert_eq!(all.articles[0].title, "Rust 阅读器");
        assert_eq!(
            store
                .snapshot(Scope::Unread, "", 500, None, true)
                .unwrap()
                .articles
                .len(),
            1
        );
        assert_eq!(
            store
                .snapshot(Scope::Starred, "", 500, None, true)
                .unwrap()
                .articles
                .len(),
            1
        );
        assert_eq!(
            store
                .snapshot(Scope::Later, "", 500, None, true)
                .unwrap()
                .articles
                .len(),
            1
        );

        store
            .mark(all.articles[0].id, MarkField::Read, true)
            .unwrap();
        assert_eq!(
            store
                .snapshot(Scope::Unread, "", 500, None, true)
                .unwrap()
                .articles
                .len(),
            0
        );
        assert!(store.article(all.articles[0].id).unwrap().summary.is_read);
    }

    #[test]
    fn article_pages_use_keyset_cursor() {
        let (_directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(feed_url,title) VALUES('https://example.com/feed.xml','Feed')",
                [],
            )
            .unwrap();
        for i in 1..=5 {
            store
                .connection
                .execute(
                    "INSERT INTO articles(feed_id,guid,title,snippet,published_at) VALUES(1,?1,?2,'',?3)",
                    params![
                        format!("g{i}"),
                        format!("Title {i}"),
                        format!("2026-01-0{i}T00:00:00Z")
                    ],
                )
                .unwrap();
        }
        let first = store.snapshot(Scope::All, "", 2, None, false).unwrap();
        assert_eq!(first.articles.len(), 2);
        assert!(first.has_more);
        assert_eq!(first.articles[0].title, "Title 5");
        assert_eq!(first.articles[1].title, "Title 4");
        let cursor = ArticleCursor::from_summary(&first.articles[1]);
        let second = store
            .snapshot(Scope::All, "", 2, Some(&cursor), false)
            .unwrap();
        assert_eq!(second.articles.len(), 2);
        assert!(second.has_more);
        assert_eq!(second.articles[0].title, "Title 3");
        assert_eq!(second.articles[1].title, "Title 2");
        let cursor = ArticleCursor::from_summary(&second.articles[1]);
        let third = store
            .snapshot(Scope::All, "", 2, Some(&cursor), false)
            .unwrap();
        assert_eq!(third.articles.len(), 1);
        assert!(!third.has_more);
        assert_eq!(third.articles[0].title, "Title 1");
    }

    #[test]
    fn opml_export_groups_subscriptions_by_folder() {
        let (_directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(feed_url,title,folder) VALUES('https://example.com/feed.xml','示例订阅','技术')",
                [],
            )
            .unwrap();
        let output = store.export_opml().unwrap();
        assert!(output.contains("示例订阅"));
        assert!(output.contains("技术"));
        assert!(output.contains("https://example.com/feed.xml"));
        assert!(OPML::from_str(&output).is_ok());
    }

    #[test]
    fn feed_fixture_resolves_article_links() {
        let feed =
            feed_rs::parser::parse(include_bytes!("../tests/fixtures/sample_feed.xml").as_slice())
                .unwrap();
        let item = map_entry(
            feed.entries.into_iter().next().unwrap(),
            "https://example.com/feed.xml",
            Some("https://example.com/"),
        )
        .unwrap();
        assert_eq!(item.title, "阅读器文章");
        assert_eq!(
            item.url.as_deref(),
            Some("https://example.com/articles/one")
        );
        assert!(item.content_html.contains("<strong>加粗</strong>"));
    }

    #[test]
    fn subscription_urls_only_allow_http_schemes() {
        assert_eq!(
            normalize_http_url("https://example.com/feed.xml").unwrap(),
            "https://example.com/feed.xml"
        );
        assert!(normalize_http_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn raw_feed_html_is_compressed_and_available_for_offline_processing() {
        let (_directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(feed_url,title) VALUES('https://example.com/feed.xml','Example')",
                [],
            )
            .unwrap();
        let original = "<article><p>Raw source kept for plugins</p></article>";
        store
            .connection
            .execute(
                "INSERT INTO articles(feed_id,guid,title,content_html,source_html) VALUES(1,'entry','Title','<p>sanitized</p>',?1)",
                [compress_html(original).unwrap()],
            )
            .unwrap();

        let article = store.article(1).unwrap();
        assert_eq!(article.source_html.as_deref(), Some(original));
        assert_eq!(article.content_html, "<p>sanitized</p>");
    }

    #[test]
    fn processed_content_cache_invalidates_on_source_url_title_or_pipeline_change() {
        let (_directory, store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(feed_url,title) VALUES('https://example.com/feed.xml','Example')",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO articles(feed_id,guid,title) VALUES(1,'entry','Title')",
                [],
            )
            .unwrap();
        assert!(
            store
                .processed_content(
                    1,
                    "<p>raw</p>",
                    "https://example.com/1",
                    "Title",
                    "plugins-v1"
                )
                .unwrap()
                .is_none()
        );
        store
            .save_processed_content(
                1,
                "<p>raw</p>",
                "https://example.com/1",
                "Title",
                "plugins-v1",
                "<p>cleaned</p>",
            )
            .unwrap();
        assert_eq!(
            store
                .processed_content(
                    1,
                    "<p>raw</p>",
                    "https://example.com/1",
                    "Title",
                    "plugins-v1"
                )
                .unwrap()
                .as_deref(),
            Some("<p>cleaned</p>")
        );
        assert!(
            store
                .processed_content(
                    1,
                    "<p>raw</p>",
                    "https://example.com/2",
                    "Title",
                    "plugins-v1"
                )
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .processed_content(
                    1,
                    "<p>raw</p>",
                    "https://example.com/1",
                    "Title",
                    "plugins-v2"
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn canonical_revision_is_stable_and_tracks_each_pipeline_input() {
        let revision = Store::canonical_revision(
            "<p>body</p>",
            "https://example.com/a",
            "Title",
            "pipeline-v1",
        );
        assert_eq!(
            revision,
            Store::canonical_revision(
                "<p>body</p>",
                "https://example.com/a",
                "Title",
                "pipeline-v1",
            )
        );
        assert_ne!(
            revision,
            Store::canonical_revision(
                "<p>changed</p>",
                "https://example.com/a",
                "Title",
                "pipeline-v1",
            )
        );
        assert_ne!(
            revision,
            Store::canonical_revision(
                "<p>body</p>",
                "https://example.com/b",
                "Title",
                "pipeline-v1",
            )
        );
        assert_ne!(
            revision,
            Store::canonical_revision(
                "<p>body</p>",
                "https://example.com/a",
                "Title",
                "pipeline-v2",
            )
        );
    }

    #[tokio::test]
    async fn cached_page_can_be_reprocessed_offline_and_skips_unchanged_pipeline() {
        let (_directory, mut store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(feed_url,title) VALUES('https://example.com/feed.xml','Example')",
                [],
            )
            .unwrap();
        let text = "A recovered article paragraph with enough detail for the standard full-text quality threshold. ".repeat(2);
        let raw = format!("<html><body><nav>Page chrome</nav><p>{text}</p></body></html>");
        let selected = format!("<p>{text}</p>");
        store
            .connection
            .execute(
                "INSERT INTO articles(feed_id,guid,title,url,content_html,source_page_html,extracted_html,extraction_pipeline_hash) VALUES(1,'entry','Title','https://127.0.0.1:1/story','<p>old</p>',?1,'<p>old extracted</p>','old-pipeline')",
                [compress_html(&raw).unwrap()],
            )
            .unwrap();

        store
            .extract_using(
                1,
                false,
                ContentExtractor::DomSmoothie,
                "new-pipeline",
                |source, _, _| {
                    assert_eq!(source, raw);
                    Ok(Some((selected.clone(), true)))
                },
            )
            .await
            .unwrap();
        let updated = store.article(1).unwrap();
        assert!(
            updated
                .extracted_html
                .as_deref()
                .unwrap()
                .contains("recovered article paragraph")
        );
        let canonical = updated.extracted_html.as_deref().unwrap();
        store
            .save_processed_content(
                1,
                canonical,
                "https://127.0.0.1:1/story",
                "Title",
                "canonical-html-v1:new-pipeline",
                canonical,
            )
            .unwrap();
        assert_eq!(
            store
                .snapshot(Scope::All, "recovered article paragraph", 20, None, false)
                .unwrap()
                .articles
                .len(),
            1,
            "the canonical body must refresh the FTS row"
        );

        store
            .extract_using(
                1,
                false,
                ContentExtractor::DomSmoothie,
                "new-pipeline",
                |_, _, _| panic!("unchanged pipeline should reuse the stored extraction"),
            )
            .await
            .unwrap();
    }

    #[test]
    fn extract_article_html_keeps_body_and_drops_chrome() {
        let raw = r#"<!DOCTYPE html><html><body>
<nav><a href="/">Home</a><a href="/about">About</a></nav>
<aside>Related ads and more sidebar links go here forever.</aside>
<article>
  <h1>Clean headline</h1>
  <p>This is the main article body with enough text so the readability
  algorithm can identify it as the primary content block of the page.</p>
  <p>A second paragraph continues the story with supporting detail that
  should survive extraction while navigation and sidebars are removed.</p>
</article>
<footer>Copyright site chrome</footer>
</body></html>"#;
        let html = extract_article_html(
            raw,
            "https://example.com/post",
            "Clean headline",
            ContentExtractor::DomSmoothie,
        );
        let text = plain_text(&html);
        assert!(text.contains("main article body"));
        assert!(text.contains("second paragraph"));
        assert!(!text.contains("Related ads"));
        assert!(!html.to_ascii_lowercase().contains("<nav"));
        assert!(!html.to_ascii_lowercase().contains("<footer"));
    }

    #[test]
    fn removing_subscription_cascades_its_cached_articles() {
        let (_directory, mut store) = test_store();
        store
            .connection
            .execute(
                "INSERT INTO feeds(feed_url,title) VALUES('https://example.com/feed.xml','示例订阅')",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO articles(feed_id,guid,title) VALUES(1,'entry','文章')",
                [],
            )
            .unwrap();

        store.remove_feed(1).unwrap();

        let feed_count: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM feeds", [], |row| row.get(0))
            .unwrap();
        let article_count: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM articles", [], |row| row.get(0))
            .unwrap();
        assert_eq!(feed_count, 0);
        assert_eq!(article_count, 0);
    }
}
