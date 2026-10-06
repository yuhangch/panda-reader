use super::Settings;
use crate::ui::components::text_input;
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    select::Select,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use panda_translate::Provider;

impl Settings {
    pub(in crate::ui) fn render_translation_settings(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        v_flex()
            .child(self.settings_heading(owner, "Translation"))
            .child(div().pb_2().font_semibold().child(owner.t("Language")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t(
                        "Interface language and article translation language can differ.",
                    )),
            )
            .child(div().pt_4().pb_2().text_sm().child(owner.t("Article language")))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Translate article bodies into the selected language.")),
            )
            .child(
                div().pt_3().w(px(260.)).child(
                    Select::new(&self.translation_language_select)
                        .small()
                        .placeholder(owner.t("Article language")),
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
                            .child(div().text_sm().child(owner.t("Automatically translate titles")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(owner.t(
                                        "Turn this on, then edit feeds in General to choose which titles to translate; original appears first.",
                                    )),
                            ),
                    )
                    .child(
                        Button::new("settings-auto-translate-titles")
                            .small()
                            .secondary()
                            .label(owner.t(if owner.preferences.auto_translate_titles {
                                "On"
                            } else {
                                "Off"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_auto_translate_titles(cx)
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .pt_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().child(owner.t("Title translation usage (daily)")))
                    .children(owner.title_translation_usage.iter().map(|usage| {
                        div().child(format!(
                            "{} · {} · {} requests · {} chars",
                            usage.day, usage.provider, usage.requests, usage.characters
                        ))
                    })),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Display mode")))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t(
                        "Immersive shows original and translation together, like Immersive Translate.",
                    )),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .pt_3()
                    .child(
                        div()
                            .text_sm()
                            .child(owner.t(owner.preferences.translation_layout.label())),
                    )
                    .child(
                        Button::new("settings-translation-layout")
                            .small()
                            .secondary()
                            .label(owner.t("Switch"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_translation_layout(cx)
                            })),
                    ),
            )
            .child(div().my_5().border_t_1().border_color(cx.theme().border))
            .child(div().pb_2().font_semibold().child(owner.t("Provider")))
            .child(
                div().w(px(260.)).child(
                    Select::new(&self.provider_select)
                        .small()
                        .placeholder(owner.t("Provider")),
                ),
            )
            .when(owner.translator_config.provider == Provider::Azure, |view| {
                view.child(div().pt_4().pb_2().text_sm().child(owner.t("Azure API key")))
                    .child(text_input(&self.azure_key_input).mask_toggle())
                    .child(div().pt_4().pb_2().text_sm().child(owner.t("Azure region")))
                    .child(text_input(&self.azure_region_input))
            })
            .when(owner.translator_config.provider == Provider::Volcengine, |view| {
                view.child(
                    div()
                        .pt_4()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(owner.t(
                            "Use Access Key ID / Secret from Volcengine IAM. Service: translate.",
                        )),
                )
                .child(div().pt_4().pb_2().text_sm().child(owner.t("Access Key ID")))
                .child(text_input(&self.volcengine_ak_input))
                .child(div().pt_4().pb_2().text_sm().child(owner.t("Secret Access Key")))
                .child(text_input(&self.volcengine_sk_input).mask_toggle())
            })
            .when(owner.translator_config.provider.is_ready(), |view| {
                view.child(
                    div().pt_4().child(
                        Button::new("settings-save-translator")
                            .small()
                            .secondary()
                            .label(owner.t("Save translation settings"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.persist_translator_settings(cx)
                            })),
                    ),
                )
            })
            .when(!owner.translator_config.provider.is_ready(), |view| {
                view.child(
                    div()
                        .pt_4()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(owner.t("Coming soon")),
                )
            })
    }
}
