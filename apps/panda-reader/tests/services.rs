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
fn worker_routes_local_reads_and_checks_provider_operations() {
    let dir = tempfile::tempdir().unwrap();
    let services = AppServices::start(
        dir.path().join("library.sqlite3"),
        dir.path().join("providers.json"),
        dir.path().join("translator.json"),
        dir.path().join("plugins"),
        "local",
        panda_providers::ProviderSettingsMap::new(),
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
