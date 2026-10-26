use crate::app::preferences::{LibrarySource, Preferences};
use crate::services::AppServices;
use crate::ui::i18n;
use crate::ui::window::ReaderWindow;
use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::component::{
    IndexPath,
    input::{InputEvent, InputState, TextareaState},
    select::{SearchableVec, SelectEvent, SelectState},
};
use gpui_kit::*;
use panda_core::Scope;
use panda_plugins::{CommunityPlugin, PluginSummary};
use panda_providers::ProviderSettingsMap;
use panda_translate::TranslatorConfig;
use std::{path::PathBuf, sync::Arc};

use super::selectors::{
    ArticleNetworkModeOption, BackupModeOption, BackupTargetOption, LanguageOption,
    LogRetentionOption, SettingsPage, TranslatorProvider,
};

pub(in crate::ui) struct Settings {
    _subscriptions: Vec<Subscription>,
    pub(in crate::ui) add_input: Entity<InputState>,
    pub(in crate::ui) provider_url_input: Entity<InputState>,
    pub(in crate::ui) provider_username_input: Entity<InputState>,
    pub(in crate::ui) provider_secret_input: Entity<InputState>,
    pub(in crate::ui) added_feeds_search_input: Entity<InputState>,
    pub(in crate::ui) share_template_input: Entity<InputState>,
    pub(in crate::ui) language_select: Entity<SelectState<SearchableVec<LanguageOption>>>,
    pub(in crate::ui) backup_mode_select: Entity<SelectState<SearchableVec<BackupModeOption>>>,
    pub(in crate::ui) backup_target_select: Entity<SelectState<SearchableVec<BackupTargetOption>>>,
    pub(in crate::ui) translation_language_select:
        Entity<SelectState<SearchableVec<LanguageOption>>>,
    pub(in crate::ui) provider_select: Entity<SelectState<SearchableVec<TranslatorProvider>>>,
    pub(in crate::ui) log_retention_select: Entity<SelectState<SearchableVec<LogRetentionOption>>>,
    pub(in crate::ui) article_network_mode_select:
        Entity<SelectState<SearchableVec<ArticleNetworkModeOption>>>,
    pub(in crate::ui) article_proxy_input: Entity<InputState>,
    pub(in crate::ui) trusted_fake_ip_ranges_input: Entity<TextareaState>,
    pub(in crate::ui) upstream_select: Entity<SelectState<SearchableVec<LibrarySource>>>,
    pub(in crate::ui) azure_key_input: Entity<InputState>,
    pub(in crate::ui) azure_region_input: Entity<InputState>,
    pub(in crate::ui) volcengine_ak_input: Entity<InputState>,
    pub(in crate::ui) volcengine_sk_input: Entity<InputState>,
    pub(in crate::ui) openai_url_input: Entity<InputState>,
    pub(in crate::ui) openai_key_input: Entity<InputState>,
    pub(in crate::ui) openai_model_input: Entity<InputState>,
    pub(in crate::ui) anthropic_url_input: Entity<InputState>,
    pub(in crate::ui) anthropic_key_input: Entity<InputState>,
    pub(in crate::ui) anthropic_model_input: Entity<InputState>,
    pub(in crate::ui) gemini_url_input: Entity<InputState>,
    pub(in crate::ui) gemini_key_input: Entity<InputState>,
    pub(in crate::ui) gemini_model_input: Entity<InputState>,
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
    pub(in crate::ui) added_feeds_scroll: VirtualListScrollHandle,
    pub(in crate::ui) settings_scroll: ScrollHandle,
    pub(in crate::ui) is_connecting: bool,
    pub(in crate::ui) provider_settings: ProviderSettingsMap,
    pub(in crate::ui) library_source: LibrarySource,
    pub(in crate::ui) open: bool,
    pub(in crate::ui) page: SettingsPage,
    pub(in crate::ui) pending_reset_settings: bool,
    pub(in crate::ui) theme_picker_open: bool,
    pub(in crate::ui) advanced_network_settings_open: bool,
    pub(in crate::ui) backup_full_mode: bool,
    pub(in crate::ui) backup_status: Option<String>,
    pub(in crate::ui) backup_encrypt_local: bool,
    pub(in crate::ui) backup_password_input: Entity<InputState>,
    pub(in crate::ui) backup_local_password_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_s3: bool,
    pub(in crate::ui) backup_cloud_endpoint_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_username_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_password_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_directory_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_region_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_bucket_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_prefix_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_access_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_secret_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_session_input: Entity<InputState>,
    pub(in crate::ui) backup_cloud_path_style: bool,
    pub(in crate::ui) backup_cloud_allow_insecure_http: bool,
    pub(in crate::ui) backup_cloud_credentials_saved: bool,
    pub(in crate::ui) backup_cloud_items: Vec<panda_backup::BackupItem>,
}

