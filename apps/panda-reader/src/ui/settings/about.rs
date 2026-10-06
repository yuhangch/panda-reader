use super::Settings;
use crate::ui::i18n;
use crate::ui::window::ReaderWindow;
use crate::updater::UpdateStatus;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

impl Settings {
    pub(in crate::ui) fn render_about_settings(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        v_flex()
            .child(self.settings_heading(owner, "About"))
            .child(div().pb_2().font_semibold().child("Panda Reader"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "Version {} · gpui-kit",
                        env!("CARGO_PKG_VERSION")
                    )),
            )
            .child(
                h_flex()
                    .gap_4()
                    .pt_2()
                    .text_sm()
                    .text_color(cx.theme().primary)
                    .child(
                        div()
                            .id("about-github-link")
                            .cursor_pointer()
                            .child("Panda Reader on GitHub")
                            .on_click(|_, _, cx| cx.open_url("https://github.com/yuhangch/panda-reader")),
                    )
            )
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Updates")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Check for stable releases and download updates in the background.")),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(div().text_sm().child(owner.t("Check for updates automatically")))
                    .child(
                        Switch::new("settings-auto-updates")
                            .checked(owner.preferences.check_for_updates)
                            .accessibility_label(owner.t("Check for updates automatically"))
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.set_update_checks(*checked, cx);
                            })),
                    ),
            )
            .child(update_status(owner, cx))
            .when(self.community_plugins_checked && self.community_plugin_error.is_none(), |view| {
                let available = self.community_update_count();
                view.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(if available == 0 {
                            owner.t("Community plugins are up to date.").to_owned()
                        } else {
                            i18n::format(
                                owner.preferences.language,
                                "{} community plugin update(s) available. Open Plugins to update.",
                                &available.to_string(),
                            )
                        }),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .pt_2()
                    .when(!matches!(owner.update_status, UpdateStatus::Ready(_) | UpdateStatus::NextLaunch(_)), |row| {
                        row.child(
                            Button::new("check-for-updates")
                                .small()
                                .secondary()
                                .icon(IconName::RefreshCw)
                                .label(owner.t("Check now"))
                                .on_click(cx.listener(|this, _, _, cx| this.check_for_updates(cx))),
                        )
                    })
                    .when(matches!(owner.update_status, UpdateStatus::Ready(_)), |row| {
                        row.child(
                            Button::new("update-and-restart")
                                .small()
                                .primary()
                                .label(owner.t("Update and restart"))
                                .on_click(cx.listener(|this, _, _, cx| this.update_now(cx))),
                        )
                        .child(
                            Button::new("update-next-launch")
                                .small()
                                .secondary()
                                .label(owner.t("Next launch"))
                                .on_click(cx.listener(|this, _, _, cx| this.update_next_launch(cx))),
                        )
                        .child(
                            Button::new("update-later")
                                .small()
                                .ghost()
                                .label(owner.t("Later"))
                                .on_click(cx.listener(|this, _, _, cx| this.update_later(cx))),
                        )
                    })
                    .when(matches!(owner.update_status, UpdateStatus::ManualRequired { .. } | UpdateStatus::Later(_)), |row| {
                        row.child(
                            Button::new("open-release-page")
                                .small()
                                .secondary()
                                .label(owner.t("Open releases"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    match &this.update_status {
                                        UpdateStatus::ManualRequired { release_url, .. } => cx.open_url(release_url),
                                        UpdateStatus::Later(update) => cx.open_url(&update.release_url),
                                        _ => {}
                                    }
                                })),
                        )
                    }),
            )
            .child(div().my_6().border_t_1().border_color(cx.theme().border))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Feeds, articles and settings are stored on this device.")),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Papr reference")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Papr informed our feed parsing, article extraction, and store design. Panda Reader's implementation is independent and does not include papr-core.")),
            )
            .child(
                h_flex()
                    .gap_4()
                    .pt_2()
                    .text_sm()
                    .text_color(cx.theme().primary)
                    .child(
                        div()
                            .id("about-papr-link")
                            .cursor_pointer()
                            .child("Papr on GitHub")
                            .on_click(|_, _, cx| cx.open_url("https://github.com/l0ng-ai/papr")),
                    ),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("TTY7 reference")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("TTY7 inspired our packaging and theme design.")),
            )
    }
}

fn update_status(owner: &ReaderWindow, cx: &mut Context<ReaderWindow>) -> impl IntoElement {
    let message = match &owner.update_status {
        UpdateStatus::Idle => owner.t("Updates have not been checked yet.").to_owned(),
        UpdateStatus::Checking => owner.t("Checking for updates…").to_owned(),
        UpdateStatus::Downloading {
            version,
            received,
            total,
        } => {
            let progress = if *total > 0 {
                received.saturating_mul(100) / total
            } else {
                0
            };
            format!(
                "{} · {progress}%",
                i18n::format(owner.preferences.language, "Downloading update {}", version)
            )
        }
        UpdateStatus::UpToDate => owner.t("Panda Reader is up to date.").to_owned(),
        UpdateStatus::Ready(update) => i18n::format(
            owner.preferences.language,
            "Version {} is ready to install.",
            &update.version,
        ),
        UpdateStatus::NextLaunch(update) => i18n::format(
            owner.preferences.language,
            "Version {} will install next launch.",
            &update.version,
        ),
        UpdateStatus::Later(update) => i18n::format(
            owner.preferences.language,
            "Version {} is available when you are ready.",
            &update.version,
        ),
        UpdateStatus::ManualRequired {
            version, reason, ..
        } => format!(
            "{} · {reason}",
            i18n::format(
                owner.preferences.language,
                "Version {} is available.",
                version
            )
        ),
        UpdateStatus::Failed(error) => format!("{} {error}", owner.t("Update check failed:")),
    };
    div()
        .py_1()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(message)
}
