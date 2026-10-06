//! Persisted application preferences. Keep serialization independent of the UI.

use panda_core::{ContentExtractor, TranslationLayout};
use panda_providers::ProviderKind;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

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
        }
    }
}

impl Preferences {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, bytes)
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
}
