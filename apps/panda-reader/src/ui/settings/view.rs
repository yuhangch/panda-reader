use crate::ui::components::{sidebar_font, tty_icon};
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::selectors::SettingsPage;
use super::state::Settings;

impl Settings {
    pub(in crate::ui) fn render_settings(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let page = match self.page {
            SettingsPage::General => self.render_general_settings(owner, cx).into_any_element(),
            SettingsPage::Appearance => self
                .render_appearance_settings(owner, cx)
                .into_any_element(),
            SettingsPage::Reading => self.render_reading_settings(owner, cx).into_any_element(),
            SettingsPage::Translation => self
                .render_translation_settings(owner, cx)
                .into_any_element(),
            SettingsPage::About => self.render_about_settings(owner, cx).into_any_element(),
        };
        let build_label = if cfg!(debug_assertions) {
            owner.t("Development build").to_owned()
        } else {
            format!("v{}", env!("CARGO_PKG_VERSION"))
        };
        h_flex()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .w(px(220.))
                    .h_full()
                    .font(sidebar_font())
                    .bg(cx.theme().sidebar)
                    .border_r_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(
                        h_flex()
                            .id("settings-back")
                            .items_center()
                            .gap_2()
                            .mx_3()
                            .mt_3()
                            .mb_5()
                            .px_3()
                            .py_2()
                            .rounded(cx.theme().radius)
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().sidebar_accent))
                            .child(IconName::ArrowLeft)
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .child(owner.t("Back to Reader")),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.open = false;
                                this.settings.theme_picker_open = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .px_5()
                            .pb_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(owner.t("SETTINGS")),
                    )
                    .child(
                        v_flex()
                            .gap_0p5()
                            .px_2()
                            .child(self.settings_nav_row(
                                owner,
                                "settings-general",
                                "General",
                                tty_icon("settings"),
                                SettingsPage::General,
                                cx,
                            ))
                            .child(self.settings_nav_row(
                                owner,
                                "settings-reading",
                                "Reading",
                                Icon::new(IconName::BookOpen),
                                SettingsPage::Reading,
                                cx,
                            ))
                            .child(self.settings_nav_row(
                                owner,
                                "settings-translation",
                                "Translation",
                                Icon::new(IconName::Globe),
                                SettingsPage::Translation,
                                cx,
                            ))
                            .child(self.settings_nav_row(
                                owner,
                                "settings-appearance",
                                "Appearance",
                                tty_icon("appearance"),
                                SettingsPage::Appearance,
                                cx,
                            ))
                            .child(self.settings_nav_row(
                                owner,
                                "settings-about",
                                "About",
                                tty_icon("about"),
                                SettingsPage::About,
                                cx,
                            )),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .px_5()
                            .py_4()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Panda Reader · {build_label}")),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("settings-scroll")
                            .flex_1()
                            .w_full()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(
                                h_flex().w_full().justify_center().child(
                                    div()
                                        .w(px(640.))
                                        .max_w_full()
                                        .px_5()
                                        .pt_5()
                                        .pb_8()
                                        .child(page),
                                ),
                            ),
                    ),
            )
            .when(self.theme_picker_open, |view| {
                view.child(self.render_theme_picker(owner, cx))
            })
    }

    pub(in crate::ui) fn settings_nav_row(
        &self,
        owner: &ReaderWindow,
        id: &'static str,
        label: &'static str,
        icon: Icon,
        page: SettingsPage,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let selected = self.page == page;
        h_flex()
            .id(id)
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .cursor_pointer()
            .bg(if selected {
                cx.theme().sidebar_accent
            } else {
                cx.theme().sidebar
            })
            .hover(|style| style.bg(cx.theme().sidebar_accent))
            .child(icon.small())
            .child(div().text_sm().child(owner.t(label)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings.page = page;
                this.settings.theme_picker_open = false;
                cx.notify();
            }))
    }

    pub(in crate::ui) fn settings_heading(
        &self,
        owner: &ReaderWindow,
        title: &'static str,
    ) -> impl IntoElement {
        div().pb_5().text_xl().font_semibold().child(owner.t(title))
    }
}
