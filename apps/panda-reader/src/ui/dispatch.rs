//! Shared command dispatch for palette, chords, and vim keys.

use super::commands::CommandKind;
use super::*;
use gpui_kit::ClipboardItem;
use std::path::{Path, PathBuf};

impl ReaderView {
    pub(super) fn run_command(
        &mut self,
        kind: CommandKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match kind {
            CommandKind::TogglePalette => self.toggle_palette(window, cx),
            CommandKind::NextArticle => self.move_article(1, false, cx),
            CommandKind::PreviousArticle => self.move_article(-1, false, cx),
            CommandKind::NextUnread => self.move_article(1, true, cx),
            CommandKind::PreviousUnread => self.move_article(-1, true, cx),
            CommandKind::GoAll => self.select_scope(Scope::All, cx),
            CommandKind::GoUnread => self.select_scope(Scope::Unread, cx),
            CommandKind::GoStarred => self.select_scope(Scope::Starred, cx),
            CommandKind::GoLater => self.select_scope(Scope::Later, cx),
            CommandKind::ToggleRead => {
                if let Some(article) = self.article.as_ref() {
                    let id = article.summary.id;
                    let value = !article.summary.is_read;
                    self.mark(id, MarkField::Read, value, cx);
                }
            }
            CommandKind::ToggleStar => {
                if let Some(article) = self.article.as_ref() {
                    let id = article.summary.id;
                    let value = !article.summary.is_starred;
                    self.mark(id, MarkField::Starred, value, cx);
                }
            }
            CommandKind::ToggleLater => {
                if let Some(article) = self.article.as_ref() {
                    let id = article.summary.id;
                    let value = !article.summary.read_later;
                    self.mark(id, MarkField::Later, value, cx);
                }
            }
            CommandKind::ExtractFullText => {
                if let Some(article) = self.article.as_ref() {
                    self.extract(article.summary.id, true, cx);
                }
            }
            CommandKind::Translate => {
                if let Some(article) = self.article.as_ref() {
                    self.toggle_or_translate(article.summary.id, cx);
                }
            }
            CommandKind::ToggleHideImages => self.toggle_hide_images(cx),
            CommandKind::ToggleTranslationLayout => self.toggle_translation_layout(cx),
            CommandKind::OpenOriginal => {
                if let Some(url) = self
                    .article
                    .as_ref()
                    .and_then(|article| article.url.clone())
                {
                    cx.open_url(&url);
                }
            }
            CommandKind::CopyLink => {
                if let Some(url) = self
                    .article
                    .as_ref()
                    .and_then(|article| article.url.clone())
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                    self.set_flash(self.t("Link copied"), cx);
                }
            }
            CommandKind::CopyTitle => {
                if let Some(title) = self.article.as_ref().map(|a| a.summary.title.clone()) {
                    cx.write_to_clipboard(ClipboardItem::new_string(title));
                    self.set_flash(self.t("Title copied"), cx);
                }
            }
            CommandKind::Refresh => self.refresh(cx),
            CommandKind::AddFeed => {
                self.settings_open = true;
                self.settings_page = SettingsPage::General;
                self.theme_picker_open = false;
                let _ = self.add_input.read(cx).focus_handle(cx).focus(window, cx);
                cx.notify();
            }
            CommandKind::MarkAllRead => self.mark_all_read(cx),
            CommandKind::LoadMoreArticles => self.load_more_articles(cx),
            CommandKind::ToggleSidebar => self.toggle_sidebar(cx),
            CommandKind::OpenSettings => {
                self.settings_open = true;
                self.settings_page = SettingsPage::General;
                self.theme_picker_open = false;
                cx.notify();
            }
            CommandKind::OpenAppearance => {
                self.settings_open = true;
                self.settings_page = SettingsPage::Appearance;
                self.theme_picker_open = false;
                cx.notify();
            }
            CommandKind::OpenReading => {
                self.settings_open = true;
                self.settings_page = SettingsPage::Reading;
                self.theme_picker_open = false;
                cx.notify();
            }
            CommandKind::OpenAbout => {
                self.settings_open = true;
                self.settings_page = SettingsPage::About;
                self.theme_picker_open = false;
                cx.notify();
            }
            CommandKind::FocusSearch => {
                self.settings_open = false;
                let _ = self
                    .search_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
            }
            CommandKind::ShowKeyboardShortcuts => self.open_palette(window, cx),
        }
    }

    pub(super) fn move_article(&mut self, delta: isize, unread_only: bool, cx: &mut Context<Self>) {
        if self.articles.is_empty() {
            return;
        }
        let current = self.article.as_ref().map(|article| article.summary.id);
        let current_index =
            current.and_then(|id| self.articles.iter().position(|row| row.id == id));
        let next_id = if unread_only {
            match (delta > 0, current_index) {
                (true, Some(index)) => self.articles[index + 1..]
                    .iter()
                    .find(|row| !row.is_read)
                    .map(|row| row.id),
                (true, None) => self
                    .articles
                    .iter()
                    .find(|row| !row.is_read)
                    .map(|row| row.id),
                (false, Some(index)) => self.articles[..index]
                    .iter()
                    .rev()
                    .find(|row| !row.is_read)
                    .map(|row| row.id),
                (false, None) => self
                    .articles
                    .iter()
                    .rev()
                    .find(|row| !row.is_read)
                    .map(|row| row.id),
            }
        } else {
            let next_index = match current_index {
                Some(index) => {
                    let next = index as isize + delta;
                    if next < 0 || next >= self.articles.len() as isize {
                        return;
                    }
                    next as usize
                }
                None if delta > 0 => 0,
                None => self.articles.len().saturating_sub(1),
            };
            Some(self.articles[next_index].id)
        };
        if let Some(id) = next_id {
            if let Some(index) = self.articles.iter().position(|row| row.id == id) {
                self.article_list_scroll
                    .scroll_to_item(index, ScrollStrategy::Center);
            }
            self.open_article(id, cx);
        }
    }

    pub(super) fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::MarkAllRead {
            scope: self.scope.clone(),
            reply,
        });
        self.set_busy(self.t("Marking all as read…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(count) => {
                        this.set_flash(
                            i18n::format(
                                this.preferences.language,
                                "Marked {} articles as read",
                                count,
                            ),
                            cx,
                        );
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn load_more_articles(&mut self, cx: &mut Context<Self>) {
        if !self.can_load_more() || self.is_loading_more {
            return;
        }
        self.is_loading_more = true;
        self.load_snapshot_page(true, cx);
    }

    pub(super) fn can_load_more(&self) -> bool {
        self.articles_has_more
    }

    pub(super) fn update_feed_fields(
        &mut self,
        id: i64,
        title: String,
        folder: Option<String>,
        feed_url: String,
        cx: &mut Context<Self>,
    ) {
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::UpdateFeed {
            id,
            title,
            folder,
            feed_url,
            reply,
        });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.editing_feed = None;
                        this.set_flash(this.t("Feed updated"), cx);
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_one_feed(&mut self, id: i64, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.backend.send(Command::RefreshFeed { id, reply });
        self.set_busy(self.t("Refreshing feed…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.set_flash(this.t("Feed refreshed"), cx);
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn typing_in_input(&self, window: &Window, cx: &App) -> bool {
        [
            &self.search_input,
            &self.add_input,
            &self.provider_url_input,
            &self.provider_username_input,
            &self.provider_secret_input,
            &self.azure_key_input,
            &self.azure_region_input,
            &self.volcengine_ak_input,
            &self.volcengine_sk_input,
            &self.palette.input,
            &self.feed_title_input,
            &self.feed_folder_input,
            &self.feed_url_input,
        ]
        .into_iter()
        .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    pub(super) fn vim_allowed(&self, window: &Window, cx: &App) -> bool {
        self.preferences.vim_navigation && !self.palette.open && !self.typing_in_input(window, cx)
    }

    pub(super) fn begin_edit_feed(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(feed) = self.feeds.iter().find(|feed| feed.id == id).cloned() else {
            return;
        };
        self.editing_feed = Some(id);
        let _ = self.feed_title_input.update(cx, |input, cx| {
            input.set_value(feed.title.as_str(), window, cx);
            input.focus_handle(cx).focus(window, cx);
        });
        let _ = self.feed_folder_input.update(cx, |input, cx| {
            input.set_value(feed.folder.as_deref().unwrap_or(""), window, cx);
        });
        let _ = self.feed_url_input.update(cx, |input, cx| {
            input.set_value(feed.feed_url.as_str(), window, cx);
        });
        cx.notify();
    }

    pub(super) fn save_feed_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.editing_feed else {
            return;
        };
        let title = self.feed_title_input.read(cx).value().to_string();
        let folder = {
            let value = self.feed_folder_input.read(cx).value().to_string();
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        };
        let feed_url = self.feed_url_input.read(cx).value().to_string();
        let _ = window;
        self.update_feed_fields(id, title, folder, feed_url, cx);
    }

    pub(super) fn render_feed_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scrim = cx.theme().background.opacity(0.55);
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(scrim)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.editing_feed = None;
                    cx.notify();
                }),
            )
            .child(
                v_flex()
                    .w(px(440.))
                    .gap_3()
                    .p_5()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(div().text_lg().font_semibold().child(self.t("Edit feed")))
                    .child(div().text_sm().child(self.t("Title")))
                    .child(text_input(&self.feed_title_input))
                    .child(div().text_sm().child(self.t("Folder")))
                    .child(text_input(&self.feed_folder_input))
                    .child(div().text_sm().child(self.t("Feed URL")))
                    .child(text_input(&self.feed_url_input))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .pt_2()
                            .child(
                                Button::new("feed-edit-cancel")
                                    .small()
                                    .secondary()
                                    .label(self.t("Cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.editing_feed = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("feed-edit-save")
                                    .small()
                                    .primary()
                                    .label(self.t("Save"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_feed_editor(window, cx);
                                    })),
                            ),
                    ),
            )
    }

    pub(super) fn handle_chord(
        &mut self,
        kind: CommandKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.open && !matches!(kind, CommandKind::TogglePalette) {
            return;
        }
        self.run_command(kind, window, cx);
    }

    pub(super) fn handle_vim(
        &mut self,
        kind: CommandKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.vim_allowed(window, cx) {
            return;
        }
        self.run_command(kind, window, cx);
    }
}

pub(super) fn letter_avatar(title: &str) -> String {
    title
        .chars()
        .find(|ch| ch.is_alphanumeric())
        .map(|ch| ch.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into())
}

pub(super) fn host_from_url(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host = rest.split(['/', '?', '#']).next()?.trim();
    let host = host.split('@').next_back()?.trim();
    (!host.is_empty()).then_some(host)
}

pub(super) fn sanitize_icon_host(host: &str) -> String {
    host.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn local_favicon_path(icons_dir: &Path, host: &str) -> Option<PathBuf> {
    let path = icons_dir.join(format!("{}.png", sanitize_icon_host(host)));
    path.is_file().then_some(path)
}

pub(super) fn feed_host(site_url: Option<&str>, feed_url: &str) -> Option<String> {
    let candidate = site_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(feed_url);
    host_from_url(candidate).map(str::to_owned)
}
