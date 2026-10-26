use super::*;
use panda_core::Scope;
use std::time::Instant;

#[test]
fn migration_is_versioned_and_read_connections_do_not_change_schema() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("migration.sqlite3");
    Store::migrate(&path).unwrap();
    let connection = Connection::open(&path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
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
    let backup_path = std::fs::read_dir(directory.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .contains("pre-migration-")
        })
        .expect("migration should create a pre-migration backup");
    let backup =
        Connection::open_with_flags(backup_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let backup_version: i64 = backup
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    let backup_last_id: i64 = backup
        .query_row(
            "SELECT last_article_id FROM article_fts_state WHERE id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(backup_version, 4);
    assert_eq!(backup_last_id, 99);
    assert_eq!(
        backup
            .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
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

#[test]
fn migration_repairs_translation_usage_columns_on_existing_version_five_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("translation-usage-v5.sqlite3");
    Store::migrate(&path).unwrap();

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO translation_usage(day,provider,requests,characters)
             VALUES('2026-10-11','llm',2,120)",
            [],
        )
        .unwrap();
    connection
        .execute_batch(
            "BEGIN;
             ALTER TABLE translation_usage RENAME TO translation_usage_with_tokens;
             CREATE TABLE translation_usage(
                day TEXT NOT NULL,
                provider TEXT NOT NULL,
                requests INTEGER NOT NULL DEFAULT 0,
                characters INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(day,provider)
             );
             INSERT INTO translation_usage(day,provider,requests,characters)
                SELECT day,provider,requests,characters FROM translation_usage_with_tokens;
             DROP TABLE translation_usage_with_tokens;
             PRAGMA user_version=5;
             COMMIT;",
        )
        .unwrap();
    drop(connection);

    Store::migrate(&path).unwrap();
    let store = Store::open(&path).unwrap();
    store
        .record_translation_usage_with_tokens("2026-10-11", "llm", 1, 30, 17, 9)
        .unwrap();
    let usage = store.translation_usage().unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(
        (
            usage[0].requests,
            usage[0].characters,
            usage[0].input_tokens,
            usage[0].output_tokens
        ),
        (3, 150, 17, 9)
    );
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
        plan("EXPLAIN QUERY PLAN SELECT rowid FROM article_fts WHERE article_fts MATCH 'article'")
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
    let bytes = include_bytes!("../../tests/fixtures/sample_feed.xml");
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
    let prepared =
        Store::prepare_remote_entries_with_revisions(vec![entry], ProviderKind::Miniflux, &known)
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
    let mut store = Store::open_writer(&directory.path().join("reader.sqlite3"), "local").unwrap();
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
    store
        .connection
        .execute(
            "INSERT INTO articles(id,feed_id,guid,title,remote_id) VALUES(1,1,'1','Article',42)",
            [],
        )
        .unwrap();
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
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
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
        assert!(stream.read(&mut request).unwrap() > 0);
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
            assert!(stream.read(&mut request).unwrap() > 0);
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
    let source = include_str!("../../tests/fixtures/article_sample.html");
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
        feed_rs::parser::parse(include_bytes!("../../tests/fixtures/sample_feed.xml").as_slice())
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
fn extraction_uses_and_persists_final_page_url_and_content_type() {
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
            "INSERT INTO articles(feed_id,guid,title,url,content_html) VALUES(1,'entry','Title','https://news.example/redirect','<p>feed</p>')",
            [],
        )
        .unwrap();
    let input = store.extraction_input(1, true).unwrap().unwrap();
    let source_url = "https://publisher.example/final/story";
    let body = "A full article body with enough readable content to pass the normal extraction threshold. ".repeat(2);
    let html = format!("<html><body><p>{body}</p></body></html>");
    let prepared = store
        .prepare_extraction_from_page(
            input,
            "https://news.example/resolved".into(),
            source_url.into(),
            "text/html".into(),
            html,
            ContentExtractor::DomSmoothie,
            "pipeline-v1",
            |_, url, _| {
                assert_eq!(url, source_url);
                Ok(None)
            },
        )
        .unwrap()
        .unwrap();

    assert_eq!(prepared.resolved_url, "https://news.example/resolved");
    assert_eq!(prepared.source_page_url, source_url);
    assert_eq!(prepared.source_page_content_type, "text/html");
    assert!(store.persist_extraction(prepared).unwrap());
    let input = store.extraction_input(1, false).unwrap().unwrap();
    assert_eq!(input.cached_page_url.as_deref(), Some(source_url));
    assert_eq!(input.cached_page_content_type.as_deref(), Some("text/html"));
}

#[tokio::test]
async fn stale_extraction_is_discarded_after_article_source_changes() {
    let (_directory, store) = test_store();
    store
        .connection
        .execute(
            "INSERT INTO feeds(feed_url,title) VALUES('https://example.com/feed.xml','Example')",
            [],
        )
        .unwrap();
    let raw = format!(
        "<html><body><p>{}</p></body></html>",
        "The original article body has enough detail to pass extraction quality checks. ".repeat(2)
    );
    store
        .connection
        .execute(
            "INSERT INTO articles(feed_id,guid,title,url,content_html,source_page_html,extracted_html,extraction_pipeline_hash,translated_html,translated_lang,translation_source_hash) VALUES(1,'entry','Title','https://example.com/story','<p>feed body</p>',?1,'<p>previous extracted body</p>','old-pipeline','<p>keep translation</p>','zh','cached-hash')",
            [compress_html(&raw).unwrap()],
        )
        .unwrap();

    let prepared = store
        .prepare_extraction_using(
            1,
            false,
            ContentExtractor::DomSmoothie,
            "new-pipeline",
            |url| Ok(url.to_owned()),
            |_, _, _| {
                Ok(Some((
                    format!(
                        "<p>{}</p>",
                        "The extracted copy belongs to the older article revision. ".repeat(2)
                    ),
                    true,
                )))
            },
        )
        .await
        .unwrap()
        .expect("changed pipeline should prepare extraction");

    store
        .connection
        .execute(
            "UPDATE articles SET content_html='<p>new feed revision</p>',content_revision=content_revision+1 WHERE id=1",
            [],
        )
        .unwrap();

    assert!(!store.persist_extraction(prepared).unwrap());
    assert_eq!(
        store.article(1).unwrap().extracted_html.as_deref(),
        Some("<p>previous extracted body</p>")
    );
    assert_eq!(
        store.article(1).unwrap().translated_html.as_deref(),
        Some("<p>keep translation</p>")
    );
}

#[test]
fn mark_does_not_mutate_an_article_owned_by_another_workspace() {
    let directory = tempfile::tempdir().expect("temporary database directory");
    let path = directory.path().join("reader.sqlite3");
    let local = Store::open_writer(&path, "local").unwrap();
    local
        .connection
        .execute(
            "INSERT INTO feeds(id,feed_url,title,workspace) VALUES(1,'https://example.com/feed.xml','Example','local')",
            [],
        )
        .unwrap();
    local
        .connection
        .execute(
            "INSERT INTO articles(id,feed_id,guid,title) VALUES(1,1,'entry','Article')",
            [],
        )
        .unwrap();
    let provider = Store::open_writer(&path, "provider:miniflux").unwrap();

    assert!(!provider.mark(1, MarkField::Read, true).unwrap());
    assert!(!local.article(1).unwrap().summary.is_read);
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

#[test]
fn remote_content_revisions_for_ids_returns_only_requested_hashes() {
    let (_directory, mut store) = test_store_for("provider:miniflux");
    let ids = store
        .save_remote_feeds(
            &[RemoteFeed {
                id: 7,
                title: "Feed".into(),
                feed_url: "https://example.com/feed.xml".into(),
                site_url: "https://example.com".into(),
                language: None,
                category: None,
            }],
            ProviderKind::Miniflux,
        )
        .unwrap();
    let feed_id = *ids.get(&7).unwrap();
    let transaction = store.connection.transaction().unwrap();
    for remote_id in 1..=20 {
        let hash = format!("hash-{remote_id}");
        transaction
            .execute(
                "INSERT INTO articles(feed_id,guid,title,remote_id,remote_content_hash,source_revision)
                 VALUES(?1,?2,?3,?4,?5,?5)",
                params![
                    feed_id,
                    format!("miniflux:{remote_id}"),
                    format!("Article {remote_id}"),
                    remote_id,
                    hash
                ],
            )
            .unwrap();
    }
    transaction.commit().unwrap();

    let revisions = store
        .remote_content_revisions_for_ids(&[3, 11, 999])
        .unwrap();
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions.get(&3).map(String::as_str), Some("hash-3"));
    assert_eq!(revisions.get(&11).map(String::as_str), Some("hash-11"));
    assert!(!revisions.contains_key(&999));
    assert_eq!(
        store.remote_content_revisions_for_ids(&[]).unwrap().len(),
        0
    );
}

#[test]
fn save_prepared_remote_entries_reports_changed_article_ids() {
    let (_directory, mut store) = test_store_for("provider:miniflux");
    let ids = store
        .save_remote_feeds(
            &[RemoteFeed {
                id: 42,
                title: "远程订阅".into(),
                feed_url: "https://example.com/feed.xml".into(),
                site_url: "https://example.com".into(),
                language: None,
                category: None,
            }],
            ProviderKind::Miniflux,
        )
        .unwrap();
    let entry = RemoteEntry {
        id: 99,
        feed_id: 42,
        title: "远程文章".into(),
        url: Some("https://example.com/post".into()),
        author: None,
        published_at: Some("2026-09-28T00:00:00Z".into()),
        content: "<p>正文</p>".into(),
        status: "unread".into(),
        starred: false,
        changed_at: None,
        revision: None,
    };
    let inserted = store
        .save_remote_entries(&[entry.clone()], &ids, ProviderKind::Miniflux)
        .unwrap();
    assert_eq!(inserted.count, 1);
    assert_eq!(inserted.changed_article_ids.len(), 1);
    let article_id = inserted.changed_article_ids[0];

    let noop = store
        .save_remote_entries(&[entry.clone()], &ids, ProviderKind::Miniflux)
        .unwrap();
    assert_eq!(noop.count, 0);
    assert!(noop.changed_article_ids.is_empty());

    let mut content_changed = entry;
    content_changed.content = "<p>更新后的正文</p>".into();
    let updated = store
        .save_remote_entries(&[content_changed.clone()], &ids, ProviderKind::Miniflux)
        .unwrap();
    assert_eq!(updated.count, 1);
    assert_eq!(updated.changed_article_ids, vec![article_id]);

    let mut mark_only = content_changed;
    mark_only.status = "read".into();
    let read_status = store
        .save_remote_entries(&[mark_only.clone()], &ids, ProviderKind::Miniflux)
        .unwrap();
    assert_eq!(read_status.count, 1);
    assert_eq!(read_status.changed_article_ids, vec![article_id]);
    assert!(store.article(article_id).unwrap().summary.is_read);

    mark_only.starred = true;
    let starred_status = store
        .save_remote_entries(&[mark_only], &ids, ProviderKind::Miniflux)
        .unwrap();
    assert_eq!(starred_status.count, 1);
    assert_eq!(starred_status.changed_article_ids, vec![article_id]);
    assert!(store.article(article_id).unwrap().summary.is_starred);
}

#[test]
#[ignore = "scale timing; run with --ignored --nocapture"]
fn revisions_scale_compares_full_scan_vs_page_ids() {
    let sizes = [10_000usize, 50_000, 100_000];
    let page_size = 500usize;
    for article_count in sizes {
        let (_directory, mut store) = test_store_for("provider:miniflux");
        let ids = store
            .save_remote_feeds(
                &[RemoteFeed {
                    id: 1,
                    title: "Scale Feed".into(),
                    feed_url: "https://example.com/scale.xml".into(),
                    site_url: "https://example.com".into(),
                    language: None,
                    category: None,
                }],
                ProviderKind::Miniflux,
            )
            .unwrap();
        let feed_id = *ids.get(&1).unwrap();
        let seed_started = Instant::now();
        let transaction = store.connection.transaction().unwrap();
        for remote_id in 1..=article_count as i64 {
            let hash = format!("hash-{remote_id}");
            transaction
                .execute(
                    "INSERT INTO articles(feed_id,guid,title,remote_id,remote_content_hash,source_revision)
                     VALUES(?1,?2,?3,?4,?5,?5)",
                    params![
                        feed_id,
                        format!("miniflux:{remote_id}"),
                        format!("Article {remote_id}"),
                        remote_id,
                        hash
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        eprintln!(
            "seeded {article_count} articles in {}ms",
            seed_started.elapsed().as_millis()
        );

        let full_started = Instant::now();
        let full = store.remote_content_revisions().unwrap();
        let full_ms = full_started.elapsed().as_millis();
        assert_eq!(full.len(), article_count);

        let page_ids: Vec<i64> = (1..=page_size as i64).collect();
        let page_started = Instant::now();
        let page = store.remote_content_revisions_for_ids(&page_ids).unwrap();
        let page_ms = page_started.elapsed().as_millis();
        assert_eq!(page.len(), page_size);

        eprintln!(
            "articles={article_count} full_revisions_ms={full_ms} page_{page_size}_revisions_ms={page_ms}"
        );
        assert!(
            page_ms <= full_ms,
            "page lookup should not be slower than full scan for {article_count} articles"
        );
    }
}
