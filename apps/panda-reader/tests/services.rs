// Services and their existing content tests run independently of GPUI rendering.
#![allow(dead_code)]

#[path = "../src/request_epoch.rs"]
mod request_epoch;
#[path = "../src/services/mod.rs"]
mod services;

use panda_core::Scope;
use services::{AppServices, Command};
use tokio::sync::oneshot;

#[test]
fn plugin_changes_that_leave_body_unchanged_keep_translation() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("library.sqlite3");
    panda_store::Store::migrate(&database).unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute(
            "INSERT INTO feeds(id,feed_url,title) VALUES(1,'https://example.com/feed','feed')",
            [],
        )
        .unwrap();
    connection.execute("INSERT INTO articles(id,feed_id,guid,title,url,content_html) VALUES(1,1,'one','Title','https://example.com/article','<p>body</p>')", []).unwrap();
    let plugin = dir.path().join("plugins/example");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("manifest.toml"), "id='example.cleanup'\nname='Example'\nversion='1.0.0'\napi_version=1\nkind='rules'\ndomains=['example.com']\nstage='cleanup'\ncapabilities=['document_read','document_write']\n").unwrap();
    std::fs::write(
        plugin.join("rules.toml"),
        "[[rules]]\nop='remove'\nselector='.not-present'\n",
    )
    .unwrap();
    let services = AppServices::start(
        database,
        dir.path().join("providers.json"),
        dir.path().join("translator.json"),
        dir.path().join("plugins"),
        "local",
        Default::default(),
        false,
        30,
        services::ArticleFetchNetwork::default(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let read = || {
        let (reply, response) = oneshot::channel();
        services.send(Command::Article {
            id: 1,
            show_translation: true,
            translation_layout: panda_core::TranslationLayout::Immersive,
            hide_images: false,
            paragraph_indent: false,
            reply,
        });
        runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), response)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        })
    };
    let first = read();
    let revision = first.article.canonical.unwrap().revision;
    let hash =
        panda_translate::translation_revision_hash(revision.0.as_ref(), "test-backend", "zh");
    connection.execute("UPDATE articles SET translated_html='<p>translation</p>', translated_lang='zh', translation_source_hash=?1, content_revision=content_revision+1 WHERE id=1", [hash]).unwrap();
    let (reply, response) = oneshot::channel();
    services.send(Command::SetPluginEnabled {
        id: "example.cleanup".into(),
        enabled: false,
        reply,
    });
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), response)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    let updated = read();
    assert_eq!(updated.article.canonical.unwrap().revision, revision);
    assert_eq!(
        updated.article.translated_html.as_deref(),
        Some("<p>translation</p>")
    );
}

#[test]
fn startup_database_failure_is_reported_to_commands() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("broken.sqlite3");
    std::fs::write(&database, b"not a sqlite database").unwrap();
    let services = AppServices::start(
        database,
        dir.path().join("providers.json"),
        dir.path().join("translator.json"),
        dir.path().join("plugins"),
        "local",
        Default::default(),
        false,
        30,
        services::ArticleFetchNetwork::default(),
    );
    let status = services.status();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for _ in 0..2 {
        let (reply, response) = oneshot::channel();
        services.send(Command::Snapshot {
            scope: Scope::All,
            search: String::new(),
            limit: 20,
            after: None,
            include_feeds: true,
            reply,
        });
        let error = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), response)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err()
        });
        assert!(error.contains("Could not start the database:"), "{error}");
    }
    assert!(
        matches!(&*status.borrow(), services::ServiceStatus::Failed(message) if message.contains("Could not start the database:"))
    );
}

#[test]
fn worker_routes_local_reads_and_checks_provider_operations() {
    let dir = tempfile::tempdir().unwrap();
    let services = AppServices::start(
        dir.path().join("library.sqlite3"),
        dir.path().join("providers.json"),
        dir.path().join("translator.json"),
        dir.path().join("plugins"),
        "local",
        panda_providers::ProviderSettingsMap::new(),
        false,
        30,
        services::ArticleFetchNetwork::default(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (reply, response) = oneshot::channel();
    services.send(Command::Snapshot {
        scope: Scope::All,
        search: String::new(),
        limit: 20,
        after: None,
        include_feeds: true,
        reply,
    });
    let snapshot = runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), response)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert!(snapshot.feeds.is_empty());
    assert!(snapshot.articles.is_empty());
    assert!(!snapshot.has_more);

    services.set_workspace("provider:freshrss");
    let (reply, response) = oneshot::channel();
    services.send(Command::UpdateFeed {
        id: 1,
        title: "Example".into(),
        folder: None,
        feed_url: "https://example.com/feed".into(),
        auto_translate_titles: false,
        reply,
    });
    let result = runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), response)
            .await
            .unwrap()
            .unwrap()
    });
    assert_eq!(
        result.unwrap_err(),
        "Feed editing is currently available in local mode only"
    );

    let (reply, response) = oneshot::channel();
    services.send(Command::Refresh {
        force: false,
        reply,
    });
    let result = runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), response)
            .await
            .unwrap()
            .unwrap()
    });
    assert_eq!(
        result.unwrap_err(),
        "Connect the selected provider before syncing"
    );
}
