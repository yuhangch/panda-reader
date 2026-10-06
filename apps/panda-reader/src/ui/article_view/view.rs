use crate::app::preferences::ReaderFontFamily;
use crate::ui::components::{BundledIcon, article_font_family, bundled_icon, preview_text};
use crate::ui::window::ReaderWindow;
use gpui_kit::base::StyledExt as _;
use gpui_kit::base::text::{MarkdownNode, MarkdownPlugin, markdown_ast};
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
            let image_viewer_app = cx.entity().downgrade();
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
                        .icon(bundled_icon(if starred {
                            BundledIcon::StarFill
                        } else {
                            BundledIcon::Star
                        }))
                        .tooltip(star_label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mark(id, MarkField::Starred, next_starred, cx)
                        })),
                )
                .child(
                    Button::new("toggle-hide-images")
                        .small()
                        .ghost()
                        .icon(bundled_icon(if owner.preferences.hide_images {
                            BundledIcon::ImageOff
                        } else {
                            BundledIcon::Image
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
                        .icon(bundled_icon(if read_later {
                            BundledIcon::BookmarkCheck
                        } else {
                            BundledIcon::Bookmark
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
                        .icon(bundled_icon(BundledIcon::Share))
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
            let published = summary.published_at.as_deref().and_then(|value| {
                super::super::date::format_published_at(value, owner.preferences.language, true)
            });
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
            article_style = article_style
                .paragraph_gap(rems(owner.preferences.reader_paragraph_spacing))
                .heading_font_size(|level, base| match level {
                    1 => base * 1.55,
                    2 => base * 1.35,
                    3 => base * 1.2,
                    _ => base,
                });
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
                    .font(article_font_family(
                        owner.preferences.reader_font_family == ReaderFontFamily::Serif,
                    )),
                )
                .when_some(
                    self.display_translated_title(article, &owner.preferences),
                    |view, title| {
                        view.child(
                            TextView::html("article-translated-title", escape_html_text(&title))
                                .selectable(true)
                                .text_xl()
                                .font(article_font_family(
                                    owner.preferences.reader_font_family == ReaderFontFamily::Serif,
                                ))
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
                                div()
                                    .id("article-body-scroll")
                                    .flex_1()
                                    .min_h_0()
                                    .w_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                                        this.schedule_reading_progress_save(cx);
                                    }))
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .max_w(px(owner.preferences.reader_content_width))
                                            .mx_auto()
                                            .px(px(48.))
                                            .child(heading)
                                            .child(
                                                TextView::markdown(
                                                    "article-body",
                                                    self.body_markdown.clone(),
                                                )
                                                .plugin(TranslationMarkdown {
                                                    style: article_style.clone(),
                                                    font: article_font_family(
                                                        owner.preferences.reader_font_family
                                                            == ReaderFontFamily::Serif,
                                                    ),
                                                    font_size: px(owner
                                                        .preferences
                                                        .reader_font_size),
                                                    line_height: rems(
                                                        owner.preferences.reader_line_height,
                                                    ),
                                                })
                                                .scrollable(false)
                                                .w_full()
                                                .font(article_font_family(
                                                    owner.preferences.reader_font_family
                                                        == ReaderFontFamily::Serif,
                                                ))
                                                .text_size(px(owner.preferences.reader_font_size))
                                                .line_height(rems(
                                                    owner.preferences.reader_line_height,
                                                ))
                                                .style(article_style)
                                                .pb_12()
                                                .on_link_click(move |url, _, _, cx| {
                                                    if let Some(index) =
                                                        url.strip_prefix("panda-image://").and_then(
                                                            |index| index.parse::<usize>().ok(),
                                                        )
                                                    {
                                                        let _ = image_viewer_app.update(
                                                            cx,
                                                            |this, cx| {
                                                                this.reader.image_viewer_url = this
                                                                    .reader
                                                                    .image_urls
                                                                    .get(index)
                                                                    .cloned();
                                                                cx.notify();
                                                            },
                                                        );
                                                    } else {
                                                        cx.open_url(url);
                                                    }
                                                }),
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

struct TranslationMarkdown {
    style: TextViewStyle,
    font: gpui::Font,
    font_size: gpui::Pixels,
    line_height: gpui::Rems,
}

impl MarkdownPlugin for TranslationMarkdown {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "panda-translation"
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        _cx: &gpui_kit::base::text::MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Code(code) = node else {
            return None;
        };
        (code.lang.as_deref() == Some("panda-translation")).then(|| {
            MarkdownNode::new("panda-translation", ())
                .text(code.value.clone())
                .markdown(code.value.clone())
        })
    }

    fn render(
        &self,
        node: &MarkdownNode,
        _window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> impl IntoElement {
        let id = node
            .source_range()
            .map(|range| format!("article-translation-{}", range.start))
            .unwrap_or_else(|| "article-translation".to_owned());
        TextView::markdown(id, node.as_markdown().to_owned())
            .font(self.font.clone())
            .text_size(self.font_size)
            .line_height(self.line_height)
            .style(self.style.clone())
            .text_color(cx.theme().muted_foreground)
    }
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
