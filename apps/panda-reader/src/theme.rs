use crate::i18n::Language;
use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::*;
use panda_core::ContentExtractor;
pub use panda_core::TranslationLayout;
use panda_providers::ProviderKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::{path::Path, rc::Rc};

pub const DEFAULT_THEME_ID: &str = "bamboo";
pub const PANDA_READER_THEME_ID: &str = "panda_reader";

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
    pub language: Language,
    /// Target language for article body translation (independent of UI language).
    pub translation_language: Language,
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
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            library_source: LibrarySource::Local,
            theme: DEFAULT_THEME_ID.into(),
            sidebar_collapsed: false,
            show_app_icon: true,
            ui_font_size: 15.,
            language: Language::English,
            translation_language: Language::ZhCn,
            auto_extract_full_text: false,
            content_extractor: ContentExtractor::DomSmoothie,
            hide_images: false,
            collapsed_folders: Vec::new(),
            folder_order: Vec::new(),
            folder_icons: BTreeMap::new(),
            vim_navigation: false,
            translation_layout: TranslationLayout::Immersive,
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

#[derive(Clone, Copy)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub background: u32,
    pub foreground: u32,
    pub accent: u32,
    pub dark: bool,
    pub content: u32,
    pub panel: u32,
    pub raised: u32,
    pub selected: u32,
    pub sidebar_selected: u32,
    pub border: u32,
    pub hover: u32,
    pub muted: u32,
    pub shadow: bool,
}

// For a new standard theme, add one `standard_preset(...)` entry below.
// Use `custom_preset(...)` when the palette needs hand-tuned surface colors.
const fn standard_preset(
    id: &'static str,
    name: &'static str,
    background: u32,
    foreground: u32,
    accent: u32,
    dark: bool,
) -> Preset {
    Preset {
        id,
        name,
        background,
        foreground,
        accent,
        dark,
        content: background,
        panel: mix(background, foreground, if dark { 7 } else { 10 }),
        raised: mix(background, foreground, if dark { 11 } else { 4 }),
        selected: mix(background, accent, if dark { 24 } else { 14 }),
        sidebar_selected: mix(background, accent, if dark { 36 } else { 24 }),
        border: mix(background, foreground, if dark { 18 } else { 13 }),
        hover: mix(background, foreground, if dark { 13 } else { 7 }),
        muted: mix(foreground, background, if dark { 42 } else { 48 }),
        shadow: true,
    }
}

const fn custom_preset(
    id: &'static str,
    name: &'static str,
    background: u32,
    foreground: u32,
    accent: u32,
    content: u32,
    raised: u32,
    selected: u32,
    sidebar_selected: u32,
    border: u32,
    hover: u32,
    muted: u32,
) -> Preset {
    Preset {
        id,
        name,
        background,
        foreground,
        accent,
        dark: false,
        content,
        panel: background,
        raised,
        selected,
        sidebar_selected,
        border,
        hover,
        muted,
        shadow: false,
    }
}

pub const PRESETS: &[Preset] = &[
    custom_preset(
        DEFAULT_THEME_ID,
        "Panda Bamboo",
        0xf1f3f6,
        0x202a36,
        0x52775d,
        0xffffff,
        0xf7f9fb,
        0xe5eee6,
        0xd4e2d5,
        0xcbd3dc,
        0xe9eef2,
        0x6b776f,
    ),
    custom_preset(
        PANDA_READER_THEME_ID,
        "Panda Reader",
        0xf5f4ee,
        0x20281f,
        0x315d47,
        0xfbfaf5,
        0xeeeee5,
        0xe4eade,
        0xdde6d9,
        0xe3e2d8,
        0xebeee7,
        0x73796f,
    ),
    standard_preset("light", "Light", 0xfcfcfb, 0x1c1c1e, 0x1f6bf0, false),
    standard_preset(
        "one_light",
        "One Light",
        0xfafafa,
        0x383a42,
        0x4078f2,
        false,
    ),
    standard_preset(
        "latte",
        "Catppuccin Latte",
        0xeff1f5,
        0x4c4f69,
        0x1e66f5,
        false,
    ),
    standard_preset(
        "dawn",
        "Rosé Pine Dawn",
        0xfaf4ed,
        0x575279,
        0x907aa9,
        false,
    ),
    standard_preset("dark", "Dark", 0x18181a, 0xececed, 0x78a8f5, true),
    standard_preset(
        "one_dark",
        "One Dark Pro",
        0x282c34,
        0xabb2bf,
        0x528bff,
        true,
    ),
    standard_preset("nord", "Nord", 0x2e3440, 0xeceff4, 0x88c0d0, true),
    standard_preset(
        "mocha",
        "Catppuccin Mocha",
        0x1e1e2e,
        0xcdd6f4,
        0x89b4fa,
        true,
    ),
    standard_preset("tokyo", "Tokyo Night", 0x1a1b26, 0xc0caf5, 0x7aa2f7, true),
    standard_preset("rose", "Rosé Pine", 0x191724, 0xe0def4, 0xc4a7e7, true),
];

