// Exercise non-UI configuration code without the GPUI-heavy binary test harness.
#![allow(dead_code)]

#[path = "../src/app/config.rs"]
mod config;
#[path = "../src/ui/i18n.rs"]
mod i18n;
#[path = "../src/app/preferences.rs"]
pub(crate) mod preferences;

mod app {
    pub(crate) use crate::preferences;
}

use config::AppConfig;
use panda_providers::{ProviderKind, ProviderSettings, ProviderSettingsMap};
use preferences::{Language, LibrarySource, Preferences};

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
