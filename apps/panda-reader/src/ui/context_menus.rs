use crate::ui::components::{BundledIcon, bundled_icon};
use crate::ui::i18n;
use crate::ui::settings::SettingsPage;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::*;
use panda_core::{ArticleSummary, Feed, MarkField, Scope};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub(in crate::ui) enum PaneMenuEvent {
    SelectScope(Scope),
    MarkAllRead(Scope),
    RefreshFeed(i64),
    EditFeed(i64),
    RemoveFeed(i64),
    ToggleFolderCollapsed(String),
    SetFolderIcon {
        folder: String,
        icon: String,
    },
    MoveFolder {
        folder: String,
        delta: isize,
    },
    ReorderFolderBefore {
        dragged: String,
        target: String,
    },
    OpenArticle(i64),
    MarkArticle {
        article_id: i64,
        field: MarkField,
        value: bool,
    },
    TranslateTitle(i64),
    ExtractFullText(i64),
    ToggleTranslation(i64),
    CreateQuote {
        article: ArticleSummary,
        selected_text: String,
    },
}

pub(in crate::ui) type PaneMenuEmitter = Rc<dyn Fn(PaneMenuEvent, &mut App)>;

pub(in crate::ui) fn pane_menu_emitter<T>(entity: WeakEntity<T>) -> PaneMenuEmitter
where
    T: EventEmitter<PaneMenuEvent> + 'static,
{
    Rc::new(move |event, cx| {
        let _ = entity.update(cx, |_, cx| cx.emit(event));
    })
}

pub(in crate::ui) fn folder_icon_element(icon_key: &str) -> Icon {
    match icon_key {
        "star" => bundled_icon(BundledIcon::Star).small(),
        "book" => Icon::new(IconName::BookOpen).small(),
        "globe" => Icon::new(IconName::Globe).small(),
        "bookmark" => bundled_icon(BundledIcon::Bookmark).small(),
        _ => bundled_icon(BundledIcon::Folder).small(),
    }
}

pub(in crate::ui) fn folder_context_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<'_, PopupMenu>,
    folder: &str,
    emit: &PaneMenuEmitter,
    language: crate::app::preferences::Language,
    selected_icon: &str,
) -> PopupMenu {
    let folder_name = folder.to_owned();
    let selected_icon = selected_icon.to_owned();
    let icon_menu_folder = folder_name.clone();
    let icon_menu_emit = emit.clone();
    let icon_menu_selected = selected_icon.clone();
    menu.min_w(px(200.))
        .item(
            PopupMenuItem::new(i18n::text(language, "Show articles")).on_click({
                let emit = emit.clone();
                let folder_name = folder_name.clone();
                move |_, _, cx| {
                    emit(
                        PaneMenuEvent::SelectScope(Scope::Folder(folder_name.clone())),
                        cx,
                    )
                }
            }),
        )
        .submenu(
            i18n::text(language, "Choose folder icon"),
            window,
            cx,
            move |mut menu, _, _| {
                for (icon_key, label_key) in [
                    ("folder", "Folder"),
                    ("star", "Star"),
                    ("book", "Book"),
                    ("globe", "Globe"),
                    ("bookmark", "Bookmark"),
                ] {
                    let folder_name = icon_menu_folder.clone();
                    let emit = icon_menu_emit.clone();
                    let label = i18n::text(language, label_key);
                    let label = if icon_menu_selected == icon_key {
                        format!("{label} ✓")
                    } else {
                        label.to_owned()
                    };
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .icon(folder_icon_element(icon_key))
                            .on_click(move |_, _, cx| {
                                emit(
                                    PaneMenuEvent::SetFolderIcon {
                                        folder: folder_name.clone(),
                                        icon: icon_key.to_owned(),
                                    },
                                    cx,
                                )
                            }),
                    );
                }
                menu
            },
        )
        .separator()
        .item(
            PopupMenuItem::new(i18n::text(language, "Move folder up")).on_click({
                let emit = emit.clone();
                let folder = folder_name.clone();
                move |_, _, cx| {
                    emit(
                        PaneMenuEvent::MoveFolder {
                            folder: folder.clone(),
                            delta: -1,
                        },
                        cx,
                    )
                }
            }),
        )
        .item(
            PopupMenuItem::new(i18n::text(language, "Move folder down")).on_click({
                let emit = emit.clone();
                let folder = folder_name;
                move |_, _, cx| {
                    emit(
                        PaneMenuEvent::MoveFolder {
                            folder: folder.clone(),
                            delta: 1,
                        },
                        cx,
                    )
                }
            }),
        )
}

