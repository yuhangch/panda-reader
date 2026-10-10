//! Persisted application preferences. Keep serialization independent of the UI.

use panda_core::{ContentExtractor, TranslationLayout};
use panda_providers::ProviderKind;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const DEFAULT_THEME_ID: &str = "bamboo";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderFontFamily {
    Sans,
    #[default]
    Serif,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    ZhCn,
    ZhTw,
    Japanese,
    French,
    German,
}

impl Language {
    pub const ALL: [Self; 6] = [
        Self::English,
        Self::ZhCn,
        Self::ZhTw,
        Self::Japanese,
        Self::French,
        Self::German,
    ];

    pub fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::ZhCn => "简体中文",
            Self::ZhTw => "繁體中文",
            Self::Japanese => "日本語",
            Self::French => "Français",
            Self::German => "Deutsch",
        }
    }

    pub fn english_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::ZhCn => "Chinese (Simplified)",
            Self::ZhTw => "Chinese (Traditional)",
            Self::Japanese => "Japanese",
            Self::French => "French",
            Self::German => "German",
        }
    }

    pub fn translator_code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::ZhCn => "zh-Hans",
            Self::ZhTw => "zh-Hant",
            Self::Japanese => "ja",
            Self::French => "fr",
            Self::German => "de",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|language| *language == self)
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySource {
    #[default]
    Local,
    Miniflux,
    FreshRss,
}

impl LibrarySource {
    pub const ALL: [Self; 3] = [Self::Local, Self::Miniflux, Self::FreshRss];

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|source| *source == self)
            .unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "Local subscriptions",
            Self::Miniflux => "Miniflux",
            Self::FreshRss => "FreshRSS",
        }
    }

    pub fn workspace(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Miniflux => "provider:miniflux",
            Self::FreshRss => "provider:freshrss",
        }
    }

    pub fn provider(self) -> Option<ProviderKind> {
        match self {
            Self::Local => None,
            Self::Miniflux => Some(ProviderKind::Miniflux),
            Self::FreshRss => Some(ProviderKind::FreshRss),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub library_source: LibrarySource,
    pub theme: String,
    pub sidebar_collapsed: bool,
    /// Show the panda app icon in the custom title bar.
    pub show_app_icon: bool,
    pub ui_font_size: f32,
    /// Font family used by article titles and body text.
    pub reader_font_family: ReaderFontFamily,
    pub reader_font_size: f32,
    pub reader_line_height: f32,
    pub reader_content_width: f32,
    pub reader_paragraph_spacing: f32,
    pub remember_reading_position: bool,
    pub language: Language,
    /// Target language for article body translation (independent of UI language).
    pub translation_language: Language,
    /// Add two full-width spaces at the start of article paragraphs.
    pub paragraph_indent: bool,
    /// Translate confidently identified article titles in the background.
    pub auto_translate_titles: bool,
    /// Restrict automatic title translation to articles received after this feature was added.
    pub only_translate_future_titles: bool,
    pub auto_extract_full_text: bool,
    /// Which library extracts article body from the fetched page.
    pub content_extractor: ContentExtractor,
    pub hide_images: bool,
    pub collapsed_folders: Vec<String>,
    /// Manual sidebar folder order (local-only; Miniflux has no category order API).
    pub folder_order: Vec<String>,
    /// Per-folder icon key (`folder`, `star`, `book`, …).
    pub folder_icons: BTreeMap<String, String>,
    /// Single-letter navigation when not typing in an input.
    pub vim_navigation: bool,
    /// How to present a finished translation in the reader.
    pub translation_layout: TranslationLayout,
    /// Text copied by the article share action. `{title}`, `{url}`, and `\\n` are expanded.
    pub share_template: String,
    /// Time of the most recent successful feed refresh, stored in RFC 3339 format.
    pub last_refresh_at: Option<String>,
    /// Check for and download stable releases in the background.
    pub check_for_updates: bool,
    /// Include detailed provider connection and sync timings in sync.log.
    pub detailed_sync_logging: bool,
    /// Delete sync.log and its rotated copy when they exceed this age.
    pub sync_log_retention_days: u16,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            library_source: LibrarySource::Local,
            theme: DEFAULT_THEME_ID.into(),
            sidebar_collapsed: false,
            show_app_icon: true,
            ui_font_size: 15.,
            reader_font_family: ReaderFontFamily::Serif,
            reader_font_size: 19.,
            reader_line_height: 1.65,
            reader_content_width: 680.,
            reader_paragraph_spacing: 1.35,
            remember_reading_position: true,
            language: Language::English,
            translation_language: Language::ZhCn,
            paragraph_indent: true,
            auto_translate_titles: false,
            only_translate_future_titles: true,
            auto_extract_full_text: false,
            content_extractor: ContentExtractor::DomSmoothie,
            hide_images: false,
            collapsed_folders: Vec::new(),
            folder_order: Vec::new(),
            folder_icons: BTreeMap::new(),
            vim_navigation: false,
            translation_layout: TranslationLayout::Immersive,
            share_template: "{title}\\n{url}".into(),
            last_refresh_at: None,
            check_for_updates: true,
            detailed_sync_logging: false,
            sync_log_retention_days: 30,
        }
    }
}

