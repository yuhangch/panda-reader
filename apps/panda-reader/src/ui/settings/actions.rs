use crate::app::preferences::{ArticleNetworkMode, Language, Preferences};
use crate::services::ArticleFetchNetwork;
use crate::services::Command;
use crate::ui::i18n;
use crate::ui::theme;
use crate::ui::window::ReaderWindow;
use crate::updater::{self, UpdateStatus};
use gpui_kit::component::{IndexPath, select::SearchableVec};
use gpui_kit::*;
use panda_core::Scope;
use panda_translate::TranslatorConfig;
use tokio::sync::oneshot;

use super::selectors::{
    ArticleNetworkModeOption, LanguageOption, LogRetentionOption, TranslatorProvider,
};

#[derive(Clone, Copy)]
pub(super) enum ReaderSetting {
    FontSize,
    LineHeight,
    ContentWidth,
    ParagraphSpacing,
}

impl ReaderWindow {
    pub(in crate::ui) fn set_article_network_mode(
        &mut self,
        mode: ArticleNetworkMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let network = ArticleFetchNetwork {
            mode,
            proxy_url: self.preferences.article_proxy_url.clone(),
            trusted_fake_ip_ranges: self.preferences.trusted_proxy_fake_ip_ranges.clone(),
        };
        if let Err(error) = network.validate() {
            self.set_error(error.to_string());
            let selected = IndexPath::new(ArticleNetworkModeOption::index(
                self.preferences.article_network_mode,
            ));
            self.settings
                .article_network_mode_select
                .update(cx, |select, cx| {
                    select.set_selected_index(Some(selected), window, cx)
                });
            cx.notify();
            return;
        }
        self.preferences.article_network_mode = mode;
        self.services.set_article_fetch_network(network);
        self.save_preferences();
        cx.notify();
    }

    pub(in crate::ui) fn save_article_network_settings(&mut self, cx: &mut Context<Self>) {
        let proxy_url = self
            .settings
            .article_proxy_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let fake_ranges = self
            .settings
            .trusted_fake_ip_ranges_input
            .read(cx)
            .value()
            .to_string();
        let network = ArticleFetchNetwork {
            mode: self.preferences.article_network_mode,
            proxy_url: proxy_url.clone(),
            trusted_fake_ip_ranges: fake_ranges.clone(),
        };
        if let Err(error) = network.validate() {
            self.set_error(error.to_string());
            cx.notify();
            return;
        }
        self.preferences.article_proxy_url = proxy_url;
        self.preferences.trusted_proxy_fake_ip_ranges = fake_ranges;
        self.services.set_article_fetch_network(network);
        self.save_preferences();
        self.set_flash(self.t("Article network settings saved"), cx);
        cx.notify();
    }

    pub(super) fn adjust_reader_setting(
        &mut self,
        setting: ReaderSetting,
        delta: f32,
        cx: &mut Context<Self>,
    ) {
        match setting {
            ReaderSetting::FontSize => {
                self.preferences.reader_font_size =
                    (self.preferences.reader_font_size + delta).clamp(14., 30.);
            }
            ReaderSetting::LineHeight => {
                self.preferences.reader_line_height =
                    (self.preferences.reader_line_height + delta).clamp(1.2, 2.2);
            }
            ReaderSetting::ContentWidth => {
                self.preferences.reader_content_width =
                    (self.preferences.reader_content_width + delta).clamp(520., 1000.);
            }
            ReaderSetting::ParagraphSpacing => {
                self.preferences.reader_paragraph_spacing =
                    (self.preferences.reader_paragraph_spacing + delta).clamp(0.5, 2.5);
            }
        }
        self.save_preferences();
        cx.notify();
    }