pub(in crate::ui) fn feed_context_menu(
    menu: PopupMenu,
    feed: &Feed,
    emit: &PaneMenuEmitter,
    language: crate::app::preferences::Language,
    local: bool,
) -> PopupMenu {
    let id = feed.id;
    let title = feed.title.clone();
    let feed_url = feed.feed_url.clone();
    let menu = menu
        .min_w(px(200.))
        .item(
            PopupMenuItem::new(i18n::text(language, "Show articles")).on_click({
                let emit = emit.clone();
                move |_, _, cx| emit(PaneMenuEvent::SelectScope(Scope::Feed(id)), cx)
            }),
        )
        .item(
            PopupMenuItem::new(i18n::text(language, "Mark all as read")).on_click({
                let emit = emit.clone();
                move |_, _, cx| emit(PaneMenuEvent::MarkAllRead(Scope::Feed(id)), cx)
            }),
        )
        .item(
            PopupMenuItem::new(i18n::text(language, "Sync now")).on_click({
                let emit = emit.clone();
                move |_, _, cx| emit(PaneMenuEvent::RefreshFeed(id), cx)
            }),
        );
    let menu = if local {
        menu.item(
            PopupMenuItem::new(i18n::text(language, "Edit feed…")).on_click({
                let emit = emit.clone();
                move |_, _, cx| emit(PaneMenuEvent::EditFeed(id), cx)
            }),
        )
    } else {
        menu
    };
    menu.separator()
        .item(
            PopupMenuItem::new(i18n::text(language, "Copy feed title")).on_click({
                let title = title.clone();
                move |_, _, cx| copy_text(cx, title.clone())
            }),
        )
        .item(
            PopupMenuItem::new(i18n::text(language, "Copy feed URL")).on_click({
                let feed_url = feed_url.clone();
                move |_, _, cx| copy_text(cx, feed_url.clone())
            }),
        )
        .item(
            PopupMenuItem::new(i18n::text(language, "Open feed URL")).on_click({
                let feed_url = feed_url.clone();
                move |_, _, cx| cx.open_url(&feed_url)
            }),
        )
        .separator()
        .item(
            PopupMenuItem::new(i18n::text(language, "Remove feed")).on_click({
                let emit = emit.clone();
                move |_, _, cx| emit(PaneMenuEvent::RemoveFeed(id), cx)
            }),
        )
}

pub(in crate::ui) fn article_context_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<'_, PopupMenu>,
    article: &ArticleSummary,
    emit: &PaneMenuEmitter,
    language: crate::app::preferences::Language,
    include_selection_edit: bool,
) -> PopupMenu {
    let id = article.id;
    let title = article.title.clone();
    let url = article.url.clone();
    let is_read = article.is_read;
    let is_starred = article.is_starred;
    let read_later = article.read_later;
    let selected_text = if include_selection_edit {
        gpui_kit::base::TextSelection::selected_text(window, cx)
    } else {
        String::new()
    };
    let mut menu = menu;
    if !selected_text.trim().is_empty() {
        let quote_article = article.clone();
        let quote_text = selected_text.clone();
        let emit_quote = emit.clone();
        return menu
            .item(PopupMenuItem::new(i18n::text(language, "Copy")).on_click({
                let selected_text = selected_text.clone();
                move |_, _, cx| copy_text(cx, selected_text.clone())
            }))
            .separator()
            .item(
                PopupMenuItem::new(i18n::text(language, "Create Bamboo Leaf")).on_click({
                    move |_, _, cx| {
                        emit_quote(
                            PaneMenuEvent::CreateQuote {
                                article: quote_article.clone(),
                                selected_text: quote_text.clone(),
                            },
                            cx,
                        )
                    }
                }),
            );
    }
    if !include_selection_edit {
        menu = menu
            .item(PopupMenuItem::new(i18n::text(language, "Open")).on_click({
                let emit = emit.clone();
                move |_, _, cx| emit(PaneMenuEvent::OpenArticle(id), cx)
            }))
            .separator();
    }
    menu = menu
        .item(
            PopupMenuItem::new(i18n::text(
                language,
                if is_starred { "Remove star" } else { "Star" },
            ))
            .on_click({
                let emit = emit.clone();
                move |_, _, cx| {
                    emit(
                        PaneMenuEvent::MarkArticle {
                            article_id: id,
                            field: MarkField::Starred,
                            value: !is_starred,
                        },
                        cx,
                    )
                }
            }),
        )
        .item(
            PopupMenuItem::new(i18n::text(
                language,
                if read_later {
                    "Remove from Read Later"
                } else {
                    "Read Later"
                },
            ))
            .on_click({
                let emit = emit.clone();
                move |_, _, cx| {
                    emit(
                        PaneMenuEvent::MarkArticle {
                            article_id: id,
                            field: MarkField::Later,
                            value: !read_later,
                        },
                        cx,
                    )
                }
            }),
        )
        .item(
            PopupMenuItem::new(i18n::text(
                language,
                if is_read {
                    "Mark as unread"
                } else {
                    "Mark as read"
                },
            ))
            .on_click({
                let emit = emit.clone();
                move |_, _, cx| {
                    emit(
                        PaneMenuEvent::MarkArticle {
                            article_id: id,
                            field: MarkField::Read,
                            value: !is_read,
                        },
                        cx,
                    )
                }
            }),
        )
        .separator()
        .item(
            PopupMenuItem::new(i18n::text(language, "Translate title")).on_click({
                let emit = emit.clone();
                move |_, _, cx| emit(PaneMenuEvent::TranslateTitle(id), cx)
            }),
        );

    if let Some(url) = url {
        menu = menu
            .item(
                PopupMenuItem::new(i18n::text(language, "Translate")).on_click({
                    let emit = emit.clone();
                    move |_, _, cx| emit(PaneMenuEvent::ToggleTranslation(id), cx)
                }),
            )
            .item(
                PopupMenuItem::new(i18n::text(language, "Extract full text")).on_click({
                    let emit = emit.clone();
                    move |_, _, cx| emit(PaneMenuEvent::ExtractFullText(id), cx)
                }),
            )
            .separator()
            .item(
                PopupMenuItem::new(i18n::text(language, "Copy title")).on_click({
                    let title = title.clone();
                    move |_, _, cx| copy_text(cx, title.clone())
                }),
            )
            .item(
                PopupMenuItem::new(i18n::text(language, "Copy link")).on_click({
                    let url = url.clone();
                    move |_, _, cx| copy_text(cx, url.clone())
                }),
            )
            .item(
                PopupMenuItem::new(i18n::text(language, "Open original")).on_click({
                    let url = url.clone();
                    move |_, _, cx| cx.open_url(&url)
                }),
            );
    } else {
        menu = menu.separator().item(
            PopupMenuItem::new(i18n::text(language, "Copy title")).on_click({
                let title = title.clone();
                move |_, _, cx| copy_text(cx, title.clone())
            }),
        );
    }

    menu
}

