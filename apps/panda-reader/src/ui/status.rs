use crate::ui::i18n;
use crate::ui::settings::SettingsPage;
use crate::ui::window::ReaderWindow;
use chrono::{DateTime, Local};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use panda_core::Scope;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum StatusTone {
    Busy,
    Error,
    Flash,
}

pub(in crate::ui) struct Status {
    pub(in crate::ui) status: Option<String>,
    pub(in crate::ui) tone: StatusTone,
    pub(in crate::ui) token: u64,
}

impl ReaderWindow {
    pub(in crate::ui) fn set_busy(&mut self, message: impl Into<String>) {
        self.status.token = self.status.token.wrapping_add(1);
        self.status.status = Some(message.into());
        self.status.tone = StatusTone::Busy;
    }

    pub(in crate::ui) fn set_flash(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.status.token = self.status.token.wrapping_add(1);
        let token = self.status.token;
        self.status.status = Some(message.into());
        self.status.tone = StatusTone::Flash;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = this.update(cx, |this, cx| {
                if this.status.token == token && matches!(this.status.tone, StatusTone::Flash) {
                    this.status.status = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::ui) fn set_error(&mut self, error: String) {
        self.status.token = self.status.token.wrapping_add(1);
        self.status.status = Some(i18n::error(self.preferences.language, error));
        self.status.tone = StatusTone::Error;
    }

    pub(in crate::ui) fn clear_status(&mut self) {
        self.status.token = self.status.token.wrapping_add(1);
        self.status.status = None;
    }

    pub(in crate::ui) fn record_refresh_success(&mut self) {
        self.preferences.last_refresh_at = Some(chrono::Utc::now().to_rfc3339());
        if let Err(error) = self.preferences.save(&self.preferences_path) {
            self.set_error(format!("Could not save settings: {error}"));
        }
    }

    pub(in crate::ui) fn status_context_label(&self) -> String {
        let scope = match &self.list.scope {
            Scope::All => self.t("All Articles"),
            Scope::Unread => self.t("Unread"),
            Scope::Starred => self.t("Starred"),
            Scope::Later => self.t("Read Later"),
            Scope::Feed(id) => self
                .sidebar
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
}

impl Default for Status {
    fn default() -> Self {
        Self {
            status: None,
            tone: StatusTone::Busy,
            token: 0,
        }
    }
}

impl Status {
    pub(in crate::ui) fn render_status_bar(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let (mid, update_prompt) = match &owner.update_status {
            crate::updater::UpdateStatus::Ready(update) => (
                format!("{} · {}", owner.t("Update ready"), update.version),
                true,
            ),
            crate::updater::UpdateStatus::ManualRequired { version, .. } => {
                (format!("{} · {version}", owner.t("Update available")), true)
            }
            crate::updater::UpdateStatus::NextLaunch(update) => (
                format!("{} · {}", owner.t("Update on next launch"), update.version),
                false,
            ),
            _ => (self.status.clone().unwrap_or_default(), false),
        };
        let mid_color = if update_prompt {
            cx.theme().primary
        } else {
            match self.tone {
                StatusTone::Error => cx.theme().danger,
                StatusTone::Busy | StatusTone::Flash => cx.theme().muted_foreground,
            }
        };
        let provider = if owner.sidebar.is_refreshing || owner.settings.is_connecting {
            owner.t("Syncing…").to_owned()
        } else {
            owner
                .upstream_provider_label()
                .unwrap_or_default()
                .to_owned()
        };
        let last_refresh = owner
            .preferences
            .last_refresh_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Local).format("%H:%M").to_string());
        let right = last_refresh.map_or(provider.clone(), |time| {
            let label = i18n::format(owner.preferences.language, "Last refresh: {}", time);
            if provider.is_empty() {
                label
            } else {
                format!("{provider} · {label}")
            }
        });
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
                    .child(owner.status_context_label()),
            )
            .child(
                div()
                    .id("update-status-prompt")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_center()
                    .text_color(mid_color)
                    .child(mid)
                    .when(update_prompt, |el| {
                        el.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                            this.settings.open = true;
                            this.settings.page = SettingsPage::About;
                            cx.notify();
                        }))
                    }),
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
