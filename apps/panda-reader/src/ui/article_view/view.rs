use crate::app::preferences::ReaderFontFamily;
use crate::ui::components::{BundledIcon, article_font_family, bundled_icon, preview_text};
use crate::ui::i18n;
use gpui_kit::base::StyledExt as _;
use gpui_kit::base::text::{MarkdownNode, MarkdownPlugin, markdown_ast};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::ContextMenuExt as _,
    scroll::Scrollbar,
    text::{TextView, TextViewStyle},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use panda_core::{MarkField, TranslationLayout};

use super::state::{ArticleView, ArticleViewEvent};

impl ArticleView {
    fn render_loading_skeleton(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let color = cx.theme().skeleton;
        let line = move |width: f32, height: f32| {
            div()
                .h(px(height))
                .w(relative(width))
                .rounded_sm()
                .bg(color)
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                h_flex()
                    .h(px(48.))
                    .px_5()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(line(0.2, 12.)),
            )
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(self.preferences.reader_content_width))
                    .mx_auto()
                    .px(px(48.))
                    .pt_8()
                    .gap_5()
                    .child(line(0.84, 28.))
                    .child(line(0.54, 28.))
                    .child(line(0.32, 12.))
                    .child(div().h(px(1.)).w_full().bg(cx.theme().border))
                    .child(v_flex().pt_3().gap_4().children(
                        [0.94, 1.0, 0.88, 0.97, 0.75, 0.92, 0.58].map(|width| line(width, 14.)),
                    )),
            )
    }

    fn render_reader(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let reader_entity = cx.entity().downgrade();
        let menu_emit = super::super::context_menus::pane_menu_emitter(reader_entity.clone());
        let language = self.preferences.language;
        let mut reader = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);
        if let Some(article) = &self.article {
            let summary = &article.summary;
            let id = summary.id;
            let menu_emit = menu_emit.clone();
            let menu_article = summary.clone();
            let star_label = i18n::text(
                language,
                if summary.is_starred {
                    "Remove star"
                } else {
                    "Star"
                },
            );
            let later_label = i18n::text(
                language,
                if summary.read_later {
                    "Remove from Read Later"
                } else {
                    "Read Later"
                },
            );
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
                        .on_click({
                            let reader_entity = reader_entity.clone();
                            move |_, _, cx| {
                                let _ = reader_entity.update(cx, |_, cx| {
                                    cx.emit(ArticleViewEvent::Mark {
                                        article_id: id,
                                        field: MarkField::Starred,
                                        value: next_starred,
                                    });
                                });
                            }
                        }),
                )
                .child(
                    Button::new("toggle-hide-images")
                        .small()
                        .ghost()
                        .icon(bundled_icon(if self.preferences.hide_images {
                            BundledIcon::ImageOff
                        } else {
                            BundledIcon::Image
                        }))
                        .tooltip(i18n::text(
                            language,
                            if self.preferences.hide_images {
                                "Show images"
                            } else {
                                "Hide images"
                            },
                        ))
                        .on_click({
                            let reader_entity = reader_entity.clone();
                            move |_, _, cx| {
                                let _ = reader_entity.update(cx, |_, cx| {
                                    cx.emit(ArticleViewEvent::ToggleHideImages);
                                });
                            }
                        }),
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
                        .on_click({
                            let reader_entity = reader_entity.clone();
                            move |_, _, cx| {
                                let _ = reader_entity.update(cx, |_, cx| {
                                    cx.emit(ArticleViewEvent::Mark {
                                        article_id: id,
                                        field: MarkField::Later,
                                        value: next_later,
                                    });
                                });
                            }
                        }),
                );
            if article.url.is_some() {
                let has_full_text = article.extracted_html.is_some();
                toolbar = toolbar.child(
                    Button::new("extract-fulltext")
                        .small()
                        .ghost()
                        .icon(IconName::FileText)
                        .loading(self.is_extracting)
                        .tooltip(i18n::text(
                            language,
                            if has_full_text {
                                "Re-extract full text"
                            } else {
                                "Extract full text"
                            },
                        ))
                        .on_click({
                            let reader_entity = reader_entity.clone();
                            move |_, _, cx| {
                                let _ = reader_entity.update(cx, |_, cx| {
                                    cx.emit(ArticleViewEvent::ExtractFullText(id));
                                });
                            }
                        }),
                );
            }
            let translate_label = if self.showing_translation {
                i18n::text(language, "Show original")
            } else if self.has_translation_for_ui(
                article,
                &self.preferences,
                &self.translator_config.revision_cache_id(),
            ) {
                i18n::text(language, "Show translation")
            } else {
                i18n::text(language, "Translate")
            };
            if self.translating_article_id == Some(id) {
                toolbar = toolbar.child(
                    Button::new("cancel-translation")
                        .small()
                        .ghost()
                        .tooltip(i18n::text(language, "Cancel"))
                        .child(
                            Icon::new(IconName::Loader).transform(Transformation::rotate(
                                percentage((self.translation_spinner_frame % 8) as f32 / 8.),
                            )),
                        )
                        .on_click({
                            let reader_entity = reader_entity.clone();
                            move |_, _, cx| {
                                let _ = reader_entity.update(cx, |_, cx| {
                                    cx.emit(ArticleViewEvent::ToggleTranslation(id));
                                });
                            }
                        }),
                );
            } else {
                toolbar = toolbar.child(
                    Button::new("translate-article")
                        .small()
                        .ghost()
                        .icon(IconName::Globe)
                        .tooltip(translate_label)
                        .on_click({
                            let reader_entity = reader_entity.clone();
                            move |_, _, cx| {
                                let _ = reader_entity.update(cx, |_, cx| {
                                    cx.emit(ArticleViewEvent::ToggleTranslation(id));
                                });
                            }
                        }),
                );
            }
            if self.showing_translation {
                toolbar = toolbar.child(
                    Button::new("translation-layout")
                        .small()
                        .ghost()
                        .tooltip(i18n::text(
                            language,
                            self.preferences.translation_layout.label(),
                        ))
                        .label(i18n::text(
                            language,
                            match self.preferences.translation_layout {
                                TranslationLayout::Immersive => "Bilingual",
                                TranslationLayout::Replaced => "Translated",
                            },
                        ))
                        .on_click({
                            let reader_entity = reader_entity.clone();
                            move |_, _, cx| {
                                let _ = reader_entity.update(cx, |_, cx| {
                                    cx.emit(ArticleViewEvent::ToggleTranslationLayout);
                                });
                            }
                        }),
                );
            }
            if article.url.is_some() || summary.url.is_some() {
                toolbar = toolbar.child(
                    Button::new("share-article")
                        .small()
                        .ghost()
                        .icon(bundled_icon(BundledIcon::Share))
                        .tooltip(i18n::text(language, "Share link"))
                        .on_click({
                            let reader_entity = reader_entity.clone();
                            move |_, _, cx| {
                                let _ = reader_entity.update(cx, |_, cx| {
                                    cx.emit(ArticleViewEvent::ShareArticle);
                                });
                            }
                        }),
                );
            }
            if let Some(url) = article.url.clone() {
                toolbar = toolbar.child(
                    Button::new("open-original")
                        .small()
                        .ghost()
                        .icon(IconName::ExternalLink)
                        .tooltip(i18n::text(language, "Open original"))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            let published = summary
                .published_at
                .as_deref()
                .and_then(|value| super::super::date::format_published_at(value, language, true));
            let feed_id = summary.feed_id;
            let mut meta = h_flex().items_center().gap_2().child(
                div()
                    .id("article-feed-source")
                    .cursor_pointer()
                    .text_color(cx.theme().primary)
                    .child(summary.feed_title.clone())
                    .on_click({
                        let reader_entity = reader_entity.clone();
                        move |_, _, cx| {
                            let _ = reader_entity.update(cx, |_, cx| {
                                cx.emit(ArticleViewEvent::SelectFeed(feed_id));
                            });
                        }
                    }),
            );
            if let Some(author) = summary
                .author
                .as_deref()
                .filter(|author| !author.is_empty())
            {
                meta = meta.child(div().text_color(cx.theme().muted_foreground).child("·"));
                meta = meta.child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(author.to_owned()),
                );
            }
            if let Some(published) = published {
                meta = meta.child(div().text_color(cx.theme().muted_foreground).child("·"));
                meta = meta.child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(published),
                );
            }
            let mut article_style = TextViewStyle::default();
            article_style.heading_base_font_size = px(18.);
            article_style = article_style.inline_code(HighlightStyle {
                color: Some(cx.theme().foreground),
                ..Default::default()
            });
            // Immersive translation places a translated block after each
            // source block, so the normal paragraph gap looks doubled there.
            let paragraph_gap = if self.showing_translation
                && self.preferences.translation_layout == TranslationLayout::Immersive
            {
                self.preferences.reader_paragraph_spacing * 0.45
            } else {
                self.preferences.reader_paragraph_spacing
            };
            article_style = article_style
                .paragraph_gap(rems(paragraph_gap))
                .heading_font_size(|level, base| match level {
                    1 => base * 1.4,
                    2 => base * 1.22,
                    3 => base * 1.08,
                    _ => base,
                });
            let heading = v_flex()
                .gap_2()
                .pt_4()
                .pb_5()
                .mb(rems(self.preferences.reader_paragraph_spacing))
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    TextView::html(
                        "article-title",
                        escape_html_text(&self.display_title(article, &self.preferences)),
                    )
                    .selectable(true)
                    .text_3xl()
                    .font_semibold()
                    .font(article_font_family(
                        self.preferences.reader_font_family == ReaderFontFamily::Serif,
                    )),
                )
                .when_some(
                    self.display_translated_title(article, &self.preferences),
                    |view, title| {
                        view.child(
                            TextView::html("article-translated-title", escape_html_text(&title))
                                .selectable(true)
                                .text_xl()
                                .font(article_font_family(
                                    self.preferences.reader_font_family == ReaderFontFamily::Serif,
                                ))
                                .text_color(cx.theme().muted_foreground),
                        )
                    },
                )
                .child(div().pt_1().text_sm().child(meta));
            reader = reader.child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .on_mouse_down(gpui::MouseButton::Right, {
                        let article_view = reader_entity.clone();
                        move |event, window, cx| {
                            let selectable = gpui_kit::base::TextSelection::is_selectable_at(
                                event.position,
                                window,
                                cx,
                            );
                            let image_target = article_view
                                .update(cx, |view, _| {
                                    view.context_menu_image = view.pending_context_image.take();
                                    view.context_menu_image.is_some()
                                })
                                .unwrap_or(false);
                            if image_target || !selectable {
                                gpui_kit::base::TextSelection::clear(window, cx);
                            }
                        }
                    })
                    .context_menu({
                        let context_reader = reader_entity.clone();
                        move |menu, window, cx| {
                            let image_target = context_reader
                                .update(cx, |view, _| {
                                    view.context_menu_image.take().and_then(|url| {
                                        view.article_image_cache
                                            .get(&url)
                                            .cloned()
                                            .map(|image| (url, image))
                                    })
                                })
                                .ok()
                                .flatten();
                            if let Some((image_url, image)) = image_target {
                                let article_view = context_reader.clone();
                                return menu
                                    .item(
                                        gpui_kit::component::menu::PopupMenuItem::new(
                                            crate::ui::i18n::text(language, "Copy image"),
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                let _ = article_view.update(cx, |_, cx| {
                                                    cx.emit(ArticleViewEvent::CopyImage(
                                                        image.clone(),
                                                    ));
                                                });
                                            },
                                        ),
                                    )
                                    .separator()
                                    .item(
                                        gpui_kit::component::menu::PopupMenuItem::new(
                                            crate::ui::i18n::text(language, "Copy image link"),
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                cx.write_to_clipboard(
                                                    gpui_kit::ClipboardItem::new_string(
                                                        image_url.clone(),
                                                    ),
                                                );
                                            },
                                        ),
                                    );
                            }
                            super::super::context_menus::article_context_menu(
                                menu,
                                window,
                                cx,
                                &menu_article,
                                &menu_emit,
                                language,
                                true,
                            )
                        }
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
                                if gpui_kit::base::TextSelection::has_selection(window, cx) {
                                    gpui_kit::base::TextSelection::clear(window, cx);
                                }
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
                                    .relative()
                                    .flex_1()
                                    .min_h_0()
                                    .w_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .on_scroll_wheel({
                                        let reader_entity = reader_entity.clone();
                                        move |_, _, cx| {
                                            let _ = reader_entity.update(cx, |_, cx| {
                                                cx.emit(ArticleViewEvent::SaveReadingProgress);
                                            });
                                        }
                                    })
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .max_w(px(self.preferences.reader_content_width))
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
                                                        self.preferences.reader_font_family
                                                            == ReaderFontFamily::Serif,
                                                    ),
                                                    font_size: px(self
                                                        .preferences
                                                        .reader_font_size),
                                                    line_height: rems(
                                                        self.preferences.reader_line_height,
                                                    ),
                                                    heading_gap: rems(
                                                        self.preferences.reader_paragraph_spacing
                                                            * 0.5,
                                                    ),
                                                    image_urls: self.image_urls.clone(),
                                                    image_viewer: reader_entity.clone(),
                                                })
                                                .plugin(ArticleHeadingMarkdown {
                                                    style: article_style.clone(),
                                                    font: article_font_family(
                                                        self.preferences.reader_font_family
                                                            == ReaderFontFamily::Serif,
                                                    ),
                                                    line_height: rems(
                                                        self.preferences.reader_line_height,
                                                    ),
                                                    heading_gap: rems(
                                                        self.preferences.reader_paragraph_spacing
                                                            * 0.5,
                                                    ),
                                                })
                                                .plugin(ArticleImageMarkdown {
                                                    image_urls: self.image_urls.clone(),
                                                    article_view: reader_entity.clone(),
                                                })
                                                .scrollable(false)
                                                .w_full()
                                                .font(article_font_family(
                                                    self.preferences.reader_font_family
                                                        == ReaderFontFamily::Serif,
                                                ))
                                                .text_size(px(self.preferences.reader_font_size))
                                                .line_height(rems(
                                                    self.preferences.reader_line_height,
                                                ))
                                                .style(article_style)
                                                .pb_12()
                                                .on_link_click(move |url, _, _, cx| {
                                                    if let Some(index) =
                                                        url.strip_prefix("panda-image://").and_then(
                                                            |index| index.parse::<usize>().ok(),
                                                        )
                                                    {
                                                        let _ =
                                                            reader_entity.update(cx, |_, cx| {
                                                                cx.emit(
                                                                    ArticleViewEvent::OpenImage(
                                                                        index,
                                                                    ),
                                                                );
                                                            });
                                                    } else {
                                                        cx.open_url(url);
                                                    }
                                                }),
                                            )
                                            .child(Scrollbar::vertical(&self.scroll)),
                                    ),
                            ),
                    ),
            );
        } else if self.requested_article_id.is_some() {
            if self.show_loading_skeleton {
                reader = reader.child(self.render_loading_skeleton(cx));
            }
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
                            .child(i18n::text(language, "Start reading")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(i18n::text(
                                language,
                                "Select an article from the list to begin",
                            )),
                    ),
            );
        }
        reader
    }
}

