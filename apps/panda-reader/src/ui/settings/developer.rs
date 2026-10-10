use super::Settings;
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    select::Select,
    switch::Switch,
    v_flex,
};
use gpui_kit::*;

impl ReaderWindow {
    pub(in crate::ui) fn set_detailed_sync_logging(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.preferences.detailed_sync_logging = enabled;
        self.services.set_detailed_sync_logging(enabled);
        self.save_preferences();
        cx.notify();
    }
}

impl Settings {
    pub(in crate::ui) fn render_developer_settings(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let log_path_text = owner
            .data_dir
            .join("sync.log")
            .to_string_lossy()
            .to_string();
        v_flex()
            .child(self.settings_heading(owner, "Developer"))
            .child(div().pb_2().font_semibold().child(owner.t("Diagnostics")))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(owner.t("Detailed sync logging")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t("Record Provider connection checks and per-page sync timings. Article content and credentials are never logged.")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t("Basic sync summaries and errors are always recorded.")),
                            ),
                    )
                    .child(
                        Switch::new("settings-detailed-sync-logging")
                            .checked(owner.preferences.detailed_sync_logging)
                            .accessibility_label(owner.t("Detailed sync logging"))
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.set_detailed_sync_logging(*checked, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(owner.t("Log retention")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t("Keep sync.log and its rotated copy for this long. Expired files are removed at startup and when logging runs.")),
                            ),
                    )
                    .child(
                        div().w(px(180.)).child(
                            Select::new(&self.log_retention_select)
                                .small()
                                .placeholder(owner.t("Log retention")),
                        ),
                    ),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Log file")))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(log_path_text.clone()),
            )
            .child(
                h_flex().pt_3().child(
                    Button::new("copy-sync-log-path")
                        .small()
                        .secondary()
                        .label(owner.t("Copy path"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                log_path_text.clone(),
                            ));
                            this.set_flash(this.t("Log path copied"), cx);
                        })),
                ),
            )
    }
}
