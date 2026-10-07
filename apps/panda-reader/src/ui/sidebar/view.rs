use crate::app::preferences::LibrarySource;
use crate::services::favicon::{feed_host, local_favicon_path};
use crate::ui::components::letter_avatar;
use crate::ui::components::{BundledIcon, bundled_icon, preview_text, sidebar_font};
use crate::ui::window::ReaderWindow;
use gpui_kit::base::{StyledExt as _, v_virtual_list};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::ContextMenuExt as _,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use panda_core::{Feed, Scope};
use std::rc::Rc;

use super::state::Sidebar;

enum SidebarRow {
    Feed {
        feed_index: usize,
        nested: bool,
        has_error: bool,
    },
    Folder {
        name: String,
        unread: i64,
        collapsed: bool,
    },
}

impl SidebarRow {
    fn height(&self) -> f32 {
        match self {
            Self::Feed { has_error, .. } => {
                if *has_error {
                    44.
                } else {
                    30.
                }
            }
            Self::Folder { .. } => 30.,
        }
    }
}

#[derive(Clone, Debug)]
struct FolderDrag(String);

struct FolderDragPreview {
    name: SharedString,
}

impl Render for FolderDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .bg(cx.theme().sidebar_accent)
            .text_color(cx.theme().sidebar_accent_foreground)
            .text_sm()
            .child(bundled_icon(BundledIcon::Folder).small())
            .child(self.name.clone())
    }
}