    pub(in crate::ui) fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        self.update_status = UpdateStatus::Checking;
        updater::start_check(self.data_dir.clone(), self.update_events.clone());
        self.settings.check_community_plugins(cx);
        cx.notify();
    }

    pub(in crate::ui) fn cleanup_update_downloads(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.update_status,
            UpdateStatus::Checking | UpdateStatus::Downloading { .. }
        ) {
            self.set_flash(self.t("Wait for the update download to finish"), cx);
            cx.notify();
            return;
        }
        let keep = match &self.update_status {
            UpdateStatus::Ready(update)
            | UpdateStatus::NextLaunch(update)
            | UpdateStatus::Later(update) => Some(update.package.as_path()),
            _ => None,
        };
        match updater::cleanup_download_cache(&self.data_dir, keep) {
            Ok(0) => self.set_flash(self.t("No old update downloads to remove"), cx),
            Ok(count) => self.set_flash(
                i18n::format(
                    self.preferences.language,
                    "Removed {} old update download(s)",
                    &count.to_string(),
                ),
                cx,
            ),
            Err(error) => self.set_error(error),
        }
        cx.notify();
    }

    pub(in crate::ui) fn set_update_checks(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.preferences.check_for_updates = enabled;
        self.save_preferences();
        if enabled {
            self.check_for_updates(cx);
        } else {
            cx.notify();
        }
    }

    pub(in crate::ui) fn update_now(&mut self, cx: &mut Context<Self>) {
        let UpdateStatus::Ready(update) = &self.update_status else {
            return;
        };
        match updater::launch_now(&self.data_dir, update) {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                self.update_status = UpdateStatus::Failed(error.clone());
                self.set_error(error);
                cx.notify();
            }
        }
    }

    pub(in crate::ui) fn update_next_launch(&mut self, cx: &mut Context<Self>) {
        let UpdateStatus::Ready(update) = &self.update_status else {
            return;
        };
        match updater::save_for_next_launch(&self.data_dir, update) {
            Ok(()) => {
                self.update_status = UpdateStatus::NextLaunch(update.clone());
                self.set_flash(
                    self.t("Update will install next time you start Panda Reader."),
                    cx,
                );
                cx.notify();
            }
            Err(error) => {
                self.update_status = UpdateStatus::Failed(error.clone());
                self.set_error(error);
                cx.notify();
            }
        }
    }

    pub(in crate::ui) fn update_later(&mut self, cx: &mut Context<Self>) {
        if let UpdateStatus::Ready(update) = &self.update_status {
            self.update_status = UpdateStatus::Later(update.clone());
            cx.notify();
        }
    }

    pub(in crate::ui) fn save_preferences(&mut self) {
        if let Err(error) = self.preferences.save(&self.preferences_path) {
            self.set_error(format!("Could not save settings: {error}"));
        }
    }

    pub(in crate::ui) fn save_translator_config(&mut self) {
        if let Err(error) = self.translator_config.save(&self.translator_path) {
            self.set_error(format!("Could not save settings: {error}"));
        }
    }

    pub(in crate::ui) fn reset_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preferences = Preferences::default();
        self.services.set_detailed_sync_logging(false);
        self.services
            .set_article_fetch_network(ArticleFetchNetwork {
                mode: self.preferences.article_network_mode,
                proxy_url: self.preferences.article_proxy_url.clone(),
                trusted_fake_ip_ranges: self.preferences.trusted_proxy_fake_ip_ranges.clone(),
            });
        self.services
            .set_log_retention_days(self.preferences.sync_log_retention_days);
        self.settings.library_source = self.preferences.library_source;
        self.translator_config = TranslatorConfig::default();
        self.settings.pending_reset_settings = false;

        self.settings.language_select.update(cx, |select, cx| {
            select.set_selected_index(
                Some(IndexPath::new(self.preferences.language.index())),
                window,
                cx,
            )
        });
        self.settings
            .translation_language_select
            .update(cx, |select, cx| {
                select.set_selected_index(
                    Some(IndexPath::new(
                        self.preferences.translation_language.index(),
                    )),
                    window,
                    cx,
                )
            });
        self.settings.provider_select.update(cx, |select, cx| {
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
        self.settings.upstream_select.update(cx, |select, cx| {
            select.set_selected_index(
                Some(IndexPath::new(self.settings.library_source.index())),
                window,
                cx,
            )
        });
        self.settings.log_retention_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(LogRetentionOption::all(self.preferences.language)),
                window,
                cx,
            );
            select.set_selected_index(
                Some(IndexPath::new(LogRetentionOption::index(
                    self.preferences.sync_log_retention_days,
                ))),
                window,
                cx,
            );
        });
        let reset_language = self.preferences.language;
        let reset_network_mode = self.preferences.article_network_mode;
        self.settings
            .article_network_mode_select
            .update(cx, |select, cx| {
                select.set_items(
                    SearchableVec::new(ArticleNetworkModeOption::all(reset_language)),
                    window,
                    cx,
                );
                select.set_selected_index(
                    Some(IndexPath::new(ArticleNetworkModeOption::index(
                        reset_network_mode,
                    ))),
                    window,
                    cx,
                );
            });
        let proxy_url = self.preferences.article_proxy_url.clone();
        self.settings
            .article_proxy_input
            .update(cx, |input, cx| input.set_value(&proxy_url, window, cx));
        let fake_ranges = self.preferences.trusted_proxy_fake_ip_ranges.clone();
        self.settings
            .trusted_fake_ip_ranges_input
            .update(cx, |input, cx| input.set_value(&fake_ranges, window, cx));

        self.settings.add_input.update(cx, |input, cx| {
            input.set_placeholder(
                i18n::text(self.preferences.language, "Feed URL https://…"),
                window,
                cx,
            )
        });
        let search_input = self.list.read(cx).search_input.clone();
        search_input.update(cx, |input, cx| {
            input.set_placeholder(
                i18n::text(self.preferences.language, "Search articles"),
                window,
                cx,
            )
        });
        let share_template = self.preferences.share_template.clone();
        self.settings
            .share_template_input
            .update(cx, |input, cx| input.set_value(&share_template, window, cx));
        self.settings
            .azure_key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.settings
            .azure_region_input
            .update(cx, |input, cx| input.set_value("eastasia", window, cx));
        self.settings
            .volcengine_ak_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.settings
            .volcengine_sk_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.settings.openai_url_input.update(cx, |input, cx| {
            input.set_value("https://api.openai.com/v1", window, cx)
        });
        self.settings
            .openai_key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.settings
            .openai_model_input
            .update(cx, |input, cx| input.set_value("gpt-4.1-mini", window, cx));
        self.settings.anthropic_url_input.update(cx, |input, cx| {
            input.set_value("https://api.anthropic.com/v1", window, cx)
        });
        self.settings
            .anthropic_key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.settings.anthropic_model_input.update(cx, |input, cx| {
            input.set_value("claude-haiku-4-5-20251001", window, cx)
        });
        self.settings.gemini_url_input.update(cx, |input, cx| {
            input.set_value(
                "https://generativelanguage.googleapis.com/v1beta",
                window,
                cx,
            )
        });
        self.settings
            .gemini_key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.settings.gemini_model_input.update(cx, |input, cx| {
            input.set_value("gemini-2.5-flash", window, cx)
        });

        theme::apply_theme(&self.preferences.theme, cx);
        self.services
            .set_workspace(self.settings.library_source.workspace());
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

    pub(in crate::ui) fn persist_translator_settings(&mut self, cx: &mut Context<Self>) {
        self.translator_config.azure_key =
            self.settings.azure_key_input.read(cx).value().to_string();
        self.translator_config.azure_region = self
            .settings
            .azure_region_input
            .read(cx)
            .value()
            .to_string();
        self.translator_config.volcengine_access_key = self
            .settings
            .volcengine_ak_input
            .read(cx)
            .value()
            .to_string();
        self.translator_config.volcengine_secret_key = self
            .settings
            .volcengine_sk_input
            .read(cx)
            .value()
            .to_string();
        self.translator_config.openai_url =
            self.settings.openai_url_input.read(cx).value().to_string();
        self.translator_config.openai_key =
            self.settings.openai_key_input.read(cx).value().to_string();
        self.translator_config.openai_model = self
            .settings
            .openai_model_input
            .read(cx)
            .value()
            .to_string();
        self.translator_config.anthropic_url = self
            .settings
            .anthropic_url_input
            .read(cx)
            .value()
            .to_string();
        self.translator_config.anthropic_key = self
            .settings
            .anthropic_key_input
            .read(cx)
            .value()
            .to_string();
        self.translator_config.anthropic_model = self
            .settings
            .anthropic_model_input
            .read(cx)
            .value()
            .to_string();
        self.translator_config.gemini_url =
            self.settings.gemini_url_input.read(cx).value().to_string();
        self.translator_config.gemini_key =
            self.settings.gemini_key_input.read(cx).value().to_string();
        self.translator_config.gemini_model = self
            .settings
            .gemini_model_input
            .read(cx)
            .value()
            .to_string();
        self.save_translator_config();
        self.set_flash(self.t("Translation settings saved"), cx);
        cx.notify();
    }

    pub(in crate::ui) fn set_language(
        &mut self,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preferences.language = language;
        self.clear_status();
        self.settings.add_input.update(cx, |input, cx| {
            input.set_placeholder(i18n::text(language, "Feed URL https://…"), window, cx)
        });
        let search_input = self.list.read(cx).search_input.clone();
        search_input.update(cx, |input, cx| {
            input.set_placeholder(i18n::text(language, "Search articles"), window, cx)
        });
        self.settings
            .added_feeds_search_input
            .update(cx, |input, cx| {
                input.set_placeholder(i18n::text(language, "Search added feeds"), window, cx)
            });
        let language_index = IndexPath::new(language.index());
        self.settings.language_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(LanguageOption::all(language)),
                window,
                cx,
            );
            select.set_selected_index(Some(language_index), window, cx);
        });
        let translation_language_index =
            IndexPath::new(self.preferences.translation_language.index());
        self.settings
            .translation_language_select
            .update(cx, |select, cx| {
                select.set_items(
                    SearchableVec::new(LanguageOption::all(language)),
                    window,
                    cx,
                );
                select.set_selected_index(Some(translation_language_index), window, cx);
            });
        let selected = IndexPath::new(self.translator_config.provider.index());
        self.settings.provider_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(TranslatorProvider::all(language)),
                window,
                cx,
            );
            select.set_selected_index(Some(selected), window, cx);
        });
        let retention_index = IndexPath::new(LogRetentionOption::index(
            self.preferences.sync_log_retention_days,
        ));
        self.settings.log_retention_select.update(cx, |select, cx| {
            select.set_items(
                SearchableVec::new(LogRetentionOption::all(language)),
                window,
                cx,
            );
            select.set_selected_index(Some(retention_index), window, cx);
        });
        let network_mode_index = IndexPath::new(ArticleNetworkModeOption::index(
            self.preferences.article_network_mode,
        ));
        self.settings
            .article_network_mode_select
            .update(cx, |select, cx| {
                select.set_items(
                    SearchableVec::new(ArticleNetworkModeOption::all(language)),
                    window,
                    cx,
                );
                select.set_selected_index(Some(network_mode_index), window, cx);
            });
        self.save_preferences();
        cx.notify();
    }

    pub(in crate::ui) fn set_translation_language(
        &mut self,
        language: Language,
        cx: &mut Context<Self>,
    ) {
        self.preferences.translation_language = language;
        let loaded = self.list.read(cx).articles.as_ref().clone();
        if self.read_article_reader(cx, |reader| reader.showing_translation) {
            self.update_article_reader(cx, |reader| reader.showing_translation = false);
            self.request_prepared_body(cx);
        }
        self.save_preferences();
        self.queue_title_translations(&loaded, cx);
        cx.notify();
    }

    pub(in crate::ui) fn set_auto_translate_titles(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.preferences.auto_translate_titles = enabled;
        self.save_preferences();
        if enabled {
            let rows = self.list.read(cx).articles.as_ref().clone();
            self.queue_title_translations(&rows, cx);
        }
        cx.notify();
    }

    pub(in crate::ui) fn set_only_translate_future_titles(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.preferences.only_translate_future_titles = enabled;
        self.save_preferences();
        if self.preferences.auto_translate_titles && enabled {
            self.reset_title_translation_cutoffs(cx);
        } else if self.preferences.auto_translate_titles {
            let rows = self.list.read(cx).articles.as_ref().clone();
            self.queue_title_translations(&rows, cx);
        }
        cx.notify();
    }

    fn reset_title_translation_cutoffs(&mut self, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.services
            .send(Command::ResetTitleTranslationCutoffs { reply });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".into()));
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    if this.preferences.auto_translate_titles
                        && this.preferences.only_translate_future_titles
                    {
                        this.load_snapshot(cx);
                    }
                    cx.notify();
                }
                Err(error) => this.set_error(error),
            });
        })
        .detach();
    }

    pub(in crate::ui) fn choose_theme(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.preferences.theme = id.into();
        theme::apply_theme(id, cx);
        self.save_preferences();
        cx.notify();
    }

    pub(in crate::ui) fn set_ui_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.preferences.ui_font_size = size;
        self.save_preferences();
        cx.notify();
    }
}
