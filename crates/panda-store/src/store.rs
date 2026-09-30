use ammonia::{Builder as Sanitizer, UrlRelative};
use anyhow::Context as _;
use feed_rs::model::{Entry, Feed as ParsedFeed};
use opml::{Head, OPML, Outline};
use panda_core::{
    Article, ArticleCursor, ArticleSummary, ContentExtractor, Feed, MarkField, ParsedArticle,
    ReaderSnapshot, Scope,
};
use panda_miniflux::{Entry as RemoteEntry, Feed as RemoteFeed, Miniflux};
use panda_providers::{ProviderClient, ProviderKind};
use reqwest::Client;
use rusqlite::{Connection, OptionalExtension, params};
use scraper::{Html, Selector};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Duration;
use url::Url;

pub struct Store {
    connection: Connection,
    client: Client,
    workspace: String,
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Self::open_workspace(path, "local")
    }

    pub fn open_workspace(path: &Path, workspace: &str) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS feeds (
                id INTEGER PRIMARY KEY,
                feed_url TEXT NOT NULL,
                title TEXT NOT NULL,
                site_url TEXT,
                folder TEXT,
                etag TEXT,
                last_modified TEXT,
                last_error TEXT,
                added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
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
                extracted_html TEXT,
                is_read INTEGER NOT NULL DEFAULT 0,
                is_starred INTEGER NOT NULL DEFAULT 0,
                read_later INTEGER NOT NULL DEFAULT 0,
                UNIQUE(feed_id, guid)
            );
            CREATE INDEX IF NOT EXISTS idx_articles_feed_date ON articles(feed_id, published_at DESC);
            CREATE INDEX IF NOT EXISTS idx_articles_state_date ON articles(is_read, is_starred, read_later, published_at DESC);
            CREATE TABLE IF NOT EXISTS remote_state (workspace TEXT PRIMARY KEY, account TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS pending_remote_marks (
                workspace TEXT NOT NULL,
                remote_id INTEGER NOT NULL,
                field TEXT NOT NULL,
                value INTEGER NOT NULL,
                PRIMARY KEY(workspace,remote_id,field)
            );
            PRAGMA foreign_keys = ON;",
        )?;
        ensure_column(
            &connection,
            "feeds",
            "source",
            "TEXT NOT NULL DEFAULT 'local'",
        )?;
        ensure_column(&connection, "feeds", "remote_id", "INTEGER")?;
        ensure_column(&connection, "articles", "remote_id", "INTEGER")?;
        ensure_column(&connection, "articles", "translated_html", "TEXT")?;
        ensure_column(&connection, "articles", "translated_title", "TEXT")?;
        ensure_column(&connection, "articles", "translated_lang", "TEXT")?;
        ensure_column(&connection, "articles", "translation_source_hash", "TEXT")?;
        migrate_workspace_schema(&connection)?;
        connection.execute_batch(
            "DROP INDEX IF EXISTS idx_feeds_remote;
             DROP INDEX IF EXISTS idx_articles_remote;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_feeds_remote ON feeds(workspace,remote_id) WHERE remote_id IS NOT NULL;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_articles_remote ON articles(feed_id,remote_id) WHERE remote_id IS NOT NULL;",
        )?;
        let client = Client::builder()
            .user_agent("PandaReader/0.1 (+https://github.com)")
            .timeout(Duration::from_secs(25))
            .build()?;
        Ok(Self {
            connection,
            client,
            workspace: workspace.to_owned(),
        })
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
                "SELECT f.id, f.title, f.feed_url, f.site_url, f.folder,
                        COUNT(CASE WHEN a.is_read=0 THEN 1 END), f.last_error
                 FROM feeds f LEFT JOIN articles a ON a.feed_id=f.id
                 WHERE f.workspace=?1
                 GROUP BY f.id ORDER BY COALESCE(f.folder,''), f.title COLLATE NOCASE",
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
        let needle = format!("%{}%", search.trim());
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
        // Search title/author/snippet only -- never scan full HTML blobs.
        let sql = format!(
            "SELECT a.id,f.title,a.title,a.url,a.author,a.snippet,a.published_at,a.is_read,a.is_starred,a.read_later
             FROM articles a JOIN feeds f ON f.id=a.feed_id
                 WHERE f.workspace=? AND {filter} AND (?='' OR a.title LIKE ? OR IFNULL(a.author,'') LIKE ? OR a.snippet LIKE ?)
             {cursor_filter}
             ORDER BY COALESCE(a.published_at,'') DESC,a.id DESC LIMIT {limit}"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(ArticleSummary {
                id: row.get(0)?,
                feed_title: row.get(1)?,
                title: row.get(2)?,
                url: row.get(3)?,
                author: row.get(4)?,
                snippet: row.get(5)?,
                published_at: row.get(6)?,
                is_read: row.get(7)?,
                is_starred: row.get(8)?,
                read_later: row.get(9)?,
            })
        };
        let bind_search = |params: &mut Vec<rusqlite::types::Value>| {
            params.push(search.trim().to_string().into());
            params.push(needle.clone().into());
            params.push(needle.clone().into());
            params.push(needle.clone().into());
        };
        let mut params: Vec<rusqlite::types::Value> = Vec::new();
        params.push(self.workspace.clone().into());
        match &bind {
            ScopeBind::FeedId(feed_id) => params.push((*feed_id).into()),
            ScopeBind::Folder(folder) => params.push(folder.clone().into()),
            ScopeBind::None => {}
        }
        bind_search(&mut params);
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

    pub fn article(&self, id: i64) -> anyhow::Result<Article> {
        let summary = self.summary_by_id(id)?;
        let (
            url,
            content_html,
            extracted_html,
            translated_html,
            translated_title,
            translated_lang,
            translation_source_hash,
        ) = self.connection.query_row(
                "SELECT url,content_html,extracted_html,translated_html,translated_title,translated_lang,translation_source_hash FROM articles WHERE id=?1",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )?;
        Ok(Article {
            summary,
            url,
            content_html,
            extracted_html,
            translated_html,
            translated_title,
            translated_lang,
            translation_source_hash,
        })
    }

    pub async fn translate(
        &mut self,
        article_id: i64,
        target_lang: &str,
        translator: &panda_translate::AnyTranslator,
    ) -> anyhow::Result<Article> {
        let article = self.article(article_id)?;
        let source = article
            .extracted_html
            .as_deref()
            .filter(|html| !html.trim().is_empty())
            .unwrap_or(article.content_html.as_str());
        let title = article.summary.title.trim();
        let source_hash = panda_translate::source_hash(source, title);
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
            "UPDATE articles SET translated_html=?1, translated_title=?2, translated_lang=?3, translation_source_hash=?4 WHERE id=?5",
            params![result.html, result.title, target_lang, source_hash, article_id],
        )?;
        self.article(article_id)
    }

    fn summary_by_id(&self, id: i64) -> anyhow::Result<ArticleSummary> {
        Ok(self.connection.query_row(
            "SELECT a.id,f.title,a.title,a.url,a.author,a.snippet,a.published_at,a.is_read,a.is_starred,a.read_later
             FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.id=?1 AND f.workspace=?2",
            params![id, self.workspace],
            |row| Ok(ArticleSummary {
                id: row.get(0)?, feed_title: row.get(1)?, title: row.get(2)?,
                url: row.get(3)?,
                author: row.get(4)?, snippet: row.get(5)?, published_at: row.get(6)?,
                is_read: row.get(7)?, is_starred: row.get(8)?, read_later: row.get(9)?,
            }),
        )?)
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
                "INSERT INTO pending_remote_marks(workspace,remote_id,field,value) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(workspace,remote_id,field) DO UPDATE SET value=excluded.value",
                params![self.workspace, remote_id, column, value],
            )?;
        }
        Ok(())
    }

    pub async fn flush_provider_marks(&mut self, remote: &ProviderClient) -> anyhow::Result<usize> {
        let pending = {
            let mut statement = self.connection.prepare(
                "SELECT remote_id,field,value FROM pending_remote_marks WHERE workspace=?1 ORDER BY remote_id,field",
            )?;
            statement
                .query_map([&self.workspace], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut read_ids = Vec::new();
        let mut unread_ids = Vec::new();
        let mut starred = Vec::new();
        for (remote_id, field, value) in &pending {
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
        let completed = pending.len();
        self.connection.execute(
            "DELETE FROM pending_remote_marks WHERE workspace=?1",
            [&self.workspace],
        )?;
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
        let mut statement = self.connection.prepare(&sql)?;
        let rows: Vec<(i64, Option<i64>)> = match bind {
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
        };
        let count = rows.len();
        for (id, remote_id) in rows {
            self.connection
                .execute("UPDATE articles SET is_read=1 WHERE id=?1", [id])?;
            if let Some(remote_id) = remote_id {
                self.connection.execute(
                    "INSERT INTO pending_remote_marks(workspace,remote_id,field,value) VALUES(?1,?2,'is_read',1)
                     ON CONFLICT(workspace,remote_id,field) DO UPDATE SET value=excluded.value",
                    params![self.workspace, remote_id],
                )?;
            }
        }
        Ok(count)
    }

    pub fn update_feed(
        &self,
        id: i64,
        title: &str,
        folder: Option<&str>,
        feed_url: &str,
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
            "UPDATE feeds SET title=?1, folder=?2, feed_url=?3 WHERE id=?4 AND workspace=?5",
            params![title, folder, feed_url, id, self.workspace],
        )?;
        Ok(())
    }

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

    pub async fn sync_provider(
        &mut self,
        remote: &ProviderClient,
        kind: ProviderKind,
    ) -> anyhow::Result<usize> {
        let identity = remote.identity().await?;
        let feeds = remote.feeds().await?;
        let account = identity.account;
        let previous: Option<String> = self
            .connection
            .query_row(
                "SELECT account FROM remote_state WHERE workspace=?1",
                [&self.workspace],
                |row| row.get(0),
            )
            .optional()?;
        if previous.as_deref().is_some_and(|old| old != account) {
            self.connection.execute(
                "DELETE FROM feeds WHERE source=?1 AND workspace=?2",
                params![kind.key(), self.workspace],
            )?;
            self.connection.execute(
                "DELETE FROM pending_remote_marks WHERE workspace=?1",
                [&self.workspace],
            )?;
        }
        self.connection.execute(
            "INSERT INTO remote_state(workspace,account) VALUES(?1,?2)
             ON CONFLICT(workspace) DO UPDATE SET account=excluded.account",
            params![self.workspace, account],
        )?;
        self.flush_provider_marks(remote).await?;
        let feed_ids = self.save_remote_feeds(&feeds, kind)?;
        let entries = remote.all_entries().await?;
        self.save_remote_entries(&entries, &feed_ids)
    }

    pub async fn sync_miniflux(&mut self, remote: &Miniflux) -> anyhow::Result<usize> {
        self.sync_provider(
            &ProviderClient::Miniflux(remote.clone()),
            ProviderKind::Miniflux,
        )
        .await
    }

    fn save_remote_feeds(
        &mut self,
        feeds: &[RemoteFeed],
        kind: ProviderKind,
    ) -> anyhow::Result<HashMap<i64, i64>> {
        let mut seen = HashSet::new();
        for feed in feeds {
            seen.insert(feed.id);
            let folder = feed
                .category
                .as_ref()
                .map(|category| category.title.as_str());
            let updated = self.connection.execute(
                "UPDATE feeds SET feed_url=?1,title=?2,site_url=?3,folder=?4,source=?5
                 WHERE remote_id=?6 AND workspace=?7",
                params![
                    feed.feed_url,
                    feed.title,
                    feed.site_url,
                    folder,
                    kind.key(),
                    feed.id,
                    self.workspace
                ],
            )?;
            if updated == 0 {
                self.connection.execute(
                    "INSERT INTO feeds(feed_url,title,site_url,folder,source,remote_id,workspace)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(workspace,feed_url) DO UPDATE SET
                    title=excluded.title,site_url=excluded.site_url,folder=excluded.folder,
                    source=excluded.source,remote_id=excluded.remote_id",
                    params![
                        feed.feed_url,
                        feed.title,
                        feed.site_url,
                        folder,
                        kind.key(),
                        feed.id,
                        self.workspace
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

    fn save_remote_entries(
        &mut self,
        entries: &[RemoteEntry],
        feed_ids: &HashMap<i64, i64>,
    ) -> anyhow::Result<usize> {
        let transaction = self.connection.transaction()?;
        let mut saved = 0;
        for entry in entries {
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
            let html = sanitize_html(&entry.content, entry.url.as_deref());
            let snippet = plain_text(&html).chars().take(280).collect::<String>();
            transaction.execute(
                "INSERT INTO articles(feed_id,guid,title,url,author,published_at,snippet,content_html,is_read,is_starred,remote_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT(feed_id,guid) DO UPDATE SET
                    title=excluded.title,url=excluded.url,author=excluded.author,
                    published_at=excluded.published_at,snippet=excluded.snippet,
                    content_html=excluded.content_html,is_read=excluded.is_read,
                    is_starred=excluded.is_starred,remote_id=excluded.remote_id",
                params![
                    feed_id, format!("miniflux:{}", entry.id), entry.title, entry.url,
                    entry.author, entry.published_at, snippet, html,
                    entry.status == "read", entry.starred, entry.id
                ],
            )?;
            saved += 1;
        }
        transaction.commit()?;
        Ok(saved)
    }

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

    fn save_feed(
        &mut self,
        url: &str,
        feed: ParsedFeed,
        existing_id: Option<i64>,
        etag: Option<String>,
        modified: Option<String>,
    ) -> anyhow::Result<()> {
        let site_url = feed
            .links
            .iter()
            .find(|link| link.rel.as_deref() == Some("alternate"))
            .or_else(|| feed.links.first())
            .map(|link| resolve_url(url, &link.href));
        let title = feed
            .title
            .as_ref()
            .map(|t| t.content.trim())
            .filter(|t| !t.is_empty())
            .unwrap_or(url)
            .to_owned();
        let id = if let Some(id) = existing_id {
            self.connection.execute("UPDATE feeds SET title=?1,site_url=COALESCE(?2,site_url),etag=COALESCE(?3,etag),last_modified=COALESCE(?4,last_modified),last_error=NULL WHERE id=?5 AND workspace=?6", params![title,site_url,etag,modified,id,self.workspace])?;
            id
        } else {
            self.connection.execute("INSERT INTO feeds(feed_url,title,site_url,etag,last_modified,workspace) VALUES(?1,?2,?3,?4,?5,?6)", params![url,title,site_url,etag,modified,self.workspace])?;
            self.connection.last_insert_rowid()
        };
        let transaction = self.connection.transaction()?;
        for entry in feed.entries {
            let Some(parsed) = map_entry(entry, url, site_url.as_deref()) else {
                continue;
            };
            transaction.execute(
                "INSERT INTO articles(feed_id,guid,title,url,author,published_at,snippet,content_html)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(feed_id,guid) DO UPDATE SET title=excluded.title,url=excluded.url,author=excluded.author,
                 published_at=COALESCE(excluded.published_at,articles.published_at),snippet=excluded.snippet,
                 content_html=CASE WHEN excluded.content_html='' THEN articles.content_html ELSE excluded.content_html END",
                params![id,parsed.guid,parsed.title,parsed.url,parsed.author,parsed.published_at,parsed.snippet,parsed.content_html],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub async fn extract(
        &mut self,
        article_id: i64,
        force: bool,
        extractor: ContentExtractor,
    ) -> anyhow::Result<()> {
        let existing: Option<String> = self.connection.query_row(
            "SELECT extracted_html FROM articles WHERE id=?1",
            [article_id],
            |row| row.get(0),
        )?;
        if !force
            && existing
                .as_deref()
                .is_some_and(|html| !html.trim().is_empty())
        {
            return Ok(());
        }
        let url: String = self.connection.query_row(
            "SELECT url FROM articles WHERE id=?1",
            [article_id],
            |row| row.get(0),
        )?;
        let resolved = resolve_readable_url(&self.client, &url).await?;
        if resolved != url {
            self.connection.execute(
                "UPDATE articles SET url=?1 WHERE id=?2",
                params![resolved, article_id],
            )?;
        }
        let raw = fetch_html(&self.client, &resolved, Some("https://news.google.com/")).await?;
        let sanitized = extract_article_html(&raw, &resolved, extractor);
        if plain_text(&sanitized).trim().len() < 80 {
            anyhow::bail!("No extractable article content found on this page");
        }
        self.connection.execute(
            "UPDATE articles SET extracted_html=?1, translated_html=NULL, translated_title=NULL, translated_lang=NULL, translation_source_hash=NULL WHERE id=?2",
            params![sanitized, article_id],
        )?;
        Ok(())
    }

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
        let mut statement = self.connection.prepare("SELECT title,feed_url,folder FROM feeds WHERE workspace=?1 ORDER BY COALESCE(folder,''),title COLLATE NOCASE")?;
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
            "PRAGMA foreign_keys=OFF;
             BEGIN IMMEDIATE;
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
                workspace TEXT NOT NULL DEFAULT 'local',
                UNIQUE(workspace,feed_url)
             );",
        )?;
        let account_workspace = "provider:miniflux";
        connection.execute(
            "INSERT INTO feeds_workspace(id,feed_url,title,site_url,folder,etag,last_modified,last_error,added_at,source,remote_id,workspace)
             SELECT id,feed_url,title,site_url,folder,etag,last_modified,last_error,added_at,source,remote_id,
                    CASE WHEN source='miniflux' THEN ?1 ELSE 'local' END FROM feeds",
            [&account_workspace],
        )?;
        connection.execute_batch(
            "DROP TABLE feeds;
             ALTER TABLE feeds_workspace RENAME TO feeds;
             COMMIT;
             PRAGMA foreign_keys=ON;",
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
    })
}

fn sanitize_html(html: &str, base: Option<&str>) -> String {
    // Strip logos before ammonia -- it drops id/class we use as signals.
    let html = strip_boilerplate_images(html);
    let lower = html.to_ascii_lowercase();
    let has_embedded_media =
        lower.contains("<video") || lower.contains("<iframe") || lower.contains("<audio");
    let mut sanitizer = Sanitizer::default();
    if let Some(base) = base.and_then(|s| Url::parse(s).ok()) {
        sanitizer.url_relative(UrlRelative::RewriteWithBase(base));
    }
    let mut safe = sanitizer.clean(&html).to_string();
    // Second pass for src-only signals after URL rewrite.
    safe = strip_boilerplate_images(&safe);
    if has_embedded_media {
        safe.push_str("<p><em>Embedded media is unavailable here. Open the original article to view it.</em></p>");
    }
    safe
}

/// Drop site logos / WeChat share images that Readability often leaves behind.
/// Same idea as Miniflux/FreshRSS CSS rewrite rules -- cheap filename/id heuristics.
fn strip_boilerplate_images(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut index = 0usize;
    while index < html.len() {
        let rest_lower = &lower[index..];
        if rest_lower.starts_with("<img") {
            if let Some(rel) = rest_lower.find('>') {
                let end = index + rel + 1;
                let tag_lower = &lower[index..end];
                if is_boilerplate_img_tag(tag_lower) {
                    index = end;
                    continue;
                }
                out.push_str(&html[index..end]);
                index = end;
                continue;
            }
        }
        let ch = html[index..].chars().next().unwrap();
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn is_boilerplate_img_tag(tag_lower: &str) -> bool {
    if tag_lower.contains("wx_img") {
        return true;
    }
    if attr_contains_any(tag_lower, "id", &["logo", "qrcode", "qr-code"]) {
        return true;
    }
    if attr_contains_any(tag_lower, "class", &["logo", "qrcode", "qr-code"]) {
        return true;
    }
    if attr_contains_any(tag_lower, "src", &["logo", "qrcode", "qr-code"]) {
        return true;
    }
    false
}

fn attr_contains_any(tag_lower: &str, name: &str, needles: &[&str]) -> bool {
    let Some(value) = attr_value(tag_lower, name) else {
        return false;
    };
    needles.iter().any(|needle| value.contains(needle))
}

fn attr_value<'a>(tag_lower: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=");
    let start = tag_lower.find(&key)? + key.len();
    let bytes = tag_lower.as_bytes();
    let quote = *bytes.get(start)?;
    if quote == b'"' || quote == b'\'' {
        let close = tag_lower[start + 1..].find(quote as char)? + start + 1;
        Some(&tag_lower[start + 1..close])
    } else {
        let end = tag_lower[start..]
            .find(|ch: char| ch.is_whitespace() || ch == '>')
            .map(|rel| start + rel)
            .unwrap_or(tag_lower.len());
        Some(&tag_lower[start..end])
    }
}

fn plain_text(html: &str) -> String {
    Html::parse_fragment(html)
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_http_url(raw: &str) -> anyhow::Result<String> {
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
const GOOGLE_CONSENT_COOKIE: &str = "SOCS=CAESEwgDEgk0ODE3Nzk3MjQaAmVuIAEaBgiA_LyaBg";

fn google_news_article_id(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if parsed.host_str() != Some("news.google.com") {
        return None;
    }
    let mut segments = parsed.path_segments()?.peekable();
    while let Some(segment) = segments.next() {
        if segment == "articles" {
            let id = segments.next()?;
            if !id.is_empty() {
                return Some(id.to_owned());
            }
        }
    }
    None
}

fn html_attr_value<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let start = html.find(&needle)? + needle.len();
    let end = html[start..].find('"')? + start;
    Some(&html[start..end])
}

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
    if google_news_article_id(url).is_some() {
        request = request.header(reqwest::header::COOKIE, GOOGLE_CONSENT_COOKIE);
    }
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
    Ok(response.error_for_status()?.text().await?)
}

async fn resolve_readable_url(client: &Client, url: &str) -> anyhow::Result<String> {
    let Some(article_id) = google_news_article_id(url) else {
        return Ok(url.to_owned());
    };
    let page = fetch_html(client, url, None).await?;
    let signature = html_attr_value(&page, "data-n-a-sg")
        .ok_or_else(|| anyhow::anyhow!("Google News page is missing a decode signature"))?;
    let timestamp = html_attr_value(&page, "data-n-a-ts")
        .ok_or_else(|| anyhow::anyhow!("Google News page is missing a decode timestamp"))?
        .parse::<i64>()
        .context("Invalid Google News decode timestamp")?;
    let shell = serde_json::json!([
        [
            "X",
            "X",
            ["X", "X"],
            null,
            null,
            1,
            1,
            "US:en",
            null,
            1,
            null,
            null,
            null,
            null,
            null,
            0,
            1
        ],
        "X",
        "X",
        1,
        [1, 1, 1],
        1,
        1,
        null,
        0,
        0,
        null,
        0
    ]);
    let inner = serde_json::json!(["garturlreq", shell, article_id, timestamp, signature]);
    let f_req = serde_json::json!([[["Fbv4je", inner.to_string(), null, "generic"]]]);
    let response = client
        .post("https://news.google.com/_/DotsSplashUi/data/batchexecute")
        .header(reqwest::header::USER_AGENT, BROWSER_UA)
        .header(reqwest::header::COOKIE, GOOGLE_CONSENT_COOKIE)
        .header(reqwest::header::REFERER, "https://news.google.com/")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .form(&[("f.req", f_req.to_string())])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_google_news_batch_url(&response)
        .ok_or_else(|| anyhow::anyhow!("Could not decode Google News article URL"))
}

fn parse_google_news_batch_url(body: &str) -> Option<String> {
    let trimmed = body.trim_start();
    let trimmed = trimmed
        .strip_prefix(")]}'")
        .map(str::trim_start)
        .unwrap_or(trimmed);
    let json_start = trimmed.find('[')?;
    let envelopes: Vec<serde_json::Value> = serde_json::from_str(&trimmed[json_start..]).ok()?;
    for envelope in envelopes {
        let Some(rows) = envelope.as_array() else {
            continue;
        };
        if rows.first().and_then(|value| value.as_str()) != Some("wrb.fr") {
            continue;
        }
        if rows.get(1).and_then(|value| value.as_str()) != Some("Fbv4je") {
            continue;
        }
        let payload = rows.get(2)?.as_str()?;
        let decoded: serde_json::Value = serde_json::from_str(payload).ok()?;
        let rows = decoded.as_array()?;
        if rows.first().and_then(|value| value.as_str()) != Some("garturlres") {
            continue;
        }
        let url = rows.get(1)?.as_str()?;
        if url.starts_with("http://") || url.starts_with("https://") {
            return Some(url.to_owned());
        }
    }
    None
}

fn extract_article_html(raw: &str, base: &str, extractor: ContentExtractor) -> String {
    match extractor {
        ContentExtractor::DomSmoothie => {
            if let Some(content) = readability_article_html(raw, base) {
                return sanitize_html(&content, Some(base));
            }
            heuristic_article_html(raw, base)
        }
        ContentExtractor::Decruft => {
            if let Some(content) = decruft_article_html(raw, base) {
                return sanitize_html(&content, Some(base));
            }
            heuristic_article_html(raw, base)
        }
        ContentExtractor::Trafilatura => {
            if let Some(content) = trafilatura_article_html(raw, base) {
                return sanitize_html(&content, Some(base));
            }
            heuristic_article_html(raw, base)
        }
        ContentExtractor::Heuristic => heuristic_article_html(raw, base),
    }
}

fn readability_article_html(raw: &str, base: &str) -> Option<String> {
    let mut reader = dom_smoothie::Readability::new(raw, Some(base), None).ok()?;
    let article = reader.parse().ok()?;
    let content = article.content.trim();
    if plain_text(content).trim().len() < 80 {
        return None;
    }
    Some(content.to_owned())
}

fn decruft_article_html(raw: &str, base: &str) -> Option<String> {
    let mut options = decruft::DecruftOptions::default();
    options.url = Some(base.to_owned());
    options.markdown = false;
    options.allow_network = false;
    options.remove_small_images = true;
    let result = decruft::parse(raw, &options);
    let content = result.content.trim();
    if content.is_empty() || plain_text(content).trim().len() < 80 {
        return None;
    }
    Some(content.to_owned())
}

fn trafilatura_article_html(raw: &str, base: &str) -> Option<String> {
    let options = rs_trafilatura::Options {
        url: Some(base.to_owned()),
        include_images: true,
        include_tables: true,
        include_links: true,
        include_formatting: true,
        favor_recall: true,
        ..rs_trafilatura::Options::default()
    };
    let result = rs_trafilatura::extract_with_options(raw, &options).ok()?;
    let content = result
        .content_html
        .filter(|html| !html.trim().is_empty())
        .unwrap_or(result.content_text);
    let content = content.trim();
    if plain_text(content).trim().len() < 80 {
        return None;
    }
    Some(content.to_owned())
}

fn heuristic_article_html(raw: &str, base: &str) -> String {
    let document = Html::parse_document(raw);
    let selectors = [
        "article",
        "main",
        "[role='main']",
        ".article-content",
        ".article_content",
        "#article-content",
        ".post-content",
        ".entry-content",
        ".post-body",
        ".article-body",
        ".story-body",
        ".TRS_Editor",
        "#zoom",
        ".pages_content",
        ".Custom_UnionStyle",
        "#mainContent",
        ".main-content",
        ".content",
        "#content",
    ];
    let mut best: Option<(usize, String)> = None;
    for selector in selectors {
        let Ok(parsed) = Selector::parse(selector) else {
            continue;
        };
        for element in document.select(&parsed) {
            let html = element.inner_html();
            let score = plain_text(&html).trim().len();
            if score < 80 {
                continue;
            }
            // Prefer denser article-like nodes over huge page shells.
            let tag = element.value().name();
            let bonus = match tag {
                "article" => 400,
                "main" => 200,
                _ => 0,
            };
            let score = score + bonus;
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score > *best_score)
            {
                best = Some((score, html));
            }
        }
    }
    if best.is_none() {
        if let Ok(parsed) = Selector::parse("p") {
            let paragraphs = document
                .select(&parsed)
                .map(|element| element.html())
                .filter(|html| plain_text(html).trim().len() >= 40)
                .take(24)
                .collect::<Vec<_>>();
            if !paragraphs.is_empty() {
                let combined = paragraphs.join("\n");
                let score = plain_text(&combined).trim().len();
                if score >= 80 {
                    best = Some((score, combined));
                }
            }
        }
    }
    let html = best
        .map(|(_, html)| html)
        .unwrap_or_else(|| document.root_element().inner_html());
    sanitize_html(&html, Some(base))
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

    fn test_store() -> (tempfile::TempDir, Store) {
        test_store_for("local")
    }

    fn test_store_for(workspace: &str) -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().expect("temporary database directory");
        let store = Store::open_workspace(&directory.path().join("reader.sqlite3"), workspace)
            .expect("open test database");
        (directory, store)
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
        let provider = Store::open_workspace(&path, "provider:miniflux").unwrap();

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

        let local = Store::open_workspace(&path, "local").unwrap();
        let provider = Store::open_workspace(&path, "provider:miniflux").unwrap();
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
            category: Some(panda_miniflux::Category {
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
        };
        store.save_remote_entries(&[entry.clone()], &ids).unwrap();
        let snapshot = store.snapshot(Scope::All, "", 500, None, true).unwrap();
        assert_eq!(snapshot.feeds[0].folder.as_deref(), Some("技术"));
        let article_id = snapshot.articles[0].id;
        assert_eq!(store.remote_entry_id(article_id).unwrap(), Some(99));
        assert!(
            !store
                .article(article_id)
                .unwrap()
                .content_html
                .contains("<script")
        );
        store.mark(article_id, MarkField::Later, true).unwrap();

        let mut changed = entry;
        changed.status = "read".into();
        changed.starred = true;
        store.save_remote_entries(&[changed], &ids).unwrap();
        let article = store.article(article_id).unwrap();
        assert!(article.summary.is_read);
        assert!(article.summary.is_starred);
        assert!(article.summary.read_later);
        store.mark(article_id, MarkField::Starred, false).unwrap();
        store.mark(article_id, MarkField::Starred, true).unwrap();
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
    fn miniflux_sync_downloads_feeds_and_articles() {
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
        let config =
            panda_miniflux::Connection::new(&format!("http://{address}"), "secret").unwrap();
        let remote = Miniflux::new(config).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(runtime.block_on(store.sync_miniflux(&remote)).unwrap(), 1);
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
    fn strips_site_logo_and_wechat_share_images() {
        let source = r#"<p>Body</p>
<img id="wx_img" src="https://www.qbitai.com/wp-content/uploads/imgs/qbitai-logo-1.png" width="400" height="400">
<img src="https://cdn.example.com/photos/article-hero.jpg" alt="hero">
<img class="site-logo" src="https://cdn.example.com/brand.png">"#;
        let safe = sanitize_html(source, Some("https://www.qbitai.com/post"));
        assert!(safe.contains("article-hero.jpg"));
        assert!(!safe.contains("wx_img"));
        assert!(!safe.contains("qbitai-logo"));
        assert!(!safe.contains("site-logo"));
        assert!(!safe.contains("brand.png"));
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
                "INSERT INTO articles(feed_id,guid,title,snippet,is_starred,read_later) VALUES(1,'a','Rust 阅读器','GPUI 和文章搜索',1,0)",
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
    fn google_news_wrapper_ids_are_detected_from_article_urls() {
        assert_eq!(
            google_news_article_id("https://news.google.com/rss/articles/CBMiEXAMPLE?oc=5")
                .as_deref(),
            Some("CBMiEXAMPLE")
        );
        assert_eq!(
            google_news_article_id("https://news.google.com/articles/CBMiEXAMPLE").as_deref(),
            Some("CBMiEXAMPLE")
        );
        assert_eq!(
            google_news_article_id("https://www.theguardian.com/world/article"),
            None
        );
    }

    #[test]
    fn google_news_batch_response_yields_publisher_url() {
        let body = r#")]}'

26
[["wrb.fr","Fbv4je","[\"garturlres\",\"https://www.example.com/story\",1]",null,null,null,"generic"]]
"#;
        assert_eq!(
            parse_google_news_batch_url(body).as_deref(),
            Some("https://www.example.com/story")
        );
        assert_eq!(
            html_attr_value(
                r#"<div data-n-a-sg="sig123" data-n-a-ts="42"></div>"#,
                "data-n-a-sg"
            ),
            Some("sig123")
        );
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
