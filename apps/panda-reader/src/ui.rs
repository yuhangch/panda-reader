use crate::backend::{Backend, Command};
use crate::i18n::{self, Language};
use crate::theme::{self, LibrarySource, PRESETS, Preferences, TranslationLayout};
use gpui_kit::base::{StyledExt as _, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, InputState},
    menu::ContextMenuExt as _,
    scroll::ScrollableElement as _,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    switch::Switch,
    text::TextView,
    text::TextViewStyle,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use panda_core::{Article, ArticleCursor, ArticleSummary, Feed, MarkField, Scope};
use panda_providers::{ProviderKind, ProviderSettings, ProviderSettingsMap};
use panda_translate::{Provider, TranslatorConfig};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::oneshot;

mod clipboard_input;
mod commands;
mod context_menus;
mod dispatch;
mod keymap;
mod palette;
mod settings;

pub use keymap::bind_keys;

use clipboard_input::text_input;
use commands::CommandKind;
use dispatch::{feed_host, letter_avatar, local_favicon_path};
use keymap::*;
use palette::PaletteState;

const ARTICLE_PAGE: i64 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusTone {
    Busy,
    Error,
    Flash,
}

impl SearchableListItem for LibrarySource {
    type Value = Self;
    fn title(&self) -> SharedString {
        SharedString::from(self.label())
    }
    fn value(&self) -> &Self::Value {
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TranslatorProvider {
    provider: Provider,
    language: Language,
}

enum SidebarRow {
    Feed {
        feed_index: usize,
        nested: bool,
        has_error: bool,
    },
    Folder {
        name: String,
        unread: i64,
        collapsed: bool,
    },
}

impl SidebarRow {
    fn height(&self) -> f32 {
        match self {
            Self::Feed { has_error, .. } => {
                if *has_error {
                    44.
                } else {
                    30.
                }
            }
            Self::Folder { .. } => 30.,
        }
    }
}

impl TranslatorProvider {
    const PROVIDERS: [Provider; 4] = [
        Provider::Azure,
        Provider::Volcengine,
        Provider::DeepL,
        Provider::LibreTranslate,
    ];

    fn all(language: Language) -> Vec<Self> {
        Self::PROVIDERS
            .into_iter()
            .map(|provider| Self { provider, language })
            .collect()
    }

    fn label(self) -> &'static str {
        i18n::text(self.language, self.provider.display_name())
    }
}

impl SearchableListItem for TranslatorProvider {
    type Value = Self;

    fn title(&self) -> SharedString {
        if self.provider.is_ready() {
            SharedString::from(self.label())
        } else {
            SharedString::from(format!(
                "{} ({})",
                self.label(),
                i18n::text(self.language, "Coming soon")
            ))
        }
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

#[derive(Clone, Debug)]
struct FolderDrag(String);

struct FolderDragPreview {
    name: SharedString,
}

impl Render for FolderDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .bg(cx.theme().sidebar_accent)
            .text_color(cx.theme().sidebar_accent_foreground)
            .text_sm()
            .child(tty_icon("folder").small())
            .child(self.name.clone())
    }
}

static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../assets/app-icon.png").to_vec(),
    ))
});

fn tty_icon(name: &str) -> Icon {
    let bytes: &'static [u8] = match name {
        "panel-left" => include_bytes!("../assets/icons/panel-left.svg"),
        "plus" => include_bytes!("../assets/icons/plus.svg"),
        "ellipsis" => include_bytes!("../assets/icons/ellipsis.svg"),
        "refresh" => include_bytes!("../assets/icons/refresh.svg"),
        "folder" => include_bytes!("../assets/icons/folder-closed.svg"),
        "settings" => include_bytes!("../assets/icons/settings.svg"),
        "appearance" => include_bytes!("../assets/icons/appearance.svg"),
        "about" => include_bytes!("../assets/icons/about.svg"),
        "close" => include_bytes!("../assets/icons/close.svg"),
        "list-flat" => include_bytes!("../assets/icons/list-flat.svg"),
        "image" => include_bytes!("../assets/icons/image.svg"),
        "image-off" => include_bytes!("../assets/icons/image-off.svg"),
        "star" => include_bytes!("../assets/icons/star.svg"),
        "star-fill" => include_bytes!("../assets/icons/star-fill.svg"),
        "bookmark" => include_bytes!("../assets/icons/bookmark.svg"),
        "bookmark-check" => include_bytes!("../assets/icons/bookmark-check.svg"),
        _ => unreachable!("unknown tty7 icon"),
    };
    Icon::default().data(bytes)
}

