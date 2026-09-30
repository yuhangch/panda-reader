use super::*;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};

impl ReaderView {
    pub(super) fn folder_context_menu(
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<'_, PopupMenu>,
        folder: &str,
        app: &WeakEntity<Self>,
        language: Language,
        selected_icon: &str,
    ) -> PopupMenu {
        let folder_name = folder.to_owned();
        let selected_icon = selected_icon.to_owned();
        let icon_menu_folder = folder_name.clone();
        let icon_menu_app = app.clone();
        let icon_menu_selected = selected_icon.clone();
        menu.min_w(px(200.))
            .item(
                PopupMenuItem::new(i18n::text(language, "Show articles")).on_click({
                    let app = app.clone();
                    let folder_name = folder_name.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.select_scope(Scope::Folder(folder_name.clone()), cx);
                        });
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
                        let app = icon_menu_app.clone();
                        let label = i18n::text(language, label_key);
                        let label = if icon_menu_selected == icon_key {
                            format!("{label} ✓")
                        } else {
                            label.to_owned()
                        };
                        menu = menu.item(
                            PopupMenuItem::new(label)
                                .icon(Self::folder_icon_element(icon_key))
                                .on_click(move |_, _, cx| {
                                    let _ = app.update(cx, |this, cx| {
                                        this.set_folder_icon(&folder_name, icon_key, cx);
                                    });
                                }),
                        );
                    }
                    menu
                },
            )
            .separator()
            .item(
                PopupMenuItem::new(i18n::text(language, "Move folder up")).on_click({
                    let app = app.clone();
                    let folder_name = folder_name.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.sync_folder_order_from_feeds();
                            this.move_folder(&folder_name, -1, cx);
                        });
                    }
                }),
            )
            .item(
                PopupMenuItem::new(i18n::text(language, "Move folder down")).on_click({
                    let app = app.clone();
                    let folder_name = folder_name.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.sync_folder_order_from_feeds();
                            this.move_folder(&folder_name, 1, cx);
                        });
                    }
                }),
            )
    }

    pub(super) fn feed_context_menu(
        menu: PopupMenu,
        feed: &Feed,
        app: &WeakEntity<Self>,
        language: Language,
        local: bool,
    ) -> PopupMenu {
        let id = feed.id;
        let title = feed.title.clone();
        let feed_url = feed.feed_url.clone();
        let menu = menu
            .min_w(px(200.))
            .item(
                PopupMenuItem::new(i18n::text(language, "Show articles")).on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.select_scope(Scope::Feed(id), cx);
                        });
                    }
                }),
            )
            .item(
                PopupMenuItem::new(i18n::text(language, "Mark all as read")).on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.select_scope(Scope::Feed(id), cx);
                            this.mark_all_read(cx);
                        });
                    }
                }),
            )
            .item(
                PopupMenuItem::new(i18n::text(language, "Sync now")).on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| this.refresh_one_feed(id, cx));
                    }
                }),
            );
        let menu = if local {
            menu.item(
                PopupMenuItem::new(i18n::text(language, "Edit feed…")).on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.begin_edit_feed(id, window, cx);
                        });
                    }
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
                    let app = app.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.settings_open = true;
                            this.settings_page = SettingsPage::General;
                            this.pending_remove_feed = Some(id);
                            this.theme_picker_open = false;
                            cx.notify();
                        });
                    }
                }),
            )
    }

    pub(super) fn article_context_menu(
        menu: PopupMenu,
        article: &ArticleSummary,
        app: &WeakEntity<Self>,
        language: Language,
    ) -> PopupMenu {
        let id = article.id;
        let title = article.title.clone();
        let url = article.url.clone();
        let is_read = article.is_read;
        let is_starred = article.is_starred;
        let read_later = article.read_later;
        let mut menu = menu
            .min_w(px(220.))
            .item(PopupMenuItem::new(i18n::text(language, "Open")).on_click({
                let app = app.clone();
                move |_, _, cx| {
                    let _ = app.update(cx, |this, cx| this.open_article(id, cx));
                }
            }))
            .separator()
            .item(
                PopupMenuItem::new(i18n::text(
                    language,
                    if is_starred { "Remove star" } else { "Star" },
                ))
                .on_click({
                    let app = app.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.mark(id, MarkField::Starred, !is_starred, cx);
                        });
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
                    let app = app.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.mark(id, MarkField::Later, !read_later, cx);
                        });
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
                    let app = app.clone();
                    move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.mark(id, MarkField::Read, !is_read, cx);
                        });
                    }
                }),
            )
            .separator()
            .item(
                PopupMenuItem::new(i18n::text(language, "Copy title")).on_click({
                    let title = title.clone();
                    move |_, _, cx| copy_text(cx, title.clone())
                }),
            );

        if let Some(url) = url.clone() {
            menu = menu
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
                )
                .item(
                    PopupMenuItem::new(i18n::text(language, "Extract full text")).on_click({
                        let app = app.clone();
                        move |_, _, cx| {
                            let _ = app.update(cx, |this, cx| this.extract(id, true, cx));
                        }
                    }),
                )
                .item(
                    PopupMenuItem::new(i18n::text(language, "Translate")).on_click({
                        let app = app.clone();
                        move |_, _, cx| {
                            let _ = app.update(cx, |this, cx| this.toggle_or_translate(id, cx));
                        }
                    }),
                );
        }

        menu
    }
}

fn copy_text(cx: &mut App, text: impl Into<String>) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
}