impl Render for ArticleView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_reader(cx)
    }
}

struct TranslationMarkdown {
    style: TextViewStyle,
    font: gpui::Font,
    font_size: gpui::Pixels,
    line_height: gpui::Rems,
    heading_gap: gpui::Rems,
    image_urls: Vec<String>,
    image_viewer: gpui::WeakEntity<ArticleView>,
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
            .plugin(ArticleHeadingMarkdown {
                style: self.style.clone(),
                font: self.font.clone(),
                line_height: self.line_height,
                heading_gap: self.heading_gap,
            })
            .plugin(ArticleImageMarkdown {
                image_urls: self.image_urls.clone(),
                article_view: self.image_viewer.clone(),
            })
            .font(self.font.clone())
            .text_size(self.font_size)
            .line_height(self.line_height)
            .style(self.style.clone())
            .text_color(cx.theme().muted_foreground)
    }
}

struct ArticleImageData {
    index: usize,
    url: String,
}

struct ArticleImageMarkdown {
    image_urls: Vec<String>,
    article_view: gpui::WeakEntity<ArticleView>,
}

impl MarkdownPlugin for ArticleImageMarkdown {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "panda-reader-image"
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        _cx: &gpui_kit::base::text::MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Code(code) = node else {
            return None;
        };
        if code.lang.as_deref() != Some("panda-reader-image") {
            return None;
        }
        let index = code
            .value
            .lines()
            .next()?
            .trim()
            .strip_prefix("panda-image:")?
            .parse::<usize>()
            .ok()?;
        let url = self.image_urls.get(index)?.clone();
        Some(
            MarkdownNode::new("panda-reader-image", ArticleImageData { index, url })
                .text(code.value.clone()),
        )
    }

    fn render(
        &self,
        node: &MarkdownNode,
        _window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> impl IntoElement {
        let mut container = div().w_full().min_w_0().py_2();
        if let Some(image) = node.data::<ArticleImageData>() {
            let index = image.index;
            let article_view = self.article_view.clone();
            let display_image = article_view
                .update(cx, |view, cx| view.load_article_image(&image.url, cx))
                .ok()
                .flatten();
            let image_element = if let Some(display_image) = display_image {
                div()
                    .flex_none()
                    .max_w_full()
                    .on_mouse_down(gpui::MouseButton::Right, {
                        let url = image.url.clone();
                        let article_view = article_view.clone();
                        move |_, _, cx| {
                            let _ = article_view.update(cx, |view, _| {
                                view.pending_context_image = Some(url.clone());
                            });
                        }
                    })
                    .child(
                        img(display_image)
                            .id(("article-body-image", index as u64))
                            .max_w_full()
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                let _ = article_view.update(cx, |_, cx| {
                                    cx.emit(ArticleViewEvent::OpenImage(index));
                                });
                            }),
                    )
            } else {
                div().w_full().h(px(160.)).max_w_full()
            };
            container = container.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .justify_center()
                    .child(image_element),
            );
        }
        container
    }
}

