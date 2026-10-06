use super::Settings;
use crate::ui::components::tty_icon;
use crate::ui::theme::{self, PRESETS, Preset};
use crate::ui::window::ReaderWindow;
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
    pub(in crate::ui) fn render_appearance_settings(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let current = theme::preset(&owner.preferences.theme);
        v_flex()
            .child(self.settings_heading(owner, "Appearance"))
            .child(div().pb_1().font_semibold().child(owner.t("Theme")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Choose a color theme for reading.")),
            )
            .child(
                h_flex()
                    .id("current-theme")
                    .items_center()
                    .gap_4()
                    .mt_5()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .cursor_pointer()
                    .child(Settings::theme_preview(current))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t(if current.dark {
                                        "Dark theme"
                                    } else {
                                        "Light theme"
                                    })),
                            )
                            .child(div().font_semibold().child(owner.t(current.name))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(owner.t("Change theme  ›")),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings.theme_picker_open = true;
                        cx.notify();
                    })),
            )
            .child(div().my_7().border_t_1().border_color(cx.theme().border))
            .child(
                div()
                    .pb_2()
                    .font_semibold()
                    .child(owner.t("Interface size")),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Adjust text and controls in the sidebar, list and settings.")),
            )
            .child(self.font_size_controls(owner, cx))
            .child(div().my_7().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Branding")))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(owner.t("Show panda icon")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t("Display the panda in the top-left corner.")),
                            ),
                    )
                    .child(
                        Switch::new("settings-show-panda-icon")
                            .checked(owner.preferences.show_app_icon)
                            .accessibility_label(owner.t("Show panda icon"))
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.preferences.show_app_icon = *checked;
                                this.save_preferences();
                                cx.notify();
                            })),
                    ),
            )
    }

    pub(in crate::ui) fn font_size_controls(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        h_flex()
            .items_center()
            .gap_3()
            .pt_4()
            .child(
                Button::new("font-smaller")
                    .small()
                    .secondary()
                    .icon(IconName::Minus)
                    .tooltip(owner.t("Smaller"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_ui_font_size((this.preferences.ui_font_size - 1.).max(13.), cx)
                    })),
            )
            .child(
                div()
                    .w(px(62.))
                    .text_center()
                    .text_sm()
                    .child(format!("{} px", owner.preferences.ui_font_size as u32)),
            )
            .child(
                Button::new("font-larger")
                    .small()
                    .secondary()
                    .icon(tty_icon("plus"))
                    .tooltip(owner.t("Larger"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_ui_font_size((this.preferences.ui_font_size + 1.).min(18.), cx)
                    })),
            )
    }

    pub(in crate::ui) fn render_theme_picker(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let mut list = v_flex().gap_3().p_4();
        for preset in PRESETS {
            let id = preset.id;
            let selected = owner.preferences.theme == id;
            list = list.child(
                v_flex()
                    .id(format!("theme-preset-{id}"))
                    .gap_2()
                    .cursor_pointer()
                    .child(Settings::theme_preview(*preset))
                    .child(
                        h_flex()
                            .justify_between()
                            .text_sm()
                            .child(owner.t(preset.name))
                            .when(selected, |row| row.child(IconName::Check)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.choose_theme(id, cx))),
            );
        }
        v_flex()
            .w(px(276.))
            .h_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .pt_4()
                    .child(div().font_semibold().child(owner.t("Theme")))
                    .child(
                        Button::new("close-theme-picker")
                            .small()
                            .ghost()
                            .icon(tty_icon("close"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.theme_picker_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Choose a reading palette")),
            )
            .child(
                div()
                    .id("theme-picker-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
    }

    pub(in crate::ui) fn theme_preview(preset: Preset) -> impl IntoElement {
        let line = |width| {
            div()
                .w(px(width))
                .h(px(3.))
                .rounded_sm()
                .bg(rgb(preset.foreground))
        };
        v_flex()
            .w(px(148.))
            .h(px(78.))
            .gap_2()
            .p_3()
            .rounded(px(3.))
            .border_1()
            .border_color(rgb(preset.foreground))
            .bg(rgb(preset.background))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(7.)).h(px(3.)).bg(rgb(preset.accent)))
                    .child(line(60.)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(34.)).h(px(3.)).bg(rgb(preset.accent)))
                    .child(line(48.)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(line(24.))
                    .child(div().w(px(50.)).h(px(3.)).bg(rgb(preset.accent))),
            )
    }
}
