use super::Settings;
use crate::ui::components::text_input;
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::*;

impl Settings {
    pub(in crate::ui) fn render_reading_settings(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let collapsed = owner.preferences.sidebar_collapsed;
        let shortcuts = [
            ("j", "Next article"),
            ("k", "Previous article"),
            ("Shift+J", "Next unread"),
            ("Shift+K", "Previous unread"),
            ("u", "Toggle read"),
            ("s", "Toggle star"),
            ("l", "Toggle Read Later"),
            ("r", "Refresh feeds"),
            ("o", "Open original"),
            ("x", "Mark all as read"),
        ]
        .into_iter()
        .fold(v_flex().gap_1().pt_2(), |list, (keys, label)| {
            list.child(self.keyboard_shortcut_row(owner, keys, label, cx))
        });
        v_flex()
            .child(self.settings_heading(owner, "Reading"))
            .child(div().pb_2().font_semibold().child(owner.t("Layout")))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(owner.t("Collapse feed sidebar")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t("Leave more room for articles.")),
                            ),
                    )
                    .child(
                        Button::new("settings-sidebar").small()
                            .secondary()
                            .label(owner.t(if collapsed { "Collapsed" } else { "Expanded" }))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    ),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Keyboard")))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(owner.t("Vim navigation")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t(
                                        "Use j/k, s, u, l and other single keys when not typing.",
                                    )),
                            ),
                    )
                    .child(
                        Button::new("settings-vim-nav").small()
                            .secondary()
                            .label(owner.t(if owner.preferences.vim_navigation {
                                "On"
                            } else {
                                "Off"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences.vim_navigation = !this.preferences.vim_navigation;
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Open the Command Palette with Ctrl/Cmd+K anytime.")),
            )
            .child(shortcuts)
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Sharing")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Use {title} and {url}; write \\n for a line break.")),
            )
            .child(div().pt_3().pb_2().text_sm().child(owner.t("Share template")))
            .child(text_input(&self.share_template_input))
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Full text")))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(owner.t("Auto extract full text")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t(
                                        "Fetch the original page when opening an article without full content.",
                                    )),
                            ),
                    )
                    .child(
                        Button::new("settings-auto-extract").small()
                            .secondary()
                            .label(owner.t(if owner.preferences.auto_extract_full_text {
                                "On"
                            } else {
                                "Off"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences.auto_extract_full_text =
                                    !this.preferences.auto_extract_full_text;
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(owner.t("Content extractor")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t(
                                        "Library used when extracting full text. Re-extract to compare.",
                                    )),
                            ),
                    )
                    .child(
                        Button::new("settings-content-extractor").small()
                            .secondary()
                            .label(owner.t(owner.preferences.content_extractor.label()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences.content_extractor =
                                    this.preferences.content_extractor.next();
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            )
    }

    pub(in crate::ui) fn keyboard_shortcut_row(
        &self,
        owner: &ReaderWindow,
        keys: &'static str,
        label: &'static str,
        cx: &Context<ReaderWindow>,
    ) -> impl IntoElement {
        h_flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t(label)),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(cx.theme().muted)
                    .text_xs()
                    .font_semibold()
                    .child(keys),
            )
    }
}