struct ArticleHeadingMarkdown {
    style: TextViewStyle,
    font: gpui::Font,
    line_height: gpui::Rems,
    heading_gap: gpui::Rems,
}

impl MarkdownPlugin for ArticleHeadingMarkdown {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "panda-article-heading"
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &gpui_kit::base::text::MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Heading(heading) = node else {
            return None;
        };
        let content = heading
            .children
            .iter()
            .filter_map(|child| cx.node_source(child))
            .collect::<String>();
        if content.trim().is_empty() {
            return None;
        }

        Some(
            MarkdownNode::new("panda-article-heading", heading.depth)
                .text(content.clone())
                .markdown(content),
        )
    }

    fn render(
        &self,
        node: &MarkdownNode,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) -> impl IntoElement {
        let level = node.data::<u8>().copied().unwrap_or(2);
        let base = px(18.);
        let size = match level {
            1 => base * 1.4,
            2 => base * 1.22,
            3 => base * 1.08,
            _ => base,
        };
        let weight = match level {
            1 => gpui::FontWeight::BOLD,
            2..=5 => gpui::FontWeight::SEMIBOLD,
            6 => gpui::FontWeight::MEDIUM,
            _ => gpui::FontWeight::NORMAL,
        };
        let id = node
            .source_range()
            .map(|range| format!("article-heading-{}", range.start))
            .unwrap_or_else(|| "article-heading".to_owned());

        div().pb(self.heading_gap).child(
            TextView::markdown(id, node.as_markdown().to_owned())
                .font(self.font.clone())
                .text_size(size)
                .font_weight(weight)
                .line_height(self.line_height)
                .style(self.style.clone().paragraph_gap(rems(0.))),
        )
    }
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