impl Sidebar {
    pub(in crate::ui) fn render_feed_row(
        &self,
        owner: &ReaderWindow,
        feed: &Feed,
        nested: bool,
        selected_scope: &Scope,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let id = feed.id;
        let is_selected = selected_scope == &Scope::Feed(id);
        let menu_app = cx.entity().downgrade();
        let language = owner.preferences.language;
        let local = owner.settings.library_source == LibrarySource::Local;
        let menu_feed = feed.clone();
        let sidebar_bg = cx.theme().sidebar;
        let active_bg = cx.theme().sidebar_accent;
        h_flex()
            .id(("feed-row", id as u64))
            .w_full()
            .h(px(if feed.last_error.is_some() { 44. } else { 30. }))
            .items_center()
            .rounded(cx.theme().radius)
            .bg(if is_selected { active_bg } else { sidebar_bg })
            .hover(|style| style.bg(active_bg))
            .child(
                h_flex()
                    .id(("feed", id as u64))
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .when(nested, |row| row.pl_5())
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child({
                        let avatar = letter_avatar(&feed.title);
                        let danger = feed.last_error.is_some();
                        let accent_bg = if danger {
                            cx.theme().danger.opacity(0.18)
                        } else {
                            cx.theme().accent.opacity(0.16)
                        };
                        let accent_fg = if danger {
                            cx.theme().danger
                        } else {
                            cx.theme().accent
                        };
                        let letter = {
                            let avatar = avatar.clone();
                            move || {
                                div()
                                    .size(px(20.))
                                    .rounded(px(4.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(accent_bg)
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(accent_fg)
                                    .child(avatar.clone())
                                    .into_any_element()
                            }
                        };
                        match feed_host(feed.site_url.as_deref(), &feed.feed_url)
                            .and_then(|host| local_favicon_path(&self.icons_dir, &host))
                        {
                            Some(path) => {
                                let loading_avatar = avatar.clone();
                                img(path)
                                    .id(("feed-icon", id as u64))
                                    .size(px(20.))
                                    .rounded(px(4.))
                                    .overflow_hidden()
                                    .object_fit(ObjectFit::Cover)
                                    .with_fallback(letter)
                                    .with_loading(move || {
                                        div()
                                            .size(px(20.))
                                            .rounded(px(4.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .bg(accent_bg)
                                            .text_xs()
                                            .font_semibold()
                                            .text_color(accent_fg)
                                            .child(loading_avatar.clone())
                                            .into_any_element()
                                    })
                                    .into_any_element()
                            }
                            None => letter(),
                        }
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .overflow_hidden()
                                    .text_sm()
                                    .truncate()
                                    .child(feed.title.clone()),
                            )
                            .when_some(feed.last_error.clone(), |row, error| {
                                row.child(
                                    div()
                                        .overflow_hidden()
                                        .text_xs()
                                        .truncate()
                                        .text_color(cx.theme().danger)
                                        .child(preview_text(&error, 42)),
                                )
                            }),
                    )
                    .when(feed.unread > 0, |row| {
                        row.child(div().text_xs().child(feed.unread.to_string()))
                    })
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_scope(Scope::Feed(id), cx)),
                    )
                    .context_menu(move |menu, _, _| {
                        ReaderWindow::feed_context_menu(
                            menu, &menu_feed, &menu_app, language, local,
                        )
                    }),
            )
    }

    pub(in crate::ui) fn render_folder_row(
        &self,
        owner: &ReaderWindow,
        folder: &str,
        unread: i64,
        folder_collapsed: bool,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let selected = matches!(&owner.list.scope, Scope::Folder(name) if name == folder);
        let folder_for_scope = folder.to_owned();
        let folder_for_toggle = folder.to_owned();
        let folder_for_drop = folder.to_owned();
        let folder_for_menu = folder.to_owned();
        let selected_icon = owner.folder_icon_key(folder).to_owned();
        let folder_icon = ReaderWindow::folder_icon_element(owner.folder_icon_key(folder));
        let folder_key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            folder.hash(&mut hasher);
            hasher.finish()
        };
        let menu_app = cx.entity().downgrade();
        let language = owner.preferences.language;
        h_flex()
            .id(("folder-row", folder_key))
            .w_full()
            .h(px(30.))
            .items_center()
            .rounded(cx.theme().radius)
            .bg(if selected {
                cx.theme().sidebar_accent
            } else {
                cx.theme().sidebar
            })
            .hover(|style| style.bg(cx.theme().sidebar_accent))
            .text_color(if selected {
                cx.theme().sidebar_accent_foreground
            } else {
                cx.theme().sidebar_foreground
            })
            .cursor_pointer()
            .on_drag(FolderDrag(folder.to_owned()), |drag, _, _, cx| {
                cx.new(|_| FolderDragPreview {
                    name: SharedString::from(drag.0.clone()),
                })
            })
            .can_drop(|drag, _, _| drag.is::<FolderDrag>())
            .on_drop(cx.listener(move |this, drag: &FolderDrag, _, cx| {
                this.reorder_folder_before(&drag.0, &folder_for_drop, cx);
            }))
            .context_menu(move |menu, window, cx| {
                ReaderWindow::folder_context_menu(
                    menu,
                    window,
                    cx,
                    &folder_for_menu,
                    &menu_app,
                    language,
                    &selected_icon,
                )
            })
            .child(
                div()
                    .id(("folder-toggle", folder_key))
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_1()
                    .py_1()
                    .cursor_pointer()
                    .child(
                        Icon::new(if folder_collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .small(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_folder_collapsed(&folder_for_toggle, cx)
                    })),
            )
            .child(
                h_flex()
                    .id(("folder", folder_key))
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .pr_2()
                    .py_1()
                    .cursor_pointer()
                    .child(folder_icon)
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_sm()
                            .child(folder.to_owned()),
                    )
                    .when(unread > 0, |row| {
                        row.child(div().text_xs().child(unread.to_string()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_scope(Scope::Folder(folder_for_scope.clone()), cx)
                    })),
            )
    }

    pub(in crate::ui) fn render_sidebar(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let sidebar_bg = cx.theme().sidebar;
        let sidebar_text = cx.theme().sidebar_foreground;
        let mut folders: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (index, feed) in self.feeds.iter().enumerate() {
            folders
                .entry(feed.folder.clone().unwrap_or_default())
                .or_default()
                .push(index);
        }
        for feeds in folders.values_mut() {
            feeds.sort_by_cached_key(|index| self.feeds[*index].title.to_lowercase());
        }
        let mut folder_names = folders
            .keys()
            .filter(|name| !name.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        folder_names.sort_by_key(|name| name.to_lowercase());
        let mut folder_order = owner
            .preferences
            .folder_order
            .iter()
            .filter(|name| folder_names.iter().any(|folder| folder == *name))
            .cloned()
            .collect::<Vec<_>>();
        for name in &folder_names {
            if !folder_order.iter().any(|existing| existing == name) {
                folder_order.push(name.clone());
            }
        }

        let mut rows = Vec::with_capacity(self.feeds.len() + folder_order.len());
        if let Some(root_feeds) = folders.remove("") {
            rows.extend(root_feeds.into_iter().map(|feed_index| SidebarRow::Feed {
                feed_index,
                nested: false,
                has_error: self.feeds[feed_index].last_error.is_some(),
            }));
        }
        for folder in folder_order {
            let Some(feeds) = folders.remove(&folder) else {
                continue;
            };
            let collapsed = owner
                .preferences
                .collapsed_folders
                .iter()
                .any(|name| name == &folder);
            let unread = feeds.iter().map(|index| self.feeds[*index].unread).sum();
            rows.push(SidebarRow::Folder {
                name: folder,
                unread,
                collapsed,
            });
            if !collapsed {
                rows.extend(feeds.into_iter().map(|feed_index| SidebarRow::Feed {
                    feed_index,
                    nested: true,
                    has_error: self.feeds[feed_index].last_error.is_some(),
                }));
            }
        }
        let item_sizes = Rc::new(
            rows.iter()
                .map(|row| size(px(228.), px(row.height() + 2.)))
                .collect::<Vec<_>>(),
        );
        let rows = Rc::new(rows);
        let scroll = self.scroll.clone();
        let list_owner = cx.entity().clone();
        let feed_list = v_virtual_list(
            list_owner,
            "feed-list",
            item_sizes,
            move |this, range, _window, cx| {
                range
                    .map(|index| match &rows[index] {
                        SidebarRow::Feed {
                            feed_index, nested, ..
                        } => this
                            .sidebar
                            .feeds
                            .get(*feed_index)
                            .map(|feed| {
                                this.sidebar
                                    .render_feed_row(this, feed, *nested, &this.list.scope, cx)
                                    .into_any_element()
                            })
                            .unwrap_or_else(|| div().into_any_element()),
                        SidebarRow::Folder {
                            name,
                            unread,
                            collapsed,
                        } => this
                            .sidebar
                            .render_folder_row(this, name, *unread, *collapsed, cx)
                            .into_any_element(),
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&scroll)
        .size_full();
        v_flex()
            .w(px(228.))
            .h_full()
            .font(sidebar_font())
            .bg(sidebar_bg)
            .text_color(sidebar_text)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(div().h(px(8.)))
            .child(
                v_flex()
                    .gap_0p5()
                    .px_2()
                    .child(self.scope_row(
                        owner,
                        "scope-all",
                        "All Articles",
                        IconName::BookOpen,
                        Scope::All,
                        cx,
                    ))
                    .child(self.scope_row(
                        owner,
                        "scope-unread",
                        "Unread",
                        IconName::Inbox,
                        Scope::Unread,
                        cx,
                    ))
                    .child(self.scope_row(
                        owner,
                        "scope-starred",
                        "Starred",
                        IconName::Star,
                        Scope::Starred,
                        cx,
                    ))
                    .child(self.scope_row(
                        owner,
                        "scope-later",
                        "Read Later",
                        IconName::Inbox,
                        Scope::Later,
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .pt_4()
                    .pb_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(owner.t("Feeds"))
                    .child(
                        Button::new("refresh")
                            .small()
                            .ghost()
                            .icon(bundled_icon(BundledIcon::Refresh))
                            .tooltip(owner.t("Refresh feeds (Shift-click to force refresh)"))
                            .loading(self.is_refreshing)
                            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                                if event.modifiers().shift {
                                    this.refresh_with_force(true, cx);
                                } else {
                                    this.refresh(cx);
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .id("feed-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .px_2()
                    .child(feed_list)
                    .vertical_scrollbar(&scroll),
            )
    }

    pub(in crate::ui) fn scope_row(
        &self,
        owner: &ReaderWindow,
        id: &'static str,
        label: &'static str,
        icon: IconName,
        scope: Scope,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let selected = owner.list.scope == scope;
        let scope_for_click = scope.clone();
        h_flex()
            .id(id)
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().sidebar_accent))
            .bg(if selected {
                cx.theme().sidebar_accent
            } else {
                cx.theme().sidebar
            })
            .text_color(if selected {
                cx.theme().sidebar_accent_foreground
            } else {
                cx.theme().sidebar_foreground
            })
            .child(match &scope {
                Scope::All => bundled_icon(BundledIcon::ListFlat).small(),
                Scope::Starred => bundled_icon(BundledIcon::Star).small(),
                Scope::Later => bundled_icon(BundledIcon::Bookmark).small(),
                Scope::Folder(_) => Icon::new(IconName::FolderClosed).small(),
                _ => Icon::new(icon).small(),
            })
            .child(div().flex_1().text_sm().child(owner.t(label)))
            .on_click(
                cx.listener(move |this, _, _, cx| this.select_scope(scope_for_click.clone(), cx)),
            )
    }
}