pub(in crate::ui) fn handle_pane_menu_event(
    this: &mut crate::ui::ReaderWindow,
    event: &PaneMenuEvent,
    window: &mut Window,
    cx: &mut Context<crate::ui::ReaderWindow>,
) {
    match event {
        PaneMenuEvent::SelectScope(scope) => this.select_scope(scope.clone(), cx),
        PaneMenuEvent::MarkAllRead(scope) => {
            this.select_scope(scope.clone(), cx);
            this.mark_all_read(cx);
        }
        PaneMenuEvent::RefreshFeed(id) => this.refresh_one_feed(*id, cx),
        PaneMenuEvent::EditFeed(id) => this.begin_edit_feed(*id, window, cx),
        PaneMenuEvent::RemoveFeed(id) => {
            this.settings.open = true;
            this.settings.select_page(SettingsPage::General);
            this.sidebar.update(cx, |sidebar, cx| {
                sidebar.pending_remove_feed = Some(*id);
                cx.notify();
            });
            this.settings.theme_picker_open = false;
            cx.notify();
        }
        PaneMenuEvent::ToggleFolderCollapsed(folder) => this.toggle_folder_collapsed(folder, cx),
        PaneMenuEvent::SetFolderIcon { folder, icon } => this.set_folder_icon(folder, icon, cx),
        PaneMenuEvent::MoveFolder { folder, delta } => {
            this.sync_folder_order_from_feeds(cx);
            this.move_folder(folder, *delta, cx);
        }
        PaneMenuEvent::ReorderFolderBefore { dragged, target } => {
            this.reorder_folder_before(dragged, target, cx)
        }
        PaneMenuEvent::OpenArticle(id) => this.open_article(*id, cx),
        PaneMenuEvent::MarkArticle {
            article_id,
            field,
            value,
        } => this.mark(*article_id, *field, *value, cx),
        PaneMenuEvent::TranslateTitle(id) => this.translate_title_manually(*id, cx),
        PaneMenuEvent::ExtractFullText(id) => this.extract(*id, true, cx),
        PaneMenuEvent::ToggleTranslation(id) => this.toggle_or_translate(*id, cx),
        PaneMenuEvent::CreateQuote {
            article,
            selected_text,
        } => {
            let context_html = this.read_article_reader(cx, |reader| {
                if reader
                    .article
                    .as_ref()
                    .is_some_and(|current| current.summary.id == article.id)
                {
                    reader.body_html.to_string()
                } else {
                    String::new()
                }
            });
            let feed_icon = this
                .sidebar
                .read(cx)
                .feeds
                .iter()
                .find(|feed| feed.id == article.feed_id)
                .and_then(|feed| {
                    let host = crate::services::favicon::feed_host(
                        feed.site_url.as_deref(),
                        &feed.feed_url,
                    )?;
                    let icon_path = crate::services::favicon::local_favicon_path(
                        &this.sidebar.read(cx).icons_dir,
                        &host,
                    )?;
                    std::fs::read(icon_path).ok()
                });
            this.begin_quote_poster(
                selected_text.clone(),
                article.title.clone(),
                article.feed_title.clone(),
                article.url.clone(),
                context_html,
                feed_icon,
                cx,
            );
        }
    }
}

fn copy_text(cx: &mut App, text: impl Into<String>) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
}
