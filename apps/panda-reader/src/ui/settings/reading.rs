use super::Settings;
use super::actions::ReaderSetting;
use crate::app::preferences::ReaderFontFamily;
use crate::ui::components::text_input;
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    switch::Switch,
    v_flex,
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
            ("Shift+Ctrl/Cmd+R", "Re-fetch current article"),
            ("o", "Open original"),
            ("x", "Mark all as read"),
        ]
        .into_iter()
        .fold(v_flex().gap_1().pt_2(), |list, (keys, label)| {
            list.child(self.keyboard_shortcut_row(owner, keys, label, cx))
        });
        v_flex()
            .child(self.settings_heading(owner, "Reading"))
            .child(div().pb_2().font_semibold().child(owner.t("Typography")))
            .child(self.reader_font_controls(owner, cx))
            .child(self.reader_number_setting(
                owner,
                cx,
                "Text size",
                format!("{} px", owner.preferences.reader_font_size as u32),
                ReaderSetting::FontSize,
                1.,
            ))
            .child(self.reader_number_setting(
                owner,
                cx,
                "Line height",
                format!("{:.2}×", owner.preferences.reader_line_height),
                ReaderSetting::LineHeight,
                0.1,
            ))
            .child(self.reader_number_setting(
                owner,
                cx,
                "Text width",
                format!("{} px", owner.preferences.reader_content_width as u32),
                ReaderSetting::ContentWidth,
                40.,
            ))
            .child(self.reader_number_setting(
                owner,
                cx,
                "Paragraph spacing",
                format!("{:.2}×", owner.preferences.reader_paragraph_spacing),
                ReaderSetting::ParagraphSpacing,
                0.15,
            ))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(owner.t("Remember reading position")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t("Resume each article where you left off.")),
                            ),
                    )
                    .child(
                        Switch::new("settings-reading-position")
                            .checked(owner.preferences.remember_reading_position)
                            .accessibility_label(owner.t("Remember reading position"))
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.preferences.remember_reading_position = *checked;
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
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
                        Switch::new("settings-sidebar")
                            .checked(collapsed)
                            .accessibility_label(owner.t("Collapse feed sidebar"))
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.preferences.sidebar_collapsed = *checked;
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
                            .child(div().text_sm().child(owner.t("First-line indent")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t(
                                        "Indent article paragraphs by two full-width spaces.",
                                    )),
                            ),
                    )
                    .child(
                        Switch::new("settings-paragraph-indent")
                            .checked(owner.preferences.paragraph_indent)
                            .accessibility_label(owner.t("First-line indent"))
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.preferences.paragraph_indent = *checked;
                                this.save_preferences();
                                if this.reader.article.is_some() {
                                    this.request_prepared_body(cx);
                                }
                                cx.notify();
                            })),
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
                        Switch::new("settings-vim-nav")
                            .checked(owner.preferences.vim_navigation)
                            .accessibility_label(owner.t("Vim navigation"))
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.preferences.vim_navigation = *checked;
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
                        Switch::new("settings-auto-extract")
                            .checked(owner.preferences.auto_extract_full_text)
                            .accessibility_label(owner.t("Auto extract full text"))
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.preferences.auto_extract_full_text = *checked;
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

    fn reader_font_controls(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        h_flex()
            .items_center()
            .justify_between()
            .py_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child(owner.t("Article font")))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(owner.t("Choose a serif or sans-serif reading face.")),
                    ),
            )
            .child(
                Button::new("settings-reader-font")
                    .small()
                    .secondary()
                    .label(owner.t(match owner.preferences.reader_font_family {
                        ReaderFontFamily::Serif => "Serif",
                        ReaderFontFamily::Sans => "Sans-serif",
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.preferences.reader_font_family =
                            match this.preferences.reader_font_family {
                                ReaderFontFamily::Serif => ReaderFontFamily::Sans,
                                ReaderFontFamily::Sans => ReaderFontFamily::Serif,
                            };
                        this.save_preferences();
                        cx.notify();
                    })),
            )
    }

    fn reader_number_setting(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
        label: &'static str,
        value: String,
        setting: ReaderSetting,
        step: f32,
    ) -> impl IntoElement {
        h_flex()
            .items_center()
            .justify_between()
            .py_2()
            .child(div().text_sm().child(owner.t(label)))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new(format!("reader-setting-minus-{label}"))
                            .small()
                            .secondary()
                            .icon(IconName::Minus)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.adjust_reader_setting(setting, -step, cx)
                            })),
                    )
                    .child(div().w(px(72.)).text_center().text_sm().child(value))
                    .child(
                        Button::new(format!("reader-setting-plus-{label}"))
                            .small()
                            .secondary()
                            .icon(IconName::Plus)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.adjust_reader_setting(setting, step, cx)
                            })),
                    ),
            )
    }
}