pub fn preset(id: &str) -> Preset {
    PRESETS
        .iter()
        .copied()
        .find(|item| item.id == id)
        .unwrap_or(PRESETS[0])
}

const fn mix(a: u32, b: u32, amount: u32) -> u32 {
    let red = (((a >> 16) & 0xff) * (100 - amount) + ((b >> 16) & 0xff) * amount) / 100;
    let green = (((a >> 8) & 0xff) * (100 - amount) + ((b >> 8) & 0xff) * amount) / 100;
    let blue = ((a & 0xff) * (100 - amount) + (b & 0xff) * amount) / 100;
    (red << 16) | (green << 8) | blue
}

fn hex(color: u32) -> SharedString {
    format!("#{color:06x}").into()
}

pub fn apply_theme(id: &str, cx: &mut App) {
    let selected = preset(id);
    let mode = if selected.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    let mut config = (*if selected.dark {
        ThemeRegistry::global(cx).default_dark_theme()
    } else {
        ThemeRegistry::global(cx).default_light_theme()
    })
    .as_ref()
    .clone();
    config.name = selected.name.into();
    config.mode = mode;
    config.font_family = Some("Source Sans 3".into());
    // Every theme uses the default theme's button radius, including large surfaces.
    config.radius = Some(3);
    config.radius_lg = Some(3);
    config.shadow = Some(selected.shadow);

    let base = selected.background;
    let ink = selected.foreground;
    let accent = selected.accent;
    let content = selected.content;
    let panel = selected.panel;
    let raised = selected.raised;
    let selected_bg = selected.selected;
    let sidebar_selected = selected.sidebar_selected;
    let border = selected.border;
    let hover = selected.hover;
    let muted = selected.muted;
    let colors = &mut config.colors;
    colors.background = Some(hex(content));
    colors.foreground = Some(hex(ink));
    colors.sidebar = Some(hex(panel));
    colors.sidebar_foreground = Some(hex(ink));
    colors.sidebar_accent = Some(hex(sidebar_selected));
    colors.sidebar_accent_foreground = Some(hex(ink));
    colors.sidebar_border = Some(hex(border));
    colors.sidebar_primary = Some(hex(accent));
    colors.sidebar_primary_foreground = Some(hex(if selected.dark { base } else { 0xffffff }));
    colors.list = Some(hex(content));
    colors.list_active = Some(hex(selected_bg));
    colors.list_hover = Some(hex(hover));
    colors.border = Some(hex(border));
    colors.muted = Some(hex(raised));
    colors.muted_foreground = Some(hex(muted));
    colors.accent = Some(hex(selected_bg));
    colors.accent_foreground = Some(hex(ink));
    colors.primary = Some(hex(accent));
    colors.primary_foreground = Some(hex(if selected.dark { base } else { 0xffffff }));
    colors.secondary = Some(hex(raised));
    colors.secondary_foreground = Some(hex(ink));
    colors.button_secondary = Some(hex(raised));
    colors.button_secondary_foreground = Some(hex(ink));
    colors.button_primary = Some(hex(accent));
    colors.button_primary_foreground = Some(hex(if selected.dark { base } else { 0xffffff }));
    colors.button = Some(hex(raised));
    colors.button_foreground = Some(hex(ink));
    colors.button_hover = Some(hex(hover));
    colors.title_bar = Some(hex(panel));
    colors.title_bar_border = Some(hex(border));
    colors.input = Some(hex(border));
    colors.link = Some(hex(accent));
    colors.selection = Some(hex(selected_bg));

    Theme::update(cx, |theme| theme.apply_config(&Rc::new(config)));
}