impl Preferences {
    pub fn load(path: &Path) -> Self {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Self::default();
            }
            Err(error) => {
                eprintln!("could not read preferences at {}: {error}", path.display());
                return Self::default();
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(preferences) => preferences,
            Err(error) => {
                let backup = corrupt_preferences_path(path);
                match fs::rename(path, &backup) {
                    Ok(()) => eprintln!(
                        "preferences are invalid ({error}); original file preserved at {}",
                        backup.display()
                    ),
                    Err(rename_error) => eprintln!(
                        "preferences are invalid ({error}); could not preserve {}: {rename_error}",
                        path.display()
                    ),
                }
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let (temp_path, mut file) = (0..100)
            .find_map(|_| {
                let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let candidate = parent.join(format!(
                    ".preferences-{}-{sequence}.tmp",
                    std::process::id()
                ));
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&candidate)
                {
                    Ok(file) => Some(Ok((candidate, file))),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .unwrap_or_else(|| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "could not allocate a temporary preferences file",
                ))
            })?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            replace_file(&temp_path, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result
    }
}

fn corrupt_preferences_path(path: &Path) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let base = path.file_name().unwrap_or_default().to_string_lossy();
    (0..100)
        .map(|index| {
            let suffix = if index == 0 {
                String::new()
            } else {
                format!("-{index}")
            };
            path.with_file_name(format!("{base}.corrupt-{timestamp}{suffix}"))
        })
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| path.with_file_name(format!("{base}.corrupt-{timestamp}-overflow")))
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // MoveFileExW replaces an existing file atomically on the same volume.
    let success = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_translation_defaults_off_and_survives_preference_round_trip() {
        assert!(!Preferences::default().auto_translate_titles);
        let legacy: Preferences =
            serde_json::from_str("{\"translation_language\":\"zh_cn\"}").unwrap();
        assert!(!legacy.auto_translate_titles);
        let mut current = Preferences::default();
        current.auto_translate_titles = true;
        let parsed: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&current).unwrap()).unwrap();
        assert!(parsed.auto_translate_titles);
    }

    #[test]
    fn detailed_sync_logging_defaults_off_and_survives_preference_round_trip() {
        let legacy: Preferences = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert!(!legacy.detailed_sync_logging);
        assert_eq!(legacy.sync_log_retention_days, 30);

        let mut current = Preferences::default();
        current.detailed_sync_logging = true;
        current.sync_log_retention_days = 90;
        let parsed: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&current).unwrap()).unwrap();
        assert!(parsed.detailed_sync_logging);
        assert_eq!(parsed.sync_log_retention_days, 90);
    }

    #[test]
    fn invalid_preferences_are_preserved_before_defaults_are_used() {
        let directory = std::env::temp_dir().join(format!(
            "panda-preferences-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        fs::write(&path, b"{ not valid json").unwrap();

        let preferences = Preferences::load(&path);

        assert_eq!(preferences.theme, DEFAULT_THEME_ID);
        assert!(!path.exists());
        let backup = fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|candidate| candidate.to_string_lossy().contains(".corrupt-"))
            .expect("invalid preferences should have a backup");
        assert_eq!(fs::read(backup).unwrap(), b"{ not valid json");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_preferences_replaces_existing_file_without_truncation_window() {
        let directory = std::env::temp_dir().join(format!(
            "panda-preferences-save-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        fs::write(&path, b"old contents").unwrap();
        let mut preferences = Preferences::default();
        preferences.theme = "bamboo-dark".into();

        preferences.save(&path).unwrap();

        assert_eq!(Preferences::load(&path).theme, "bamboo-dark");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
