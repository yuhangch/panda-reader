use crate::services::favicon::{feed_host, fetch_favicon, local_favicon_path};
use crate::ui::components::{BundledIcon, bundled_icon};
use crate::ui::window::ReaderWindow;
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::*;

impl ReaderWindow {
    pub(in crate::ui) fn unread_total(&self) -> i64 {
        self.sidebar.feeds.iter().map(|feed| feed.unread).sum()
    }

    pub(in crate::ui) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.preferences.sidebar_collapsed = !self.preferences.sidebar_collapsed;
        self.save_preferences();
        cx.notify();
    }

    pub(in crate::ui) fn toggle_folder_collapsed(&mut self, folder: &str, cx: &mut Context<Self>) {
        if let Some(index) = self
            .preferences
            .collapsed_folders
            .iter()
            .position(|name| name == folder)
        {
            self.preferences.collapsed_folders.remove(index);
        } else {
            self.preferences.collapsed_folders.push(folder.to_owned());
        }
        self.save_preferences();
        cx.notify();
    }

    pub(in crate::ui) fn sync_folder_order(&mut self, folder_names: &[String]) {
        let mut order = self
            .preferences
            .folder_order
            .iter()
            .filter(|name| folder_names.iter().any(|n| n == *name))
            .cloned()
            .collect::<Vec<_>>();
        for name in folder_names {
            if !order.iter().any(|existing| existing == name) {
                order.push(name.clone());
            }
        }
        if order != self.preferences.folder_order {
            self.preferences.folder_order = order;
            self.save_preferences();
        }
    }

    pub(in crate::ui) fn sync_folder_order_from_feeds(&mut self) {
        let mut names = self
            .sidebar
            .feeds
            .iter()
            .filter_map(|feed| feed.folder.clone())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        self.sync_folder_order(&names);
    }

    pub(in crate::ui) fn reorder_folder_before(
        &mut self,
        dragged: &str,
        target: &str,
        cx: &mut Context<Self>,
    ) {
        if dragged == target || dragged.is_empty() || target.is_empty() {
            return;
        }
        self.sync_folder_order_from_feeds();
        let order = &mut self.preferences.folder_order;
        let Some(from) = order.iter().position(|name| name == dragged) else {
            return;
        };
        order.remove(from);
        let Some(to) = order.iter().position(|name| name == target) else {
            order.push(dragged.to_owned());
            self.save_preferences();
            cx.notify();
            return;
        };
        order.insert(to, dragged.to_owned());
        self.save_preferences();
        cx.notify();
    }

    pub(in crate::ui) fn move_folder(
        &mut self,
        folder: &str,
        delta: isize,
        cx: &mut Context<Self>,
    ) {
        let order = &mut self.preferences.folder_order;
        let Some(index) = order.iter().position(|name| name == folder) else {
            return;
        };
        let next = index as isize + delta;
        if next < 0 || next >= order.len() as isize {
            return;
        }
        order.swap(index, next as usize);
        self.save_preferences();
        cx.notify();
    }

    pub(in crate::ui) fn set_folder_icon(
        &mut self,
        folder: &str,
        icon: &str,
        cx: &mut Context<Self>,
    ) {
        self.preferences
            .folder_icons
            .insert(folder.to_owned(), icon.to_owned());
        self.save_preferences();
        cx.notify();
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

    pub(in crate::ui) fn folder_icon_key(&self, folder: &str) -> &str {
        self.preferences
            .folder_icons
            .get(folder)
            .map(String::as_str)
            .unwrap_or("folder")
    }

    pub(in crate::ui) fn ensure_favicons(&mut self, cx: &mut Context<Self>) {
        for feed in &self.sidebar.feeds {
            let Some(host) = feed_host(feed.site_url.as_deref(), &feed.feed_url) else {
                continue;
            };
            if local_favicon_path(&self.sidebar.icons_dir, &host).is_some() {
                continue;
            }
            if !self.sidebar.favicon_requested.insert(host.clone()) {
                continue;
            }
            let site_url = feed
                .site_url
                .clone()
                .unwrap_or_else(|| feed.feed_url.clone());
            let icons_dir = self.sidebar.icons_dir.clone();
            let weak = cx.entity().downgrade();
            cx.spawn(async move |_, cx| {
                let result = fetch_favicon(&host, &site_url, &icons_dir).await;
                if let Err(error) = result {
                    eprintln!("could not fetch feed icon for {host}: {error}");
                }
                let _ = weak.update(cx, |_, cx| {
                    cx.notify();
                });
            })
            .detach();
        }
    }
}
