use crate::app::preferences::{LibrarySource, Preferences};
use crate::services::AppServices;
use crate::ui::i18n;
use crate::ui::window::ReaderWindow;
use gpui_kit::component::{
    IndexPath,
    input::{InputEvent, InputState},
    select::{SearchableVec, SelectEvent, SelectState},
};
use gpui_kit::*;
use panda_core::Scope;
use panda_plugins::{CommunityPlugin, PluginSummary};
use panda_providers::ProviderSettingsMap;
use panda_translate::TranslatorConfig;
use std::sync::Arc;

use super::selectors::{LanguageOption, LogRetentionOption, SettingsPage, TranslatorProvider};

pub(in crate::ui) struct Settings {
    _subscriptions: Vec<Subscription>,
    pub(in crate::ui) add_input: Entity<InputState>,
    pub(in crate::ui) provider_url_input: Entity<InputState>,
    pub(in crate::ui) provider_username_input: Entity<InputState>,
    pub(in crate::ui) provider_secret_input: Entity<InputState>,
    pub(in crate::ui) added_feeds_search_input: Entity<InputState>,
    pub(in crate::ui) share_template_input: Entity<InputState>,
    pub(in crate::ui) language_select: Entity<SelectState<SearchableVec<LanguageOption>>>,
    pub(in crate::ui) translation_language_select:
        Entity<SelectState<SearchableVec<LanguageOption>>>,
    pub(in crate::ui) provider_select: Entity<SelectState<SearchableVec<TranslatorProvider>>>,
    pub(in crate::ui) log_retention_select: Entity<SelectState<SearchableVec<LogRetentionOption>>>,
    pub(in crate::ui) upstream_select: Entity<SelectState<SearchableVec<LibrarySource>>>,
    pub(in crate::ui) azure_key_input: Entity<InputState>,
    pub(in crate::ui) azure_region_input: Entity<InputState>,
    pub(in crate::ui) volcengine_ak_input: Entity<InputState>,
    pub(in crate::ui) volcengine_sk_input: Entity<InputState>,
    pub(in crate::ui) plugin_path_input: Entity<InputState>,
    pub(in crate::ui) plugins: Vec<PluginSummary>,
    pub(in crate::ui) plugins_checked: bool,
    pub(in crate::ui) plugin_list_loading: bool,
    pub(in crate::ui) community_plugins: Vec<CommunityPlugin>,
    pub(in crate::ui) plugins_loading: bool,
    pub(in crate::ui) community_plugins_loading: bool,
    pub(in crate::ui) community_plugins_checked: bool,
    pub(in crate::ui) plugin_error: Option<String>,
    pub(in crate::ui) community_plugin_error: Option<String>,
    pub(in crate::ui) services: AppServices,
    pub(in crate::ui) added_feeds_search: String,
    pub(in crate::ui) is_connecting: bool,
    pub(in crate::ui) provider_settings: ProviderSettingsMap,
    pub(in crate::ui) library_source: LibrarySource,
    pub(in crate::ui) open: bool,
    pub(in crate::ui) page: SettingsPage,
    pub(in crate::ui) pending_reset_settings: bool,
    pub(in crate::ui) theme_picker_open: bool,
}

impl Settings {
    pub(in crate::ui) fn new(
        window: &mut Window,
        cx: &mut Context<ReaderWindow>,
        preferences: &Preferences,
        translator_config: &TranslatorConfig,
        provider_settings: ProviderSettingsMap,
        services: AppServices,
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
        let added_feeds_search_placeholder = i18n::text(preferences.language, "Search added feeds");
        let added_feeds_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(added_feeds_search_placeholder));
        let share_template_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(preferences.share_template.as_str())
        });
        let language_index = IndexPath::new(preferences.language.index());
        let language_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(LanguageOption::all(preferences.language)),
                Some(language_index),
                window,
                cx,
            )
        });
        let translation_language_index = IndexPath::new(preferences.translation_language.index());
        let translation_language_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(LanguageOption::all(preferences.language)),
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
        let log_retention_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(LogRetentionOption::all(preferences.language)),
                Some(IndexPath::new(LogRetentionOption::index(
                    preferences.sync_log_retention_days,
                ))),
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
        let plugin_path_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Plugin folder, ZIP path, or HTTPS ZIP URL")
        });
        let added_feeds_search_for_events = added_feeds_search_input.clone();
        let mut _subscriptions = Vec::new();
        _subscriptions.push(cx.subscribe_in(
            &added_feeds_search_input,
            window,
            move |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.settings.added_feeds_search =
                        added_feeds_search_for_events.read(cx).value().to_string();
                    cx.notify();
                }
            },
        ));
        let share_template_for_events = share_template_input.clone();
        _subscriptions.push(cx.subscribe_in(
            &share_template_input,
            window,
            move |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let template = share_template_for_events.read(cx).value().to_string();
                    if this.preferences.share_template != template {
                        this.preferences.share_template = template;
                        this.save_preferences();
                    }
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
            &log_retention_select,
            window,
            |this, _, event, _, cx| {
                if let SelectEvent::Confirm(Some(days)) = event {
                    this.preferences.sync_log_retention_days = *days;
                    this.services.set_log_retention_days(*days);
                    this.save_preferences();
                    cx.notify();
                }
            },
        ));
        _subscriptions.push(cx.subscribe_in(
            &upstream_select,
            window,
            |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(kind)) = event {
                    this.settings.library_source = *kind;
                    this.preferences.library_source = *kind;
                    this.save_preferences();
                    this.services.set_workspace(kind.workspace());
                    let settings = kind
                        .provider()
                        .and_then(|provider| this.settings.provider_settings.get(&provider));
                    this.settings.provider_url_input.update(cx, |input, cx| {
                        input.set_value(
                            settings.map_or("", |value| value.endpoint.as_str()),
                            window,
                            cx,
                        )
                    });
                    this.settings
                        .provider_username_input
                        .update(cx, |input, cx| {
                            input.set_value(
                                settings.map_or("", |value| value.username.as_str()),
                                window,
                                cx,
                            )
                        });
                    this.settings.provider_secret_input.update(cx, |input, cx| {
                        input.set_value(
                            settings.map_or("", |value| value.secret.as_str()),
                            window,
                            cx,
                        )
                    });
                    this.list.scope = Scope::All;
                    this.reader.request_epoch.next();
                    this.reader.is_extracting = false;
                    this.reader.article = None;
                    this.reader.body_html = SharedString::default();
                    this.reader.body_markdown = SharedString::default();
                    this.reader.showing_translation = false;
                    this.list.articles = Arc::new(Vec::new());
                    this.sidebar.feeds.clear();
                    this.load_snapshot(cx);
                    cx.notify();
                }
            },
        ));
        Self {
            add_input,
            provider_url_input,
            provider_username_input,
            provider_secret_input,
            added_feeds_search_input,
            share_template_input,
            language_select,
            translation_language_select,
            provider_select,
            log_retention_select,
            upstream_select,
            azure_key_input,
            azure_region_input,
            volcengine_ak_input,
            volcengine_sk_input,
            plugin_path_input,
            plugins: Vec::new(),
            plugins_checked: false,
            plugin_list_loading: false,
            community_plugins: Vec::new(),
            plugins_loading: false,
            community_plugins_loading: false,
            community_plugins_checked: false,
            plugin_error: None,
            community_plugin_error: None,
            services,
            added_feeds_search: String::new(),
            is_connecting: false,
            provider_settings,
            library_source,
            open: false,
            page: SettingsPage::General,
            pending_reset_settings: false,
            theme_picker_open: false,
            _subscriptions,
        }
    }
}
