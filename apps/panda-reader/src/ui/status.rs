use crate::ui::components::{BundledIcon, bundled_icon};
use crate::ui::i18n;
use crate::ui::window::ReaderWindow;
use chrono::{DateTime, Local};
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) struct StatusBarPresentation {
    pub(in crate::ui) context: String,
    pub(in crate::ui) message: String,
    pub(in crate::ui) tone: StatusTone,
    pub(in crate::ui) update_prompt: bool,
    copy_error_label: Option<&'static str>,
    pub(in crate::ui) right: String,
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum StatusBarEvent {
    OpenAbout,
}

pub(in crate::ui) struct StatusBar {
    presentation: StatusBarPresentation,
}

impl EventEmitter<StatusBarEvent> for StatusBar {}

impl StatusBar {
    pub(in crate::ui) fn set_presentation(&mut self, presentation: StatusBarPresentation) -> bool {
        if self.presentation == presentation {
            return false;
        }
        self.presentation = presentation;
        true
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self {
            presentation: StatusBarPresentation {
                context: String::new(),
                message: String::new(),
                tone: StatusTone::Busy,
                update_prompt: false,
                copy_error_label: None,
                right: String::new(),
            },
        }
    }
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

    pub(in crate::ui) fn record_refresh_success(
        &mut self,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.preferences.last_refresh_at = Some(chrono::Utc::now().to_rfc3339());
        match self.preferences.save(&self.preferences_path) {
            Ok(()) => self.set_flash(message, cx),
            Err(error) => self.set_error(format!("Could not save settings: {error}")),
        }
    }

    pub(in crate::ui) fn status_context_label(&self, cx: &App) -> String {
        let current_scope = self.list.read(cx).scope.clone();
        let scope = match &current_scope {
            Scope::All => self.t("All Articles"),
            Scope::Unread => self.t("Unread"),
            Scope::Starred => self.t("Starred"),
            Scope::Later => self.t("Read Later"),
            Scope::Feed(id) => self
                .sidebar
                .read(cx)
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
            i18n::format(
                self.preferences.language,
                "{} unread",
                self.unread_total(cx)
            )
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
    pub(in crate::ui) fn presentation(
        &self,
        owner: &ReaderWindow,
        cx: &App,
    ) -> StatusBarPresentation {
        let (message, update_prompt, tone) = match (&owner.update_status, &self.status) {
            (_, Some(error)) if self.tone == StatusTone::Error => {
                (error.clone(), false, StatusTone::Error)
            }
            (crate::updater::UpdateStatus::Ready(update), _) => (
                format!("{} · {}", owner.t("Update ready"), update.version),
                true,
                StatusTone::Flash,
            ),
            (crate::updater::UpdateStatus::ManualRequired { version, .. }, _) => (
                format!("{} · {version}", owner.t("Update available")),
                true,
                StatusTone::Flash,
            ),
            (crate::updater::UpdateStatus::NextLaunch(update), _) => (
                format!("{} · {}", owner.t("Update on next launch"), update.version),
                false,
                StatusTone::Flash,
            ),
            _ => (self.status.clone().unwrap_or_default(), false, self.tone),
        };
        let provider = if owner.sidebar.read(cx).is_refreshing || owner.settings.is_connecting {
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
        StatusBarPresentation {
            context: owner.status_context_label(cx),
            copy_error_label: (tone == StatusTone::Error && !message.is_empty())
                .then(|| i18n::text(owner.preferences.language, "Copy")),
            message,
            tone,
            update_prompt,
            right,
        }
    }
}

impl ReaderWindow {
    pub(in crate::ui) fn sync_status_bar(&self, cx: &mut Context<Self>) {
        let presentation = self.status.presentation(self, cx);
        self.status_view.update(cx, |status_bar, status_cx| {
            if status_bar.set_presentation(presentation) {
                status_cx.notify();
            }
        });
    }
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let presentation = &self.presentation;
        let mid_color = if presentation.update_prompt {
            cx.theme().primary
        } else {
            match presentation.tone {
                StatusTone::Error => cx.theme().danger,
                StatusTone::Busy | StatusTone::Flash => cx.theme().muted_foreground,
            }
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
                    .child(presentation.context.clone()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .id("update-status-prompt")
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_center()
                            .text_color(mid_color)
                            .child(presentation.message.clone())
                            .when(presentation.update_prompt, |el| {
                                el.cursor_pointer().on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(StatusBarEvent::OpenAbout);
                                }))
                            }),
                    )
                    .when_some(presentation.copy_error_label, |row, label| {
                        let error = presentation.message.clone();
                        row.child(
                            Button::new("copy-status-error")
                                .small()
                                .ghost()
                                .icon(bundled_icon(BundledIcon::Copy))
                                .flex_shrink_0()
                                .h(px(20.))
                                .tooltip(label)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(error.clone()));
                                }),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_right()
                    .child(presentation.right.clone()),
            )
    }
}