fn preview_text(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let mut preview: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsPage {
    General,
    Appearance,
    Reading,
    About,
}

pub struct ReaderView {
    backend: Backend,
    focus_handle: FocusHandle,
    add_input: Entity<InputState>,
    provider_url_input: Entity<InputState>,
    provider_username_input: Entity<InputState>,
    provider_secret_input: Entity<InputState>,
    search_input: Entity<InputState>,
    added_feeds_search_input: Entity<InputState>,
    language_select: Entity<SelectState<SearchableVec<Language>>>,
    translation_language_select: Entity<SelectState<SearchableVec<Language>>>,
    provider_select: Entity<SelectState<SearchableVec<TranslatorProvider>>>,
    upstream_select: Entity<SelectState<SearchableVec<LibrarySource>>>,
    azure_key_input: Entity<InputState>,
    azure_region_input: Entity<InputState>,
    volcengine_ak_input: Entity<InputState>,
    volcengine_sk_input: Entity<InputState>,
    feed_title_input: Entity<InputState>,
    feed_folder_input: Entity<InputState>,
    feed_url_input: Entity<InputState>,
    feeds: Vec<Feed>,
    feed_list_scroll: VirtualListScrollHandle,
    articles: Arc<Vec<ArticleSummary>>,
    articles_has_more: bool,
    article_list_scroll: UniformListScrollHandle,
    article: Option<Article>,
    article_body: SharedString,
    showing_translation: bool,
    scope: Scope,
    search: String,
    added_feeds_search: String,
    search_revision: u64,
    snapshot_revision: u64,
    article_revision: u64,
    pending_remove_feed: Option<i64>,
    editing_feed: Option<i64>,
    is_loading: bool,
    is_loading_more: bool,
    is_refreshing: bool,
    is_connecting: bool,
    is_extracting: bool,
    provider_settings: ProviderSettingsMap,
    favicon_requested: HashSet<String>,
    icons_dir: PathBuf,
    status: Option<String>,
    status_tone: StatusTone,
    status_token: u64,
    library_source: LibrarySource,
    preferences: Preferences,
    preferences_path: PathBuf,
    translator_config: TranslatorConfig,
    translator_path: PathBuf,
    settings_open: bool,
    settings_page: SettingsPage,
    pending_reset_settings: bool,
    theme_picker_open: bool,
    palette: PaletteState,
    _subscriptions: Vec<Subscription>,
}

impl ReaderView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        backend: Backend,
        preferences_path: PathBuf,
        preferences: Preferences,
        translator_path: PathBuf,
        translator_config: TranslatorConfig,
        provider_settings: ProviderSettingsMap,
        data_dir: PathBuf,
    ) -> Self {
        let library_source = preferences.library_source;
        let selected_settings = library_source
            .provider()
            .and_then(|kind| provider_settings.get(&kind));
        let add_placeholder = i18n::text(preferences.language, "Feed URL https://…");
        let add_input = cx.new(|cx| InputState::new(window, cx).placeholder(add_placeholder));
        let provider_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://provider.example.org")
                .default_value(selected_settings.map_or("", |value| value.endpoint.as_str()))
        });
        let provider_username_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Username")
                .default_value(selected_settings.map_or("", |value| value.username.as_str()))
        });
        let provider_secret_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("API token or API password")
                .masked(true)
                .default_value(selected_settings.map_or("", |value| value.secret.as_str()))
        });
        let search_placeholder = i18n::text(preferences.language, "Search articles");
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder(search_placeholder));
        let added_feeds_search_placeholder = i18n::text(preferences.language, "Search added feeds");
        let added_feeds_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(added_feeds_search_placeholder));
        let language_index = IndexPath::new(preferences.language.index());
        let language_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Language::ALL.to_vec()),
                Some(language_index),
                window,
                cx,
            )
        });
        let translation_language_index = IndexPath::new(preferences.translation_language.index());
        let translation_language_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Language::ALL.to_vec()),
                Some(translation_language_index),
                window,
                cx,
            )
        });
        let provider_index = IndexPath::new(translator_config.provider.index());
        let provider_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(TranslatorProvider::all(preferences.language)),
                Some(provider_index),
                window,
                cx,
            )
        });
        let upstream_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(LibrarySource::ALL.to_vec()),
                Some(IndexPath::new(library_source.index())),
                window,
                cx,
            )
        });
        let azure_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Azure Translator key")
                .masked(true)
                .default_value(translator_config.azure_key.as_str())
        });
        let azure_region_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("eastasia")
                .default_value(translator_config.azure_region.as_str())
        });
        let volcengine_ak_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Access Key ID")
                .default_value(translator_config.volcengine_access_key.as_str())
        });
        let volcengine_sk_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Secret Access Key")
                .masked(true)
                .default_value(translator_config.volcengine_secret_key.as_str())
        });
        let feed_title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Feed title"));
        let feed_folder_input = cx.new(|cx| InputState::new(window, cx).placeholder("Folder"));
        let feed_url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://…"));
        let search_for_events = search_input.clone();
        let added_feeds_search_for_events = added_feeds_search_input.clone();
        let mut _subscriptions =
            vec![
                cx.subscribe_in(&search_input, window, move |this, _, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.search = search_for_events.read(cx).value().to_string();
                        this.search_revision = this.search_revision.wrapping_add(1);
                        let revision = this.search_revision;
                        cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(Duration::from_millis(200))
                                .await;
                            let _ = this.update(cx, |this, cx| {
                                if revision != this.search_revision {
                                    return;
                                }
                                this.load_snapshot(cx);
                            });
                        })
                        .detach();
                    }
                }),
            ];
        _subscriptions.push(cx.subscribe_in(
            &added_feeds_search_input,
            window,
            move |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.added_feeds_search =
                        added_feeds_search_for_events.read(cx).value().to_string();
                    cx.notify();
                }
            },
        ));
        _subscriptions.push(cx.subscribe_in(
            &language_select,
            window,
            |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(language)) = event {
                    this.set_language(*language, window, cx);
                }
            },
        ));
        _subscriptions.push(cx.subscribe_in(
            &translation_language_select,
            window,
            |this, _, event, _, cx| {
                if let SelectEvent::Confirm(Some(language)) = event {
                    this.set_translation_language(*language, cx);
                }
            },
        ));
        _subscriptions.push(
            cx.subscribe_in(&provider_select, window, |this, _, event, _, cx| {
                if let SelectEvent::Confirm(Some(provider)) = event {
                    this.translator_config.provider = provider.provider;
                    this.save_translator_config();
                    cx.notify();
                }
            }),
        );
        _subscriptions.push(cx.subscribe_in(
            &upstream_select,
            window,
            |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(kind)) = event {
                    this.library_source = *kind;
                    this.preferences.library_source = *kind;
                    this.save_preferences();
                    this.backend.set_workspace(kind.workspace());
                    let settings = kind
                        .provider()
                        .and_then(|provider| this.provider_settings.get(&provider));
                    this.provider_url_input.update(cx, |input, cx| {
                        input.set_value(
                            settings.map_or("", |value| value.endpoint.as_str()),
                            window,
                            cx,
                        )
                    });
                    this.provider_username_input.update(cx, |input, cx| {
                        input.set_value(
                            settings.map_or("", |value| value.username.as_str()),
                            window,
                            cx,
                        )
                    });
                    this.provider_secret_input.update(cx, |input, cx| {
                        input.set_value(
                            settings.map_or("", |value| value.secret.as_str()),
                            window,
                            cx,
                        )
                    });
                    this.scope = Scope::All;
                    this.article = None;
                    this.articles = Arc::new(Vec::new());
                    this.feeds.clear();
                    this.load_snapshot(cx);
                    cx.notify();
                }
            },
        ));
        let mut view = Self {
            backend,
            focus_handle: cx.focus_handle(),
            add_input,
            provider_url_input,
            provider_username_input,
            provider_secret_input,
            search_input,
            added_feeds_search_input,
            language_select,
            translation_language_select,
            provider_select,
            upstream_select,
            azure_key_input,
            azure_region_input,
            volcengine_ak_input,
            volcengine_sk_input,
            feed_title_input,
            feed_folder_input,
            feed_url_input,
            feeds: Vec::new(),
            feed_list_scroll: VirtualListScrollHandle::new(),
            articles: Arc::new(Vec::new()),
            articles_has_more: false,
            article_list_scroll: UniformListScrollHandle::new(),
            article: None,
            article_body: SharedString::default(),
            showing_translation: false,
            scope: Scope::All,
            search: String::new(),
            added_feeds_search: String::new(),
            search_revision: 0,
            snapshot_revision: 0,
            article_revision: 0,
            pending_remove_feed: None,
            editing_feed: None,
            is_loading: true,
            is_loading_more: false,
            is_refreshing: false,
            is_connecting: false,
            is_extracting: false,
            provider_settings,
            favicon_requested: HashSet::new(),
            icons_dir: data_dir.join("feed-icons"),
            status: None,
            status_tone: StatusTone::Busy,
            status_token: 0,
            library_source,
            preferences,
            preferences_path,
            translator_config,
            translator_path,
            settings_open: false,
            settings_page: SettingsPage::General,
            pending_reset_settings: false,
            theme_picker_open: false,
            palette: PaletteState::new(window, cx),
            _subscriptions,
        };
        view.subscribe_palette_input(window, cx);
        view.load_snapshot(cx);
        view
    }

    fn save_preferences(&mut self) {
        if let Err(error) = self.preferences.save(&self.preferences_path) {
            self.set_error(format!("Could not save settings: {error}"));
        }
    }

    fn save_translator_config(&mut self) {
        if let Err(error) = self.translator_config.save(&self.translator_path) {
            self.set_error(format!("Could not save settings: {error}"));
        }
    }

    fn reset_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preferences = Preferences::default();
        self.library_source = self.preferences.library_source;
        self.translator_config = TranslatorConfig::default();
        self.pending_reset_settings = false;

        self.language_select.update(cx, |select, cx| {
            select.set_selected_index(
                Some(IndexPath::new(self.preferences.language.index())),
                window,
                cx,
            )
        });
        self.translation_language_select.update(cx, |select, cx| {
            select.set_selected_index(
                Some(IndexPath::new(
                    self.preferences.translation_language.index(),
                )),
                window,
                cx,
            )
        });
        self.provider_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(TranslatorProvider::all(self.preferences.language)),
                window,
                cx,
            );
            select.set_selected_index(
                Some(IndexPath::new(self.translator_config.provider.index())),
                window,
                cx,
            );
        });
        self.upstream_select.update(cx, |select, cx| {
            select.set_selected_index(
                Some(IndexPath::new(self.library_source.index())),
                window,
                cx,
            )
        });

        self.add_input.update(cx, |input, cx| {
            input.set_placeholder(
                i18n::text(self.preferences.language, "Feed URL https://…"),
                window,
                cx,
            )
        });
        self.search_input.update(cx, |input, cx| {
            input.set_placeholder(
                i18n::text(self.preferences.language, "Search articles"),
                window,
                cx,
            )
        });
        self.azure_key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.azure_region_input
            .update(cx, |input, cx| input.set_value("eastasia", window, cx));
        self.volcengine_ak_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.volcengine_sk_input
            .update(cx, |input, cx| input.set_value("", window, cx));

        theme::apply_theme(&self.preferences.theme, cx);
        self.backend.set_workspace(self.library_source.workspace());
        self.select_scope(Scope::All, cx);

        let preferences_result = self.preferences.save(&self.preferences_path);
        let translator_result = self.translator_config.save(&self.translator_path);
        match (preferences_result, translator_result) {
            (Ok(()), Ok(())) => self.set_flash(self.t("Settings restored to defaults."), cx),
            (Err(error), _) => self.set_error(format!("Could not save settings: {error}")),
            (_, Err(error)) => self.set_error(format!("Could not save settings: {error}")),
        }
        cx.notify();
    }

    fn persist_translator_settings(&mut self, cx: &mut Context<Self>) {
        self.translator_config.azure_key = self.azure_key_input.read(cx).value().to_string();
        self.translator_config.azure_region = self.azure_region_input.read(cx).value().to_string();
        self.translator_config.volcengine_access_key =
            self.volcengine_ak_input.read(cx).value().to_string();
        self.translator_config.volcengine_secret_key =
            self.volcengine_sk_input.read(cx).value().to_string();
        self.save_translator_config();
        self.set_flash(self.t("Translation settings saved"), cx);
        cx.notify();
    }

    fn t(&self, english: &'static str) -> &'static str {
        i18n::text(self.preferences.language, english)
    }

    fn set_busy(&mut self, message: impl Into<String>) {
        self.status_token = self.status_token.wrapping_add(1);
        self.status = Some(message.into());
        self.status_tone = StatusTone::Busy;
    }

    fn set_flash(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.status_token = self.status_token.wrapping_add(1);
        let token = self.status_token;
        self.status = Some(message.into());
        self.status_tone = StatusTone::Flash;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = this.update(cx, |this, cx| {
                if this.status_token == token && matches!(this.status_tone, StatusTone::Flash) {
                    this.status = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_error(&mut self, error: String) {
        self.status_token = self.status_token.wrapping_add(1);
        self.status = Some(i18n::error(self.preferences.language, error));
        self.status_tone = StatusTone::Error;
    }

    fn clear_status(&mut self) {
        self.status_token = self.status_token.wrapping_add(1);
        self.status = None;
    }

    fn unread_total(&self) -> i64 {
        self.feeds.iter().map(|feed| feed.unread).sum()
    }

    fn status_context_label(&self) -> String {
        let scope = match &self.scope {
            Scope::All => self.t("All Articles"),
            Scope::Unread => self.t("Unread"),
            Scope::Starred => self.t("Starred"),
            Scope::Later => self.t("Read Later"),
            Scope::Feed(id) => self
                .feeds
                .iter()
                .find(|feed| feed.id == *id)
                .map(|feed| feed.title.as_str())
                .unwrap_or_else(|| self.t("Feed")),
            Scope::Folder(name) => name.as_str(),
        };
        format!(
            "{} · {}",
            scope,
            i18n::format(self.preferences.language, "{} unread", self.unread_total())
        )
    }

    fn upstream_provider_label(&self) -> Option<&'static str> {
        self.library_source.provider().and_then(|kind| {
            self.provider_settings
                .contains_key(&kind)
                .then_some(self.library_source.label())
        })
    }

    fn set_language(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        self.preferences.language = language;
        self.clear_status();
        self.add_input.update(cx, |input, cx| {
            input.set_placeholder(i18n::text(language, "Feed URL https://…"), window, cx)
        });
        self.search_input.update(cx, |input, cx| {
            input.set_placeholder(i18n::text(language, "Search articles"), window, cx)
        });
        self.added_feeds_search_input.update(cx, |input, cx| {
            input.set_placeholder(i18n::text(language, "Search added feeds"), window, cx)
        });
        let selected = IndexPath::new(self.translator_config.provider.index());
        self.provider_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(TranslatorProvider::all(language)),
                window,
                cx,
            );
            select.set_selected_index(Some(selected), window, cx);
        });
        self.save_preferences();
        cx.notify();
    }

    fn set_translation_language(&mut self, language: Language, cx: &mut Context<Self>) {
        self.preferences.translation_language = language;
        if self.showing_translation {
            self.showing_translation = false;
            self.request_prepared_body(cx);
        }
        self.save_preferences();
        cx.notify();
    }

    fn translation_target_code(&self) -> &'static str {
        self.preferences.translation_language.translator_code()
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.preferences.sidebar_collapsed = !self.preferences.sidebar_collapsed;
        self.save_preferences();
        cx.notify();
    }

    fn toggle_hide_images(&mut self, cx: &mut Context<Self>) {
        self.preferences.hide_images = !self.preferences.hide_images;
        self.save_preferences();
        if self.article.is_some() {
            self.request_prepared_body(cx);
        }
        cx.notify();
    }

    fn toggle_translation_layout(&mut self, cx: &mut Context<Self>) {
        self.preferences.translation_layout = match self.preferences.translation_layout {
            TranslationLayout::Immersive => TranslationLayout::Replaced,
            TranslationLayout::Replaced => TranslationLayout::Immersive,
        };
        self.save_preferences();
        if self.article.is_some() {
            self.request_prepared_body(cx);
        }
        cx.notify();
    }

    fn choose_theme(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.preferences.theme = id.into();
        theme::apply_theme(id, cx);
        self.save_preferences();
        cx.notify();
    }

    fn set_ui_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.preferences.ui_font_size = size;
        self.save_preferences();
        cx.notify();
    }

    fn toggle_folder_collapsed(&mut self, folder: &str, cx: &mut Context<Self>) {
        if let Some(index) = self
            .preferences
            .collapsed_folders
            .iter()
            .position(|name| name == folder)
        {
            self.preferences.collapsed_folders.remove(index);
        } else {
            self.preferences.collapsed_folders.push(folder.to_owned());
        }
        self.save_preferences();
        cx.notify();
    }

    fn sync_folder_order(&mut self, folder_names: &[String]) {
        let mut order = self
            .preferences
            .folder_order
            .iter()
            .filter(|name| folder_names.iter().any(|n| n == *name))
            .cloned()
            .collect::<Vec<_>>();
        for name in folder_names {
            if !order.iter().any(|existing| existing == name) {
                order.push(name.clone());
            }
        }
        if order != self.preferences.folder_order {
            self.preferences.folder_order = order;
            self.save_preferences();
        }
    }

    fn sync_folder_order_from_feeds(&mut self) {
        let mut names = self
            .feeds
            .iter()
            .filter_map(|feed| feed.folder.clone())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        self.sync_folder_order(&names);
    }

    fn reorder_folder_before(&mut self, dragged: &str, target: &str, cx: &mut Context<Self>) {
        if dragged == target || dragged.is_empty() || target.is_empty() {
            return;
        }
        self.sync_folder_order_from_feeds();
        let order = &mut self.preferences.folder_order;
        let Some(from) = order.iter().position(|name| name == dragged) else {
            return;
        };
        order.remove(from);
        let Some(to) = order.iter().position(|name| name == target) else {
            order.push(dragged.to_owned());
            self.save_preferences();
            cx.notify();
            return;
        };
        order.insert(to, dragged.to_owned());
        self.save_preferences();
        cx.notify();
    }

    fn move_folder(&mut self, folder: &str, delta: isize, cx: &mut Context<Self>) {
        let order = &mut self.preferences.folder_order;
        let Some(index) = order.iter().position(|name| name == folder) else {
            return;
        };
        let next = index as isize + delta;
        if next < 0 || next >= order.len() as isize {
            return;
        }
        order.swap(index, next as usize);
        self.save_preferences();
        cx.notify();
    }

    fn set_folder_icon(&mut self, folder: &str, icon: &str, cx: &mut Context<Self>) {
        self.preferences
            .folder_icons
            .insert(folder.to_owned(), icon.to_owned());
        self.save_preferences();
        cx.notify();
    }

    fn folder_icon_element(icon_key: &str) -> Icon {
        match icon_key {
            "star" => tty_icon("star").small(),
            "book" => Icon::new(IconName::BookOpen).small(),
            "globe" => Icon::new(IconName::Globe).small(),
            "bookmark" => tty_icon("bookmark").small(),
            _ => tty_icon("folder").small(),
        }
    }

    fn folder_icon_key(&self, folder: &str) -> &str {
        self.preferences
            .folder_icons
            .get(folder)
            .map(String::as_str)
            .unwrap_or("folder")
    }

    fn select_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.article_revision = self.article_revision.wrapping_add(1);
        self.scope = scope;
        self.article = None;
        self.article_body = SharedString::default();
        self.showing_translation = false;
        self.articles = Arc::new(Vec::new());
        self.articles_has_more = false;
        self.load_snapshot(cx);
    }

    fn load_snapshot(&mut self, cx: &mut Context<Self>) {
        self.is_loading_more = false;
        self.load_snapshot_page(false, cx);
    }

    fn load_snapshot_page(&mut self, append: bool, cx: &mut Context<Self>) {
        self.snapshot_revision = self.snapshot_revision.wrapping_add(1);
        let revision = self.snapshot_revision;
        let after = if append {
            self.articles.last().map(ArticleCursor::from_summary)
        } else {
            None
        };
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::Snapshot {
            scope: self.scope.clone(),
            search: self.search.clone(),
            limit: ARTICLE_PAGE,
            after,
            include_feeds: !append,
            reply,
        });
        if !append {
            self.is_loading = self.articles.is_empty();
        }
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if revision != this.snapshot_revision {
                    return;
                }
                this.is_loading = false;
                this.is_loading_more = false;
                match result {
                    Ok(snapshot) => {
                        if !append {
                            if !snapshot.feeds.is_empty() || this.feeds.is_empty() {
                                this.feeds = snapshot.feeds;
                                this.sync_folder_order_from_feeds();
                                this.ensure_favicons(cx);
                            }
                            this.articles = Arc::new(snapshot.articles);
                        } else if !snapshot.articles.is_empty() {
                            let articles = Arc::make_mut(&mut this.articles);
                            articles.extend(snapshot.articles);
                        }
                        this.articles_has_more = snapshot.has_more;
                        if let Some(article) = &mut this.article {
                            article.summary = this
                                .articles
                                .iter()
                                .find(|row| row.id == article.summary.id)
                                .cloned()
                                .unwrap_or_else(|| article.summary.clone());
                        }
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_article(&mut self, id: i64, cx: &mut Context<Self>) {
        self.article_revision = self.article_revision.wrapping_add(1);
        let revision = self.article_revision;
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::Article {
            id,
            show_translation: false,
            translation_layout: self.preferences.translation_layout,
            hide_images: self.preferences.hide_images,
            reply,
        });
        let backend = self.backend.clone();
        // Optimistic list update — avoid a full snapshot round-trip.
        if let Some(summary) = Arc::make_mut(&mut self.articles)
            .iter_mut()
            .find(|article| article.id == id)
        {
            if !summary.is_read {
                summary.is_read = true;
                if let Some(feed) = self
                    .feeds
                    .iter_mut()
                    .find(|feed| feed.title == summary.feed_title)
                {
                    feed.unread = feed.unread.saturating_sub(1);
                }
            }
        }
        self.showing_translation = false;
        self.article_body = SharedString::default();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if revision != this.article_revision {
                    return;
                }
                match result {
                    Ok(prepared) => {
                        let mut article = prepared.article;
                        article.summary.is_read = true;
                        this.showing_translation = false;
                        this.article_body = prepared.body_html.into();
                        let should_auto_extract = this.preferences.auto_extract_full_text
                            && article.extracted_html.is_none()
                            && article.url.is_some();
                        this.article = Some(article);
                        let (reply, _) = oneshot::channel();
                        backend.send(Command::Mark {
                            id,
                            field: MarkField::Read,
                            value: true,
                            reply,
                        });
                        if should_auto_extract {
                            this.extract(id, false, cx);
                        }
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn request_prepared_body(&mut self, cx: &mut Context<Self>) {
        let Some(article) = &self.article else {
            return;
        };
        let id = article.summary.id;
        let revision = self.article_revision;
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::Article {
            id,
            show_translation: self.showing_translation,
            translation_layout: self.preferences.translation_layout,
            hide_images: self.preferences.hide_images,
            reply,
        });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if revision != this.article_revision {
                    return;
                }
                match result {
                    Ok(prepared) => {
                        this.article = Some(prepared.article);
                        this.article_body = prepared.body_html.into();
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn ensure_favicons(&mut self, cx: &mut Context<Self>) {
        for feed in &self.feeds {
            let Some(host) = feed_host(feed.site_url.as_deref(), &feed.feed_url) else {
                continue;
            };
            if local_favicon_path(&self.icons_dir, &host).is_some() {
                continue;
            }
            if !self.favicon_requested.insert(host.clone()) {
                continue;
            }
            let site_url = feed
                .site_url
                .clone()
                .unwrap_or_else(|| feed.feed_url.clone());
            let icons_dir = self.icons_dir.clone();
            let (reply, response) = oneshot::channel();
            self.backend.send(Command::EnsureFavicon {
                host,
                site_url,
                icons_dir,
                reply,
            });
            cx.spawn(async move |this, cx| {
                let _ = response.await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
    }

    fn connect_provider(&mut self, cx: &mut Context<Self>) {
        if self.is_connecting {
            return;
        }
        let Some(kind) = self.library_source.provider() else {
            return;
        };
        let endpoint = self.provider_url_input.read(cx).value().to_string();
        let username = self.provider_username_input.read(cx).value().to_string();
        let secret = self.provider_secret_input.read(cx).value().to_string();
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::Connect {
            kind,
            endpoint: endpoint.clone(),
            username: username.clone(),
            secret: secret.clone(),
            reply,
        });
        self.is_connecting = true;
        self.set_busy(self.t("Connecting upstream…"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".into()));
            let _ = this.update(cx, |this, cx| {
                this.is_connecting = false;
                match result {
                    Ok(username) => {
                        this.provider_settings.insert(
                            kind,
                            ProviderSettings {
                                endpoint,
                                username: if kind == ProviderKind::FreshRss {
                                    username.clone()
                                } else {
                                    String::new()
                                },
                                secret,
                            },
                        );
                        this.set_flash(
                            i18n::format(this.preferences.language, "Connected: {}", username),
                            cx,
                        );
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn disconnect_provider(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.library_source.provider() else {
            return;
        };
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::Disconnect { kind, reply });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".into()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.provider_settings.remove(&kind);
                        this.set_flash(
                            this.t("Disconnected; cached articles remain available"),
                            cx,
                        );
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn add_feed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.add_input.read(cx).value().trim().to_string();
        if url.is_empty() {
            return;
        }
        let _ = self
            .add_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::AddFeed { url, reply });
        self.set_busy(self.t("Adding feed…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.set_flash(this.t("Feed added"), cx);
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_feed(&mut self, id: i64, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::RemoveFeed { id, reply });
        self.set_busy(self.t("Removing feed…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                this.pending_remove_feed = None;
                match result {
                    Ok(()) => {
                        this.scope = Scope::All;
                        this.article = None;
                        this.set_flash(this.t("Feed removed from provider and locally"), cx);
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.is_refreshing {
            return;
        }
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::Refresh { reply });
        self.is_refreshing = true;
        self.set_busy(self.t("Syncing…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                this.is_refreshing = false;
                match result {
                    Ok(count) => {
                        this.set_flash(
                            i18n::format(this.preferences.language, "Synced {} articles", count),
                            cx,
                        );
                    }
                    Err(error) => this.set_error(error),
                }
                this.load_snapshot(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn mark(&mut self, id: i64, field: MarkField, value: bool, cx: &mut Context<Self>) {
        if let Some(article) = &mut self.article {
            if article.summary.id == id {
                match field {
                    MarkField::Read => article.summary.is_read = value,
                    MarkField::Starred => article.summary.is_starred = value,
                    MarkField::Later => article.summary.read_later = value,
                }
            }
        }
        if let Some(summary) = Arc::make_mut(&mut self.articles)
            .iter_mut()
            .find(|article| article.id == id)
        {
            match field {
                MarkField::Read => summary.is_read = value,
                MarkField::Starred => summary.is_starred = value,
                MarkField::Later => summary.read_later = value,
            }
        }
        cx.notify();

        let (reply, response) = oneshot::channel();
        self.backend.send(Command::Mark {
            id,
            field,
            value,
            reply,
        });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    if let Some(article) = &mut this.article {
                        if article.summary.id == id {
                            match field {
                                MarkField::Read => article.summary.is_read = !value,
                                MarkField::Starred => article.summary.is_starred = !value,
                                MarkField::Later => article.summary.read_later = !value,
                            }
                        }
                    }
                    if let Some(summary) = Arc::make_mut(&mut this.articles)
                        .iter_mut()
                        .find(|article| article.id == id)
                    {
                        match field {
                            MarkField::Read => summary.is_read = !value,
                            MarkField::Starred => summary.is_starred = !value,
                            MarkField::Later => summary.read_later = !value,
                        }
                    }
                    this.set_error(error);
                }
                this.load_snapshot(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn extract(&mut self, id: i64, force: bool, cx: &mut Context<Self>) {
        if self.is_extracting {
            return;
        }
        if !force
            && self.article.as_ref().is_some_and(|article| {
                article.summary.id == id
                    && article
                        .extracted_html
                        .as_deref()
                        .is_some_and(|html| !html.trim().is_empty())
            })
        {
            return;
        }
        let (reply, response) = oneshot::channel();
        let extractor = self.preferences.content_extractor;
        let show_translation = self.showing_translation;
        let translation_layout = self.preferences.translation_layout;
        let hide_images = self.preferences.hide_images;
        self.backend.send(Command::Extract {
            id,
            force,
            extractor,
            reply,
        });
        self.is_extracting = true;
        self.set_busy(self.t("Extracting full text…"));
        cx.notify();
        let (article_reply, article_response) = oneshot::channel();
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            if result.is_ok() {
                backend.send(Command::Article {
                    id,
                    show_translation,
                    translation_layout,
                    hide_images,
                    reply: article_reply,
                });
            }
            let _ = this.update(cx, |this, cx| {
                this.is_extracting = false;
                match result {
                    Ok(()) => {
                        this.set_flash(this.t("Full text extracted"), cx);
                        cx.spawn(async move |this, cx| {
                            if let Ok(Ok(prepared)) = article_response.await {
                                let _ = this.update(cx, |this, cx| {
                                    if this
                                        .article
                                        .as_ref()
                                        .is_some_and(|current| current.summary.id == id)
                                    {
                                        this.article_body = prepared.body_html.into();
                                        this.article = Some(prepared.article);
                                        cx.notify();
                                    }
                                });
                            }
                        })
                        .detach();
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn has_translation_for_ui(&self, article: &Article) -> bool {
        let target = self.translation_target_code();
        let source = article
            .extracted_html
            .as_deref()
            .filter(|html| !html.trim().is_empty())
            .unwrap_or(article.content_html.as_str());
        let hash = panda_translate::source_hash(source, article.summary.title.trim());
        article
            .translated_lang
            .as_deref()
            .is_some_and(|lang| lang == target)
            && article
                .translated_html
                .as_deref()
                .is_some_and(|html| !html.trim().is_empty())
            && article
                .translation_source_hash
                .as_deref()
                .is_some_and(|stored| stored == hash)
    }

    fn display_title(&self, article: &Article) -> String {
        if !self.showing_translation {
            return article.summary.title.clone();
        }
        let translated = article
            .translated_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty());
        match (self.preferences.translation_layout, translated) {
            (TranslationLayout::Immersive, Some(translated))
                if translated != article.summary.title =>
            {
                // Heading shows original; translated title is rendered separately.
                article.summary.title.clone()
            }
            (_, Some(translated)) => translated.to_owned(),
            (_, None) => article.summary.title.clone(),
        }
    }

    fn display_translated_title(&self, article: &Article) -> Option<String> {
        if !self.showing_translation
            || self.preferences.translation_layout != TranslationLayout::Immersive
        {
            return None;
        }
        article
            .translated_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty() && *title != article.summary.title)
            .map(str::to_owned)
    }

    fn toggle_or_translate(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(article) = self.article.as_ref() else {
            return;
        };
        if article.summary.id != id {
            return;
        }
        if self.showing_translation {
            self.showing_translation = false;
            self.request_prepared_body(cx);
            return;
        }
        if self.has_translation_for_ui(article) {
            self.showing_translation = true;
            self.request_prepared_body(cx);
            return;
        }
        if !self.translator_config.is_configured() {
            self.set_flash(
                self.t("Configure a translation provider in Settings before translating."),
                cx,
            );
            self.settings_open = true;
            self.settings_page = SettingsPage::Reading;
            cx.notify();
            return;
        }
        let target_lang = self.translation_target_code().to_owned();
        let translation_layout = self.preferences.translation_layout;
        let hide_images = self.preferences.hide_images;
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::Translate {
            id,
            target_lang,
            translation_layout,
            hide_images,
            reply,
        });
        self.set_busy(self.t("Translating…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(prepared) => {
                        if this
                            .article
                            .as_ref()
                            .is_some_and(|current| current.summary.id == id)
                        {
                            this.showing_translation = true;
                            this.article_body = prepared.body_html.into();
                            this.article = Some(prepared.article);
                            this.set_flash(this.t("Translation ready"), cx);
                        }
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn import_opml(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let backend = self.backend.clone();
        cx.spawn(async move |_, cx| {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("OPML", &["opml", "xml"])
                .pick_file()
                .await
            else {
                return;
            };
            let content = String::from_utf8_lossy(&file.read().await).into_owned();
            let (reply, response) = oneshot::channel();
            backend.send(Command::ImportOpml { content, reply });
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(count) => {
                        this.set_flash(
                            i18n::format(this.preferences.language, "Imported {} feeds", count),
                            cx,
                        );
                    }
                    Err(error) => this.set_error(error),
                }
                this.load_snapshot(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn export_opml(&mut self, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::ExportOpml { reply });
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            match result {
                Ok(content) => {
                    if let Some(file) = rfd::AsyncFileDialog::new()
                        .set_file_name("panda-reader.opml")
                        .add_filter("OPML", &["opml"])
                        .save_file()
                        .await
                    {
                        match file.write(content.as_bytes()).await {
                            Ok(()) => {
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_flash(this.t("OPML exported"), cx);
                                    cx.notify();
                                });
                            }
                            Err(error) => {
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_error(error.to_string());
                                    cx.notify();
                                });
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = weak.update(cx, |this, cx| {
                        this.set_error(error);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn render_title_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.preferences.sidebar_collapsed;
        let settings_open = self.settings_open;
        h_flex()
            .h(px(40.))
            .flex_shrink_0()
            .items_center()
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .child(
                h_flex()
                    .h_full()
                    .w(px(228.))
                    .pl(px(12.))
                    .items_center()
                    .gap_1()
                    .when(self.preferences.show_app_icon, |row| {
                        row.child(img(APP_ICON.clone()).size_5().rounded(cx.theme().radius))
                    })
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .pl_2()
                            .text_sm()
                            .font_semibold()
                            .child("Panda Reader")
                            .window_control_area(WindowControlArea::Drag)
                            .on_mouse_down(MouseButton::Left, |_, window, _| {
                                window.start_window_move();
                            }),
                    )
                    .child(
                        div().occlude().child(
                            Button::new("title-add-feed")
                                .small()
                                .ghost()
                                .icon(tty_icon("plus"))
                                .tooltip(self.t("Add feed"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.settings_open = true;
                                    this.settings_page = SettingsPage::General;
                                    this.add_input.read(cx).focus_handle(cx).focus(window, cx);
                                    cx.notify();
                                })),
                        ),
                    )
                    .when(!settings_open, |row| {
                        row.child(
                            div().occlude().child(
                                Button::new("toggle-sidebar")
                                    .small()
                                    .ghost()
                                    .icon(tty_icon("panel-left"))
                                    .tooltip(self.t(if collapsed {
                                        "Expand sidebar"
                                    } else {
                                        "Collapse sidebar"
                                    }))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
                                    ),
                            ),
                        )
                    }),
            )
            .child(
                div()
                    .id("title-bar-drag")
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    }),
            )
            .child(
                div().occlude().child(
                    Button::new("title-appearance")
                        .small()
                        .ghost()
                        .icon(tty_icon("appearance"))
                        .tooltip(self.t("Appearance"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings_open = true;
                            this.settings_page = SettingsPage::Appearance;
                            this.theme_picker_open = true;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div().occlude().child(
                    Button::new("title-settings")
                        .small()
                        .ghost()
                        .icon(tty_icon("settings"))
                        .tooltip(self.t("Settings"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings_open = true;
                            this.theme_picker_open = false;
                            cx.notify();
                        })),
                ),
            )
            .when(!cfg!(target_os = "macos"), |bar| {
                bar.child(
                    div()
                        .id("window-minimize")
                        .occlude()
                        .w(px(34.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().secondary_hover))
                        .window_control_area(WindowControlArea::Min)
                        .when(cfg!(target_os = "linux"), |button| {
                            button.on_click(|_, window, _| window.minimize_window())
                        })
                        .child(IconName::WindowMinimize),
                )
                .child(
                    div()
                        .id("window-maximize")
                        .occlude()
                        .w(px(34.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().secondary_hover))
                        .window_control_area(WindowControlArea::Max)
                        .when(cfg!(target_os = "linux"), |button| {
                            button.on_click(|_, window, _| window.zoom_window())
                        })
                        .child(if window.is_maximized() {
                            IconName::WindowRestore
                        } else {
                            IconName::WindowMaximize
                        }),
                )
                .child(
                    div()
                        .id("window-close")
                        .occlude()
                        .w(px(34.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().danger))
                        .window_control_area(WindowControlArea::Close)
                        .when(cfg!(target_os = "linux"), |button| {
                            button.on_click(|_, window, _| window.remove_window())
                        })
                        .child(IconName::WindowClose),
                )
            })
    }

    fn render_feed_row(
        &self,
        feed: &Feed,
        nested: bool,
        selected_scope: &Scope,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = feed.id;
        let is_selected = selected_scope == &Scope::Feed(id);
        let menu_app = cx.entity().downgrade();
        let language = self.preferences.language;
        let local = self.library_source == LibrarySource::Local;
        let menu_feed = feed.clone();
        let sidebar_bg = cx.theme().sidebar;
        let active_bg = cx.theme().sidebar_accent;
        h_flex()
            .id(("feed-row", id as u64))
            .w_full()
            .h(px(if feed.last_error.is_some() { 44. } else { 30. }))
            .items_center()
            .rounded(cx.theme().radius)
            .bg(if is_selected { active_bg } else { sidebar_bg })
            .hover(|style| style.bg(active_bg))
            .child(
                h_flex()
                    .id(("feed", id as u64))
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .when(nested, |row| row.pl_5())
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child({
                        let avatar = letter_avatar(&feed.title);
                        let danger = feed.last_error.is_some();
                        let accent_bg = if danger {
                            cx.theme().danger.opacity(0.18)
                        } else {
                            cx.theme().accent.opacity(0.16)
                        };
                        let accent_fg = if danger {
                            cx.theme().danger
                        } else {
                            cx.theme().accent
                        };
                        let letter = {
                            let avatar = avatar.clone();
                            move || {
                                div()
                                    .size(px(20.))
                                    .rounded(px(4.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(accent_bg)
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(accent_fg)
                                    .child(avatar.clone())
                                    .into_any_element()
                            }
                        };
                        match feed_host(feed.site_url.as_deref(), &feed.feed_url)
                            .and_then(|host| local_favicon_path(&self.icons_dir, &host))
                        {
                            Some(path) => {
                                let loading_avatar = avatar.clone();
                                img(path)
                                    .id(("feed-icon", id as u64))
                                    .size(px(20.))
                                    .rounded(px(4.))
                                    .overflow_hidden()
                                    .object_fit(ObjectFit::Cover)
                                    .with_fallback(letter)
                                    .with_loading(move || {
                                        div()
                                            .size(px(20.))
                                            .rounded(px(4.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .bg(accent_bg)
                                            .text_xs()
                                            .font_semibold()
                                            .text_color(accent_fg)
                                            .child(loading_avatar.clone())
                                            .into_any_element()
                                    })
                                    .into_any_element()
                            }
                            None => letter(),
                        }
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .overflow_hidden()
                                    .text_sm()
                                    .truncate()
                                    .child(feed.title.clone()),
                            )
                            .when_some(feed.last_error.clone(), |row, error| {
                                row.child(
                                    div()
                                        .overflow_hidden()
                                        .text_xs()
                                        .truncate()
                                        .text_color(cx.theme().danger)
                                        .child(preview_text(&error, 42)),
                                )
                            }),
                    )
                    .when(feed.unread > 0, |row| {
                        row.child(div().text_xs().child(feed.unread.to_string()))
                    })
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_scope(Scope::Feed(id), cx)),
                    )
                    .context_menu(move |menu, _, _| {
                        Self::feed_context_menu(menu, &menu_feed, &menu_app, language, local)
                    }),
            )
    }

    fn render_folder_row(
        &self,
        folder: &str,
        unread: i64,
        folder_collapsed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = matches!(&self.scope, Scope::Folder(name) if name == folder);
        let folder_for_scope = folder.to_owned();
        let folder_for_toggle = folder.to_owned();
        let folder_for_drop = folder.to_owned();
        let folder_for_menu = folder.to_owned();
        let selected_icon = self.folder_icon_key(folder).to_owned();
        let folder_icon = Self::folder_icon_element(self.folder_icon_key(folder));
        let folder_key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            folder.hash(&mut hasher);
            hasher.finish()
        };
        let menu_app = cx.entity().downgrade();
        let language = self.preferences.language;
        h_flex()
            .id(("folder-row", folder_key))
            .w_full()
            .h(px(30.))
            .items_center()
            .rounded(cx.theme().radius)
            .bg(if selected {
                cx.theme().sidebar_accent
            } else {
                cx.theme().sidebar
            })
            .hover(|style| style.bg(cx.theme().sidebar_accent))
            .text_color(if selected {
                cx.theme().sidebar_accent_foreground
            } else {
                cx.theme().sidebar_foreground
            })
            .cursor_pointer()
            .on_drag(FolderDrag(folder.to_owned()), |drag, _, _, cx| {
                cx.new(|_| FolderDragPreview {
                    name: SharedString::from(drag.0.clone()),
                })
            })
            .can_drop(|drag, _, _| drag.is::<FolderDrag>())
            .on_drop(cx.listener(move |this, drag: &FolderDrag, _, cx| {
                this.reorder_folder_before(&drag.0, &folder_for_drop, cx);
            }))
            .context_menu(move |menu, window, cx| {
                Self::folder_context_menu(
                    menu,
                    window,
                    cx,
                    &folder_for_menu,
                    &menu_app,
                    language,
                    &selected_icon,
                )
            })
            .child(
                div()
                    .id(("folder-toggle", folder_key))
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_1()
                    .py_1()
                    .cursor_pointer()
                    .child(
                        Icon::new(if folder_collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .small(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_folder_collapsed(&folder_for_toggle, cx)
                    })),
            )
            .child(
                h_flex()
                    .id(("folder", folder_key))
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .pr_2()
                    .py_1()
                    .cursor_pointer()
                    .child(folder_icon)
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_sm()
                            .child(folder.to_owned()),
                    )
                    .when(unread > 0, |row| {
                        row.child(div().text_xs().child(unread.to_string()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_scope(Scope::Folder(folder_for_scope.clone()), cx)
                    })),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar_bg = cx.theme().sidebar;
        let sidebar_text = cx.theme().sidebar_foreground;
        let mut folders: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (index, feed) in self.feeds.iter().enumerate() {
            folders
                .entry(feed.folder.clone().unwrap_or_default())
                .or_default()
                .push(index);
        }
        for feeds in folders.values_mut() {
            feeds.sort_by_cached_key(|index| self.feeds[*index].title.to_lowercase());
        }
        let mut folder_names = folders
            .keys()
            .filter(|name| !name.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        folder_names.sort_by_key(|name| name.to_lowercase());
        let mut folder_order = self
            .preferences
            .folder_order
            .iter()
            .filter(|name| folder_names.iter().any(|folder| folder == *name))
            .cloned()
            .collect::<Vec<_>>();
        for name in &folder_names {
            if !folder_order.iter().any(|existing| existing == name) {
                folder_order.push(name.clone());
            }
        }

        let mut rows = Vec::with_capacity(self.feeds.len() + folder_order.len());
        if let Some(root_feeds) = folders.remove("") {
            rows.extend(root_feeds.into_iter().map(|feed_index| SidebarRow::Feed {
                feed_index,
                nested: false,
                has_error: self.feeds[feed_index].last_error.is_some(),
            }));
        }
        for folder in folder_order {
            let Some(feeds) = folders.remove(&folder) else {
                continue;
            };
            let collapsed = self
                .preferences
                .collapsed_folders
                .iter()
                .any(|name| name == &folder);
            let unread = feeds.iter().map(|index| self.feeds[*index].unread).sum();
            rows.push(SidebarRow::Folder {
                name: folder,
                unread,
                collapsed,
            });
            if !collapsed {
                rows.extend(feeds.into_iter().map(|feed_index| SidebarRow::Feed {
                    feed_index,
                    nested: true,
                    has_error: self.feeds[feed_index].last_error.is_some(),
                }));
            }
        }
        let item_sizes = Rc::new(
            rows.iter()
                .map(|row| size(px(228.), px(row.height() + 2.)))
                .collect::<Vec<_>>(),
        );
        let rows = Rc::new(rows);
        let scroll = self.feed_list_scroll.clone();
        let owner = cx.entity().clone();
        let feed_list = v_virtual_list(
            owner,
            "feed-list",
            item_sizes,
            move |this, range, _window, cx| {
                range
                    .map(|index| match &rows[index] {
                        SidebarRow::Feed {
                            feed_index, nested, ..
                        } => this
                            .feeds
                            .get(*feed_index)
                            .map(|feed| {
                                this.render_feed_row(feed, *nested, &this.scope, cx)
                                    .into_any_element()
                            })
                            .unwrap_or_else(|| div().into_any_element()),
                        SidebarRow::Folder {
                            name,
                            unread,
                            collapsed,
                        } => this
                            .render_folder_row(name, *unread, *collapsed, cx)
                            .into_any_element(),
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&scroll)
        .size_full();
        v_flex()
            .w(px(228.))
            .h_full()
            .font(sidebar_font())
            .bg(sidebar_bg)
            .text_color(sidebar_text)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(div().h(px(8.)))
            .child(
                v_flex()
                    .gap_0p5()
                    .px_2()
                    .child(self.scope_row(
                        "scope-all",
                        "All Articles",
                        IconName::BookOpen,
                        Scope::All,
                        cx,
                    ))
                    .child(self.scope_row(
                        "scope-unread",
                        "Unread",
                        IconName::Inbox,
                        Scope::Unread,
                        cx,
                    ))
                    .child(self.scope_row(
                        "scope-starred",
                        "Starred",
                        IconName::Star,
                        Scope::Starred,
                        cx,
                    ))
                    .child(self.scope_row(
                        "scope-later",
                        "Read Later",
                        IconName::Inbox,
                        Scope::Later,
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .pt_4()
                    .pb_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Feeds"))
                    .child(
                        Button::new("refresh")
                            .small()
                            .ghost()
                            .icon(tty_icon("refresh"))
                            .tooltip(self.t("Refresh feeds"))
                            .loading(self.is_refreshing)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .id("feed-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .px_2()
                    .child(feed_list)
                    .vertical_scrollbar(&scroll),
            )
    }

    fn scope_row(
        &self,
        id: &'static str,
        label: &'static str,
        icon: IconName,
        scope: Scope,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.scope == scope;
        let scope_for_click = scope.clone();
        h_flex()
            .id(id)
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().sidebar_accent))
            .bg(if selected {
                cx.theme().sidebar_accent
            } else {
                cx.theme().sidebar
            })
            .text_color(if selected {
                cx.theme().sidebar_accent_foreground
            } else {
                cx.theme().sidebar_foreground
            })
            .child(match &scope {
                Scope::All => tty_icon("list-flat").small(),
                Scope::Starred => tty_icon("star").small(),
                Scope::Later => tty_icon("bookmark").small(),
                Scope::Folder(_) => Icon::new(IconName::FolderClosed).small(),
                _ => Icon::new(icon).small(),
            })
            .child(div().flex_1().text_sm().child(self.t(label)))
            .on_click(
                cx.listener(move |this, _, _, cx| this.select_scope(scope_for_click.clone(), cx)),
            )
    }

    fn render_article_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let section_title = match &self.scope {
            Scope::All => self.t("All Articles").to_string(),
            Scope::Unread => self.t("Unread").to_string(),
            Scope::Starred => self.t("Starred").to_string(),
            Scope::Later => self.t("Read Later").to_string(),
            Scope::Folder(name) => name.clone(),
            Scope::Feed(id) => self
                .feeds
                .iter()
                .find(|feed| feed.id == *id)
                .map(|feed| feed.title.clone())
                .unwrap_or_else(|| self.t("Feed").into()),
        };
        let selected_article = self.article.as_ref().map(|article| article.summary.id);
        let articles = self.articles.clone();
        let view = cx.entity();
        let language = self.preferences.language;
        let mut list =
            v_flex()
                .w(px(340.))
                .h_full()
                .bg(cx.theme().colors.list)
                .border_r_1()
                .border_color(cx.theme().border)
                .child(
                    v_flex()
                        .gap_3()
                        .px_5()
                        .pt_5()
                        .pb_4()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .child(div().text_xl().font_semibold().child(section_title))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            Button::new("mark-all-read")
                                                .small()
                                                .ghost()
                                                .tooltip(self.t("Mark all as read"))
                                                .label(self.t("Read all"))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.mark_all_read(cx)
                                                })),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(i18n::format(
                                                    self.preferences.language,
                                                    "{} articles",
                                                    self.articles.len(),
                                                )),
                                        ),
                                ),
                        )
                        .child(text_input(&self.search_input)),
                );
        if self.is_loading {
            list = list.child(
                div()
                    .p_5()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("Loading articles…")),
            );
        } else if self.articles.is_empty() {
            list =
                list.child(
                    v_flex()
                        .items_center()
                        .gap_2()
                        .px_5()
                        .py_12()
                        .text_color(cx.theme().muted_foreground)
                        .child(div().text_2xl().child(IconName::Inbox))
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .child(self.t("No articles yet")),
                        )
                        .child(div().text_xs().child(self.t(
                            "Connect the selected provider in Settings to see your articles here",
                        ))),
                );
        } else {
            let scroll = self.article_list_scroll.clone();
            let can_load_more = self.can_load_more();
            list = list.child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(
                        uniform_list(
                            "article-list-scroll",
                            articles.len(),
                            move |range, window, cx| {
                                if can_load_more && range.end + 4 >= articles.len() {
                                    let owner = view.clone();
                                    window
                                        .spawn(cx, async move |cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                if this.can_load_more() {
                                                    this.load_more_articles(cx);
                                                }
                                            });
                                        })
                                        .detach();
                                }
                                range
                                    .map(|index| {
                                        let article = &articles[index];
                                        let id = article.id;
                                        let date = article
                                            .published_at
                                            .as_deref()
                                            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                                            .map(|d| d.format("%m-%d").to_string())
                                            .unwrap_or_default();
                                        let owner = view.clone();
                                        let menu_app = view.downgrade();
                                        let menu_article = article.clone();
                                        v_flex()
                                            .id(("article-row", id as u64))
                                            .h(px(80.))
                                            .w_full()
                                            .gap_1()
                                            .px_4()
                                            .py_2()
                                            .overflow_hidden()
                                            .cursor_pointer()
                                            .border_b_1()
                                            .border_color(cx.theme().border)
                                            .bg(if selected_article == Some(id) {
                                                cx.theme().list_active
                                            } else {
                                                cx.theme().colors.list
                                            })
                                            .hover(|style| style.bg(cx.theme().list_hover))
                                            .on_click(move |_, _, cx| {
                                                owner.update(cx, |this, cx| {
                                                    this.open_article(id, cx)
                                                });
                                            })
                                            .context_menu(move |menu, _, _| {
                                                Self::article_context_menu(
                                                    menu,
                                                    &menu_article,
                                                    &menu_app,
                                                    language,
                                                )
                                            })
                                            .child(
                                                h_flex()
                                                    .items_start()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .mt(px(6.))
                                                            .w(px(6.))
                                                            .h(px(6.))
                                                            .flex_shrink_0()
                                                            .rounded_full()
                                                            .bg(if article.is_read {
                                                                cx.theme().muted
                                                            } else {
                                                                cx.theme().primary
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .text_sm()
                                                            .font_semibold()
                                                            .line_clamp(2)
                                                            .child(article.title.clone()),
                                                    ),
                                            )
                                            .child(
                                                h_flex()
                                                    .justify_between()
                                                    .gap_2()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .overflow_hidden()
                                                            .truncate()
                                                            .child(article.feed_title.clone()),
                                                    )
                                                    .child(div().flex_shrink_0().child(date)),
                                            )
                                    })
                                    .collect::<Vec<_>>()
                            },
                        )
                        .flex_1()
                        .size_full()
                        .track_scroll(&scroll),
                    )
                    .vertical_scrollbar(&scroll),
            );
        }
        list
    }

    fn render_reader(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut reader = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);
        if let Some(article) = &self.article {
            let summary = &article.summary;
            let id = summary.id;
            let menu_app = cx.entity().downgrade();
            let menu_article = summary.clone();
            let language = self.preferences.language;
            let star_label = self.t(if summary.is_starred {
                "Remove star"
            } else {
                "Star"
            });
            let later_label = self.t(if summary.read_later {
                "Remove from Read Later"
            } else {
                "Read Later"
            });
            let next_starred = !summary.is_starred;
            let next_later = !summary.read_later;
            let starred = summary.is_starred;
            let read_later = summary.read_later;
            let mut toolbar = h_flex()
                .items_center()
                .gap_1()
                .px_5()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(preview_text(&summary.feed_title, 28)),
                )
                .child(
                    Button::new("toggle-star")
                        .small()
                        .ghost()
                        .icon(tty_icon(if starred { "star-fill" } else { "star" }))
                        .tooltip(star_label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mark(id, MarkField::Starred, next_starred, cx)
                        })),
                )
                .child(
                    Button::new("toggle-hide-images")
                        .small()
                        .ghost()
                        .icon(tty_icon(if self.preferences.hide_images {
                            "image-off"
                        } else {
                            "image"
                        }))
                        .tooltip(self.t(if self.preferences.hide_images {
                            "Show images"
                        } else {
                            "Hide images"
                        }))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_hide_images(cx))),
                )
                .child(
                    Button::new("toggle-later")
                        .small()
                        .ghost()
                        .icon(tty_icon(if read_later {
                            "bookmark-check"
                        } else {
                            "bookmark"
                        }))
                        .tooltip(later_label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mark(id, MarkField::Later, next_later, cx)
                        })),
                );
            if article.url.is_some() {
                let has_full_text = article.extracted_html.is_some();
                toolbar = toolbar.child(
                    Button::new("extract-fulltext")
                        .small()
                        .ghost()
                        .icon(IconName::FileText)
                        .loading(self.is_extracting)
                        .tooltip(self.t(if has_full_text {
                            "Re-extract full text"
                        } else {
                            "Extract full text"
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.extract(id, true, cx))),
                );
            }
            let translate_label = if self.showing_translation {
                self.t("Show original")
            } else if self.has_translation_for_ui(article) {
                self.t("Show translation")
            } else {
                self.t("Translate")
            };
            toolbar = toolbar.child(
                Button::new("translate-article")
                    .small()
                    .ghost()
                    .icon(IconName::Globe)
                    .tooltip(translate_label)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_or_translate(id, cx))),
            );
            if self.showing_translation {
                toolbar = toolbar.child(
                    Button::new("translation-layout")
                        .small()
                        .ghost()
                        .tooltip(self.t(self.preferences.translation_layout.label()))
                        .label(self.t(match self.preferences.translation_layout {
                            TranslationLayout::Immersive => "Bilingual",
                            TranslationLayout::Replaced => "Translated",
                        }))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_translation_layout(cx))),
                );
            }
            if let Some(url) = article.url.clone() {
                toolbar = toolbar.child(
                    Button::new("open-original")
                        .small()
                        .ghost()
                        .icon(IconName::ExternalLink)
                        .tooltip(self.t("Open original"))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            let published = summary
                .published_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.format("%Y-%m-%d").to_string());
            let mut meta = summary.feed_title.clone();
            if let Some(author) = summary
                .author
                .as_deref()
                .filter(|author| !author.is_empty())
            {
                meta = format!("{meta}  ·  {author}");
            }
            if let Some(published) = published {
                meta = format!("{meta}  ·  {published}");
            }
            let mut article_style = TextViewStyle::default();
            article_style.heading_base_font_size = px(18.);
            article_style = article_style.paragraph_gap(rems(1.35)).heading_font_size(
                |level, base| match level {
                    1 => base * 1.55,
                    2 => base * 1.35,
                    3 => base * 1.2,
                    _ => base,
                },
            );
            let heading = v_flex()
                .gap_2()
                .pt_4()
                .pb_5()
                .mb_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_3xl()
                        .font_semibold()
                        .font(article_font())
                        .child(self.display_title(article)),
                )
                .when_some(self.display_translated_title(article), |view, title| {
                    view.child(
                        div()
                            .text_xl()
                            .font(article_font())
                            .text_color(cx.theme().muted_foreground)
                            .child(title),
                    )
                })
                .child(
                    div()
                        .pt_1()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(meta),
                );
            reader = reader.child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .context_menu(move |menu, _, _| {
                        Self::article_context_menu(menu, &menu_article, &menu_app, language)
                    })
                    .child(toolbar)
                    .child(
                        v_flex()
                            .id("article-reader")
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            // gpui TextSelectionLayer updates on every MouseMove while
                            // `is_selecting` is true, without checking pressed_button. If
                            // mouse-up is lost (clickpad / focus), hover alone keeps
                            // extending the selection — end the gesture when the button
                            // is not down.
                            .on_mouse_move(|event, window, cx| {
                                if event.pressed_button != Some(MouseButton::Left) {
                                    if gpui_kit::base::TextSelection::touch_selection(window, cx)
                                        .is_none()
                                    {
                                        gpui_kit::base::TextSelection::end(window, cx);
                                    }
                                }
                            })
                            .on_mouse_up(MouseButton::Left, |_, window, cx| {
                                gpui_kit::base::TextSelection::end(window, cx);
                            })
                            .on_scroll_wheel(|_, window, cx| {
                                // Scroll wins over an in-progress text drag.
                                gpui_kit::base::TextSelection::clear(window, cx);
                            })
                            .child(
                                // Capture-phase guard so we end before the window
                                // selection layer extends on bubble MouseMove.
                                canvas(
                                    |_, _, _| (),
                                    |_, _, window, _| {
                                        window.on_mouse_event(
                                            |event: &MouseMoveEvent, phase, window, cx| {
                                                if phase != DispatchPhase::Capture {
                                                    return;
                                                }
                                                if event.pressed_button == Some(MouseButton::Left) {
                                                    return;
                                                }
                                                if gpui_kit::base::TextSelection::touch_selection(
                                                    window, cx,
                                                )
                                                .is_some()
                                                {
                                                    return;
                                                }
                                                gpui_kit::base::TextSelection::end(window, cx);
                                            },
                                        );
                                    },
                                )
                                .absolute()
                                .size(px(0.))
                                .top_0()
                                .left_0(),
                            )
                            .child(
                                v_flex()
                                    .size_full()
                                    .max_w(px(680.))
                                    .mx_auto()
                                    .px(px(48.))
                                    .child(heading)
                                    .child(
                                        div().flex_1().min_h_0().w_full().child(
                                            TextView::html(
                                                "article-body",
                                                self.article_body.clone(),
                                            )
                                            .scrollable(true)
                                            // Scrollbar is overlaid (absolute inset); keep
                                            // text clear of the ~16px thumb track.
                                            .pr(px(20.))
                                            .font(article_font())
                                            .style(article_style)
                                            .pb_12()
                                            .on_link_click(|url, _, _, cx| cx.open_url(url)),
                                        ),
                                    ),
                            ),
                    ),
            );
        } else {
            reader = reader.items_center().justify_center().child(
                v_flex()
                    .gap_3()
                    .items_center()
                    .child(div().text_base().font_semibold().child("Panda Reader"))
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .child(self.t("Start reading")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t("Select an article from the list to begin")),
                    ),
            );
        }
        reader
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mid = self.status.clone().unwrap_or_default();
        let mid_color = match self.status_tone {
            StatusTone::Error => cx.theme().danger,
            StatusTone::Busy | StatusTone::Flash => cx.theme().muted_foreground,
        };
        let right = if self.is_refreshing || self.is_connecting {
            self.t("Syncing…").to_owned()
        } else {
            self.upstream_provider_label()
                .unwrap_or_default()
                .to_owned()
        };
        h_flex()
            .w_full()
            .h(px(24.))
            .flex_shrink_0()
            .items_center()
            .gap_3()
            .px_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().title_bar)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(self.status_context_label()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_center()
                    .text_color(mid_color)
                    .child(mid),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_right()
                    .child(right),
            )
    }
}

impl Render for ReaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(px(self.preferences.ui_font_size));
        let body = if self.settings_open {
            self.render_settings(cx).into_any_element()
        } else {
            h_flex()
                .size_full()
                .items_stretch()
                .when(!self.preferences.sidebar_collapsed, |view| {
                    view.child(self.render_sidebar(cx))
                })
                .child(self.render_article_list(cx))
                .child(self.render_reader(cx))
                .into_any_element()
        };
        div()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .key_context("Reader")
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font(app_ui_font())
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| {
                this.toggle_palette(window, cx);
            }))
            .on_action(cx.listener(|this, _: &RefreshFeeds, window, cx| {
                this.handle_chord(CommandKind::Refresh, window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.handle_chord(CommandKind::OpenSettings, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.handle_chord(CommandKind::FocusSearch, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                this.handle_chord(CommandKind::ToggleSidebar, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextArticle, window, cx| {
                this.handle_chord(CommandKind::NextArticle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousArticle, window, cx| {
                this.handle_chord(CommandKind::PreviousArticle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowKeyboardShortcuts, window, cx| {
                this.handle_chord(CommandKind::ShowKeyboardShortcuts, window, cx);
            }))
            .on_action(cx.listener(|this, _: &LoadMoreArticles, window, cx| {
                this.handle_chord(CommandKind::LoadMoreArticles, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimNextArticle, window, cx| {
                this.handle_vim(CommandKind::NextArticle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimPreviousArticle, window, cx| {
                this.handle_vim(CommandKind::PreviousArticle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimNextUnread, window, cx| {
                this.handle_vim(CommandKind::NextUnread, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimPreviousUnread, window, cx| {
                this.handle_vim(CommandKind::PreviousUnread, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimToggleRead, window, cx| {
                this.handle_vim(CommandKind::ToggleRead, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimToggleStar, window, cx| {
                this.handle_vim(CommandKind::ToggleStar, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimToggleLater, window, cx| {
                this.handle_vim(CommandKind::ToggleLater, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimOpenOriginal, window, cx| {
                this.handle_vim(CommandKind::OpenOriginal, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimMarkAllRead, window, cx| {
                this.handle_vim(CommandKind::MarkAllRead, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimRefresh, window, cx| {
                this.handle_vim(CommandKind::Refresh, window, cx);
            }))
            .on_action(cx.listener(|this, _: &VimShowHelp, window, cx| {
                this.handle_vim(CommandKind::ShowKeyboardShortcuts, window, cx);
            }))
            .child(
                v_flex()
                    .size_full()
                    .child(self.render_title_bar(window, cx))
                    .child(div().flex_1().min_h_0().child(body))
                    .child(self.render_status_bar(cx)),
            )
            .when(self.palette.open, |view| {
                view.child(self.render_palette(cx))
            })
            .when(self.editing_feed.is_some(), |view| {
                view.child(self.render_feed_editor(cx))
            })
    }
}

impl Focusable for ReaderView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

use chrono::DateTime;

pub(crate) fn sidebar_font() -> Font {
    let mut face = font("Inter");
    face.fallbacks = Some(chinese_sans_fallbacks());
    face
}

fn app_ui_font() -> Font {
    let mut face = font("Source Sans 3");
    face.fallbacks = Some(chinese_sans_fallbacks());
    face
}

fn article_font() -> Font {
    let mut face = font("Source Serif 4");
    face.fallbacks = Some(chinese_sans_fallbacks());
    face
}

fn chinese_sans_fallbacks() -> FontFallbacks {
    FontFallbacks::from_fonts(vec![
        "Noto Sans SC".into(),
        "Microsoft YaHei UI".into(),
        "PingFang SC".into(),
        "Segoe UI".into(),
    ])
}
