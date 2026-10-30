// Exercise non-UI configuration code without the GPUI-heavy binary test harness.
#![allow(dead_code, unused_imports)]

#[path = "../src/app/backup.rs"]
mod backup;
#[path = "../src/app/config.rs"]
mod config;
#[path = "../src/ui/i18n.rs"]
mod i18n;
#[path = "../src/app/preferences.rs"]
pub(crate) mod preferences;
#[path = "../src/request_epoch.rs"]
mod request_epoch;
#[path = "../src/services/mod.rs"]
mod services;

mod app {
    pub(crate) use crate::preferences;
}

use config::AppConfig;
use panda_providers::{ProviderKind, ProviderSettings, ProviderSettingsMap};
use preferences::{Language, LibrarySource, Preferences};

#[test]
fn full_backup_is_staged_and_restored_on_next_start() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let db = source.path().join("panda-reader.sqlite3");
    panda_store::Store::migrate(&db).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute(
            "INSERT INTO feeds(id,feed_url,title) VALUES(1,'https://example.org/feed','feed')",
            [],
        )
        .unwrap();
    drop(connection);
    let archive = scratch.path().join("backup.pdbk");
    backup::write_backup_archive(
        source.path(),
        panda_backup::BackupMode::Full,
        &archive,
        serde_json::to_vec(&Preferences::default()).unwrap(),
        b"{}".to_vec(),
        b"{}".to_vec(),
        scratch.path(),
    )
    .unwrap();
    let validated = panda_backup::read_archive(&archive).unwrap();
    backup::stage_full_restore(target.path(), validated).unwrap();
    assert!(target.path().join("restore-ready").exists());
    let _config = AppConfig::load_from(target.path().to_owned());
    let restored = rusqlite::Connection::open(target.path().join("panda-reader.sqlite3")).unwrap();
    let count: i64 = restored
        .query_row("SELECT COUNT(*) FROM feeds", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert!(!target.path().join("restore-ready").exists());
}

#[test]
fn fresh_install_loads_defaults_without_creating_settings() {
    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig::load_from(dir.path().to_owned());
    assert_eq!(config.preferences.library_source, LibrarySource::Local);
    assert_eq!(config.preferences.language, Language::English);
    assert_eq!(config.preferences.translation_language, Language::ZhCn);
    assert!(config.provider_settings.is_empty());
    assert_eq!(config.database, dir.path().join("panda-reader.sqlite3"));
    assert!(!config.preferences_path.exists());
}

#[test]
fn legacy_miniflux_credentials_migrate_and_select_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("miniflux.json");
    panda_miniflux::Connection {
        endpoint: "https://legacy.example.com".into(),
        token: "legacy-token".into(),
    }
    .save(&legacy_path)
    .unwrap();
    std::fs::write(dir.path().join("preferences.json"), r#"{"theme":"dark"}"#).unwrap();

    let config = AppConfig::load_from(dir.path().to_owned());
    assert_eq!(config.preferences.library_source, LibrarySource::Miniflux);
    assert_eq!(config.preferences.theme, "dark");
    let saved = panda_providers::load_settings(&config.provider_settings_path).unwrap();
    assert_eq!(saved[&ProviderKind::Miniflux].secret, "legacy-token");
    assert_eq!(
        Preferences::load(&config.preferences_path).library_source,
        LibrarySource::Miniflux
    );
    let restarted = AppConfig::load_from(dir.path().to_owned());
    assert_eq!(
        restarted.preferences.library_source,
        LibrarySource::Miniflux
    );
    assert!(!legacy_path.exists());
}

#[test]
fn migration_preserves_explicit_source_and_existing_provider_settings() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        library_source: LibrarySource::FreshRss,
        language: Language::Japanese,
        ..Preferences::default()
    };
    preferences
        .save(&dir.path().join("preferences.json"))
        .unwrap();
    let mut providers = ProviderSettingsMap::new();
    providers.insert(
        ProviderKind::Miniflux,
        ProviderSettings {
            endpoint: "https://current.example.com".into(),
            username: String::new(),
            secret: "current-token".into(),
        },
    );
    panda_providers::save_settings(&dir.path().join("providers.json"), &providers).unwrap();
    panda_miniflux::Connection {
        endpoint: "https://legacy.example.com".into(),
        token: "legacy-token".into(),
    }
    .save(&dir.path().join("miniflux.json"))
    .unwrap();

    let config = AppConfig::load_from(dir.path().to_owned());
    assert_eq!(config.preferences.library_source, LibrarySource::FreshRss);
    assert_eq!(config.preferences.language, Language::Japanese);
    assert_eq!(
        config.provider_settings[&ProviderKind::Miniflux].secret,
        "current-token"
    );
}