impl Settings {
    pub(in crate::ui) fn select_page(&mut self, page: SettingsPage) {
        self.page = page;
        self.settings_scroll = ScrollHandle::default();
    }

    pub(in crate::ui) fn new(
        window: &mut Window,
        cx: &mut Context<ReaderWindow>,
        preferences: &Preferences,
        translator_config: &TranslatorConfig,
        provider_settings: ProviderSettingsMap,
        services: AppServices,
        data_dir: PathBuf,
    ) -> Self {
        let library_source = preferences.library_source;
        let selected_settings = library_source
            .provider()
            .and_then(|kind| provider_settings.get(&kind));
        let add_placeholder = i18n::text(preferences.language, "Feed URL https://…");
        let add_input = cx.new(|cx| InputState::new(window, cx).placeholder(add_placeholder));
        let backup_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Backup password")
                .masked(true)
        });
        let backup_local_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Local backup password")
                .masked(true)
        });
        let saved_target = panda_backup::load_targets(&data_dir.join("backup-targets.json"))
            .ok()
            .and_then(|mut targets| {
                if targets.is_empty() {
                    None
                } else {
                    Some(targets.remove(0))
                }
            });
        let (
            backup_cloud_s3,
            cloud_endpoint,
            cloud_username,
            cloud_directory,
            cloud_region,
            cloud_bucket,
            cloud_prefix,
            cloud_path_style,
            cloud_allow_http,
            credentials_saved,
        ) = match saved_target {
            Some(panda_backup::BackupTargetConfig::WebDav(target)) => (
                false,
                target.endpoint,
                target.username,
                target.directory,
                String::new(),
                String::new(),
                String::new(),
                false,
                target.allow_insecure_http,
                !target.password.is_empty(),
            ),
            Some(panda_backup::BackupTargetConfig::S3(target)) => (
                true,
                target.endpoint.unwrap_or_default(),
                String::new(),
                String::new(),
                target.region,
                target.bucket,
                target.prefix,
                target.path_style,
                target.allow_insecure_http,
                !target.access_key.is_empty() || !target.secret_key.is_empty(),
            ),
            None => (
                false,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                true,
                false,
                false,
            ),
        };
        let backup_cloud_endpoint_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://storage.example")
                .default_value(&cloud_endpoint)
        });
        let backup_cloud_username_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Username")
                .default_value(&cloud_username)
        });
        let backup_cloud_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let backup_cloud_directory_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Remote directory")
                .default_value(&cloud_directory)
        });
        let backup_cloud_region_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Region")
                .default_value(&cloud_region)
        });
        let backup_cloud_bucket_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Bucket")
                .default_value(&cloud_bucket)
        });
        let backup_cloud_prefix_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Prefix")
                .default_value(&cloud_prefix)
        });
        let backup_cloud_access_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Access key")
                .masked(true)
        });
        let backup_cloud_secret_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Secret key")
                .masked(true)
        });
        let backup_cloud_session_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Session token (optional)")
                .masked(true)
        });
        let provider_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(library_source.endpoint_placeholder())
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
        let backup_mode_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(BackupModeOption::all(preferences.language)),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let backup_target_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(BackupTargetOption::all(preferences.language)),
                Some(IndexPath::new(usize::from(backup_cloud_s3))),
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
        let article_network_mode_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(ArticleNetworkModeOption::all(preferences.language)),
                Some(IndexPath::new(preferences.article_network_mode.index())),
                window,
                cx,
            )
        });
        let article_proxy_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("http://127.0.0.1:7890 or socks5h://127.0.0.1:1080")
                .default_value(preferences.article_proxy_url.as_str())
        });
        let trusted_fake_ip_ranges_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 4)
                .placeholder("One CIDR per line")
                .default_value(preferences.trusted_proxy_fake_ip_ranges.as_str())
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
        let openai_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.openai.com/v1")
                .default_value(translator_config.openai_url.as_str())
        });
        let openai_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("API key")
                .masked(true)
                .default_value(translator_config.openai_key.as_str())
        });
        let openai_model_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("gpt-4.1-mini")
                .default_value(translator_config.openai_model.as_str())
        });
        let anthropic_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.anthropic.com/v1")
                .default_value(translator_config.anthropic_url.as_str())
        });
        let anthropic_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("API key")
                .masked(true)
                .default_value(translator_config.anthropic_key.as_str())
        });
        let anthropic_model_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("claude-haiku-4-5-20251001")
                .default_value(translator_config.anthropic_model.as_str())
        });
        let gemini_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://generativelanguage.googleapis.com/v1beta")
                .default_value(translator_config.gemini_url.as_str())
        });
        let gemini_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("API key")
                .masked(true)
                .default_value(translator_config.gemini_key.as_str())
        });
        let gemini_model_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("gemini-2.5-flash")
                .default_value(translator_config.gemini_model.as_str())
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
                    this.settings
                        .added_feeds_scroll
                        .scroll_to_item(0, ScrollStrategy::Top);
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
            &backup_mode_select,
            window,
            |this, _, event, _, cx| {
                if let SelectEvent::Confirm(Some(option)) = event {
                    this.settings.backup_full_mode = option.mode == panda_backup::BackupMode::Full;
                    cx.notify();
                }
            },
        ));
        _subscriptions.push(cx.subscribe_in(
            &backup_target_select,
            window,
            |this, _, event, _, cx| {
                if let SelectEvent::Confirm(Some(option)) = event {
                    this.settings.backup_cloud_s3 = option.s3;
                    cx.notify();
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
            &article_network_mode_select,
            window,
            |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(option)) = event {
                    this.set_article_network_mode(option.mode, window, cx);
                }
            },
        ));
        _subscriptions.push(cx.subscribe_in(
            &upstream_select,
            window,
            |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(kind)) = event {
                    let (requested_article, translating_article) =
                        this.update_article_reader(cx, |reader| {
                            let requested_article = reader.requested_article_id.take();
                            let translating_article = reader.translating_article_id.take();
                            reader.translation_epoch.next();
                            (requested_article, translating_article)
                        });
                    if let Some(article_id) = requested_article {
                        this.services.cancel_article_work(article_id);
                    }
                    if let Some(article_id) = translating_article {
                        this.services.cancel_article_work(article_id);
                    }
                    this.settings.library_source = *kind;
                    this.preferences.library_source = *kind;
                    this.save_preferences();
                    this.services.set_workspace(kind.workspace());
                    let settings = kind
                        .provider()
                        .and_then(|provider| this.settings.provider_settings.get(&provider));
                    this.settings.provider_url_input.update(cx, |input, cx| {
                        input.set_placeholder(kind.endpoint_placeholder(), window, cx);
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
                    this.update_article_list(cx, |list, _| list.scope = Scope::All);
                    this.update_article_reader(cx, |reader| {
                        reader.request_epoch.next();
                        reader.is_extracting = false;
                        reader.translation_incomplete = false;
                        reader.article = None;
                        reader.body_html = SharedString::default();
                        reader.body_markdown = SharedString::default();
                        reader.showing_translation = false;
                    });
                    this.update_article_list(cx, |list, _| {
                        list.articles = Arc::new(Vec::new());
                    });
                    this.sidebar.update(cx, |sidebar, cx| {
                        sidebar.feeds.clear();
                        cx.notify();
                    });
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
            backup_mode_select,
            backup_target_select,
            translation_language_select,
            provider_select,
            log_retention_select,
            article_network_mode_select,
            article_proxy_input,
            trusted_fake_ip_ranges_input,
            upstream_select,
            azure_key_input,
            azure_region_input,
            volcengine_ak_input,
            volcengine_sk_input,
            openai_url_input,
            openai_key_input,
            openai_model_input,
            anthropic_url_input,
            anthropic_key_input,
            anthropic_model_input,
            gemini_url_input,
            gemini_key_input,
            gemini_model_input,
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
            added_feeds_scroll: VirtualListScrollHandle::new(),
            settings_scroll: ScrollHandle::default(),
            is_connecting: false,
            provider_settings,
            library_source,
            open: false,
            page: SettingsPage::General,
            pending_reset_settings: false,
            theme_picker_open: false,
            advanced_network_settings_open: false,
            backup_full_mode: false,
            backup_status: None,
            backup_encrypt_local: false,
            backup_password_input,
            backup_local_password_input,
            backup_cloud_s3,
            backup_cloud_endpoint_input,
            backup_cloud_username_input,
            backup_cloud_password_input,
            backup_cloud_directory_input,
            backup_cloud_region_input,
            backup_cloud_bucket_input,
            backup_cloud_prefix_input,
            backup_cloud_access_input,
            backup_cloud_secret_input,
            backup_cloud_session_input,
            backup_cloud_path_style: cloud_path_style,
            backup_cloud_allow_insecure_http: cloud_allow_http,
            backup_cloud_credentials_saved: credentials_saved,
            backup_cloud_items: Vec::new(),
            _subscriptions,
        }
    }
}
