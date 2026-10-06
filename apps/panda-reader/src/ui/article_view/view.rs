use crate::ui::components::{article_font, preview_text, tty_icon};
use crate::ui::window::ReaderWindow;
use chrono::DateTime;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::ContextMenuExt as _,
    text::{TextView, TextViewStyle},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use panda_core::{MarkField, TranslationLayout};

use super::state::ArticleView;

impl ArticleView {
    pub(in crate::ui) fn render_reader(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let mut reader = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);
        if let Some(article) = &self.article {
            let summary = &article.summary;
            let id = summary.id;
            let menu_app = cx.entity().downgrade();
            let menu_article = summary.clone();
            let language = owner.preferences.language;
            let star_label = owner.t(if summary.is_starred {
                "Remove star"
            } else {
                "Star"
            });
            let later_label = owner.t(if summary.read_later {
                "Remove from Read Later"
            } else {
                "Read Later"
            });
            let next_starred = !summary.is_starred;
            let next_later = !summary.read_later;
            let starred = summary.is_starred;
            let read_later = summary.read_later;
            let mut toolbar = h_flex()
                .items_center()
                .gap_1()
                .px_5()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(preview_text(&summary.feed_title, 28)),
                )
                .child(
                    Button::new("toggle-star")
                        .small()
                        .ghost()
                        .icon(tty_icon(if starred { "star-fill" } else { "star" }))
                        .tooltip(star_label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mark(id, MarkField::Starred, next_starred, cx)
                        })),
                )
                .child(
                    Button::new("toggle-hide-images")
                        .small()
                        .ghost()
                        .icon(tty_icon(if owner.preferences.hide_images {
                            "image-off"
                        } else {
                            "image"
                        }))
                        .tooltip(owner.t(if owner.preferences.hide_images {
                            "Show images"
                        } else {
                            "Hide images"
                        }))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_hide_images(cx))),
                )
                .child(
                    Button::new("toggle-later")
                        .small()
                        .ghost()
                        .icon(tty_icon(if read_later {
                            "bookmark-check"
                        } else {
                            "bookmark"
                        }))
                        .tooltip(later_label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mark(id, MarkField::Later, next_later, cx)
                        })),
                );
            if article.url.is_some() {
                let has_full_text = article.extracted_html.is_some();
                toolbar = toolbar.child(
                    Button::new("extract-fulltext")
                        .small()
                        .ghost()
                        .icon(IconName::FileText)
                        .loading(self.is_extracting)
                        .tooltip(owner.t(if has_full_text {
                            "Re-extract full text"
                        } else {
                            "Extract full text"
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.extract(id, true, cx))),
                );
            }
            let translate_label = if self.showing_translation {
                owner.t("Show original")
            } else if self.has_translation_for_ui(
                article,
                &owner.preferences,
                owner.translator_config.provider.id(),
            ) {
                owner.t("Show translation")
            } else {
                owner.t("Translate")
            };
            toolbar = toolbar.child(
                Button::new("translate-article")
                    .small()
                    .ghost()
                    .icon(IconName::Globe)
                    .tooltip(translate_label)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_or_translate(id, cx))),
            );
            if self.showing_translation {
                toolbar = toolbar.child(
                    Button::new("translation-layout")
                        .small()
                        .ghost()
                        .tooltip(owner.t(owner.preferences.translation_layout.label()))
                        .label(owner.t(match owner.preferences.translation_layout {
                            TranslationLayout::Immersive => "Bilingual",
                            TranslationLayout::Replaced => "Translated",
                        }))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_translation_layout(cx))),
                );
            }
            if article.url.is_some() || summary.url.is_some() {
                toolbar = toolbar.child(
                    Button::new("share-article")
                        .small()
                        .ghost()
                        .icon(tty_icon("share"))
                        .tooltip(owner.t("Share link"))
                        .on_click(cx.listener(|this, _, _, cx| this.share_article(cx))),
                );
            }
            if let Some(url) = article.url.clone() {
                toolbar = toolbar.child(
                    Button::new("open-original")
                        .small()
                        .ghost()
                        .icon(IconName::ExternalLink)
                        .tooltip(owner.t("Open original"))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            let published = summary
                .published_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.format("%Y-%m-%d").to_string());
            let mut meta = summary.feed_title.clone();
            if let Some(author) = summary
                .author
                .as_deref()
                .filter(|author| !author.is_empty())
            {
                meta = format!("{meta}  ·  {author}");
            }
            if let Some(published) = published {
                meta = format!("{meta}  ·  {published}");
            }
            let mut article_style = TextViewStyle::default();
            article_style.heading_base_font_size = px(18.);
            article_style = article_style.inline_code(HighlightStyle {
                color: Some(cx.theme().foreground),
                ..Default::default()
            });
            article_style = article_style.paragraph_gap(rems(1.35)).heading_font_size(
                |level, base| match level {
                    1 => base * 1.55,
                    2 => base * 1.35,
                    3 => base * 1.2,
                    _ => base,
                },
            );
            let heading = v_flex()
                .gap_2()
                .pt_4()
                .pb_5()
                .mb_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    TextView::html(
                        "article-title",
                        escape_html_text(&self.display_title(article, &owner.preferences)),
                    )
                    .selectable(true)
                    .text_3xl()
                    .font_semibold()
                    .font(article_font()),
                )
                .when_some(
                    self.display_translated_title(article, &owner.preferences),
                    |view, title| {
                        view.child(
                            TextView::html("article-translated-title", escape_html_text(&title))
                                .selectable(true)
                                .text_xl()
                                .font(article_font())
                                .text_color(cx.theme().muted_foreground),
                        )
                    },
                )
                .child(
                    div()
                        .pt_1()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(meta),
                );
            reader = reader.child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .context_menu(move |menu, window, cx| {
                        ReaderWindow::article_context_menu(
                            menu,
                            window,
                            cx,
                            &menu_article,
                            &menu_app,
                            language,
                            true,
                        )
                    })
                    .child(toolbar)
                    .child(
                        v_flex()
                            .id("article-reader")
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            // gpui TextSelectionLayer updates on every MouseMove while
                            // `is_selecting` is true, without checking pressed_button. If
                            // mouse-up is lost (clickpad / focus), hover alone keeps
                            // extending the selection — end the gesture when the button
                            // is not down.
                            .on_mouse_move(|event, window, cx| {
                                if event.pressed_button != Some(MouseButton::Left) {
                                    if gpui_kit::base::TextSelection::touch_selection(window, cx)
                                        .is_none()
                                    {
                                        gpui_kit::base::TextSelection::end(window, cx);
                                    }
                                }
                            })
                            .on_mouse_up(MouseButton::Left, |_, window, cx| {
                                gpui_kit::base::TextSelection::end(window, cx);
                            })
                            .on_scroll_wheel(|_, window, cx| {
                                // Scroll wins over an in-progress text drag.
                                gpui_kit::base::TextSelection::clear(window, cx);
                            })
                            .child(
                                // Capture-phase guard so we end before the window
                                // selection layer extends on bubble MouseMove.
                                canvas(
                                    |_, _, _| (),
                                    |_, _, window, _| {
                                        window.on_mouse_event(
                                            |event: &MouseMoveEvent, phase, window, cx| {
                                                if phase != DispatchPhase::Capture {
                                                    return;
                                                }
                                                if event.pressed_button == Some(MouseButton::Left) {
                                                    return;
                                                }
                                                if gpui_kit::base::TextSelection::touch_selection(
                                                    window, cx,
                                                )
                                                .is_some()
                                                {
                                                    return;
                                                }
                                                gpui_kit::base::TextSelection::end(window, cx);
                                            },
                                        );
                                    },
                                )
                                .absolute()
                                .size(px(0.))
                                .top_0()
                                .left_0(),
                            )
                            .child(
                                v_flex()
                                    .size_full()
                                    .max_w(px(680.))
                                    .mx_auto()
                                    .px(px(48.))
                                    .child(heading)
                                    .child(
                                        div().flex_1().min_h_0().w_full().child(
                                            TextView::html("article-body", self.body_html.clone())
                                                .scrollable(true)
                                                // Scrollbar is overlaid (absolute inset); keep
                                                // text clear of the ~16px thumb track.
                                                .pr(px(20.))
                                                .font(article_font())
                                                .style(article_style)
                                                .pb_12()
                                                .on_link_click(|url, _, _, cx| cx.open_url(url)),
                                        ),
                                    ),
                            ),
                    ),
            );
        } else {
            reader = reader.items_center().justify_center().child(
                v_flex()
                    .gap_3()
                    .items_center()
                    .child(div().text_base().font_semibold().child("Panda Reader"))
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .child(owner.t("Start reading")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(owner.t("Select an article from the list to begin")),
                    ),
            );
        }
        reader
    }
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
