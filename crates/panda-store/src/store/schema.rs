use super::*;

pub(super) fn ensure_column(
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

pub(super) fn migrate_workspace_schema(connection: &Connection) -> anyhow::Result<()> {
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

impl Store {
    pub fn migrate(path: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            rusqlite::version_number() >= 3_051_003,
            "SQLite 3.51.3 or newer is required; linked version is {}",
            rusqlite::version()
        );
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
        if schema_version < CURRENT_SCHEMA_VERSION && has_existing_schema {
            let timestamp = chrono::Utc::now().timestamp_millis();
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            let backup_path =
                path.with_file_name(format!("{file_name}.pre-migration-{timestamp}.bak"));
            connection.backup("main", &backup_path, None)?;
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
                source_page_url TEXT,
                source_page_content_type TEXT,
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
            ensure_column(&connection, "articles", "source_page_url", "TEXT")?;
            ensure_column(&connection, "articles", "source_page_content_type", "TEXT")?;
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
        // Token usage columns were added after schema version 4 without a
        // dedicated migration version. Repair databases that already report
        // version 4 or 5, and keep the operation safe to repeat.
        if schema_version < 6 {
            connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS translation_usage(
                    day TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    requests INTEGER NOT NULL DEFAULT 0,
                    characters INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY(day,provider)
                );",
            )?;
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
            connection.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
        }
        if schema_version < 7 {
            // Cached article pages retain their final URL and content type so
            // redirects and non-HTML responses can be reprocessed correctly.
            ensure_column(&connection, "articles", "source_page_url", "TEXT")?;
            ensure_column(&connection, "articles", "source_page_content_type", "TEXT")?;
            connection.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
        }
        if schema_version < CURRENT_SCHEMA_VERSION {
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
}
