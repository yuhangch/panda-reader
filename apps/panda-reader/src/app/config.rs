//! Paths, persisted settings, and startup migration.

use super::preferences::{LibrarySource, Preferences};
use directories::ProjectDirs;
use panda_providers::ProviderSettingsMap;
use panda_translate::TranslatorConfig;
use std::path::PathBuf;

pub(super) struct AppConfig {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub preferences_path: PathBuf,
    pub provider_settings_path: PathBuf,
    pub translator_path: PathBuf,
    pub preferences: Preferences,
    pub provider_settings: ProviderSettingsMap,
    pub translator_config: TranslatorConfig,
}

fn data_dir() -> PathBuf {
    ProjectDirs::from("com", "PandaReader", "PandaReader")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("data"))
}

impl AppConfig {
    pub fn load() -> Self {
        Self::load_from(data_dir())
    }

    pub(super) fn load_from(data_dir: PathBuf) -> Self {
        let database = data_dir.join("panda-reader.sqlite3");
        let preferences_path = data_dir.join("preferences.json");
        let provider_settings_path = data_dir.join("providers.json");
        let legacy_credentials_path = data_dir.join("miniflux.json");
        let translator_path = data_dir.join("translator.json");
        let mut preferences = Preferences::load(&preferences_path);
        let has_saved_source = std::fs::read(&preferences_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|value| value.get("library_source").is_some());
        let mut provider_settings = panda_providers::load_settings(&provider_settings_path)
            .unwrap_or_else(|error| {
                eprintln!("could not read provider settings: {error}");
                panda_providers::ProviderSettingsMap::new()
            });
        if let Ok(Some(legacy)) = panda_miniflux::Connection::load(&legacy_credentials_path) {
            provider_settings
                .entry(panda_providers::ProviderKind::Miniflux)
                .or_insert_with(|| panda_providers::ProviderSettings {
                    endpoint: legacy.endpoint,
                    username: String::new(),
                    secret: legacy.token,
                });
            if !has_saved_source {
                preferences.library_source = LibrarySource::Miniflux;
            }
            let settings_saved =
                match panda_providers::save_settings(&provider_settings_path, &provider_settings) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("could not migrate Miniflux settings: {error}");
                        false
                    }
                };
            let preferences_saved = if has_saved_source {
                true
            } else {
                preferences.library_source = LibrarySource::Miniflux;
                match preferences.save(&preferences_path) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("could not save migrated library selection: {error}");
                        false
                    }
                }
            };
            if settings_saved
                && preferences_saved
                && let Err(error) = std::fs::remove_file(&legacy_credentials_path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("could not remove migrated Miniflux settings: {error}");
            }
        }
        let translator_config = panda_translate::TranslatorConfig::load(&translator_path)
            .unwrap_or_else(|error| {
                eprintln!("could not read translator settings: {error}");
                panda_translate::TranslatorConfig::default()
            });
        Self {
            data_dir,
            database,
            preferences_path,
            provider_settings_path,
            translator_path,
            preferences,
            provider_settings,
            translator_config,
        }
    }
}
