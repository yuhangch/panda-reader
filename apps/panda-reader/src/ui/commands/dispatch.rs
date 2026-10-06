use crate::ui::commands::CommandKind;
use crate::ui::settings::SettingsPage;
use crate::ui::window::ReaderWindow;
use gpui_kit::*;
use panda_core::{MarkField, Scope};

impl ReaderWindow {
    pub(in crate::ui) fn run_command(
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
                if let Some(article) = self.reader.article.as_ref() {
                    let id = article.summary.id;
                    let value = !article.summary.is_read;
                    self.mark(id, MarkField::Read, value, cx);
                }
            }
            CommandKind::ToggleStar => {
                if let Some(article) = self.reader.article.as_ref() {
                    let id = article.summary.id;
                    let value = !article.summary.is_starred;
                    self.mark(id, MarkField::Starred, value, cx);
                }
            }
            CommandKind::ToggleLater => {
                if let Some(article) = self.reader.article.as_ref() {
                    let id = article.summary.id;
                    let value = !article.summary.read_later;
                    self.mark(id, MarkField::Later, value, cx);
                }
            }
            CommandKind::ExtractFullText => {
                if let Some(article) = self.reader.article.as_ref() {
                    self.extract(article.summary.id, true, cx);
                }
            }
            CommandKind::Translate => {
                if let Some(article) = self.reader.article.as_ref() {
                    self.toggle_or_translate(article.summary.id, cx);
                }
            }
            CommandKind::ToggleHideImages => self.toggle_hide_images(cx),
            CommandKind::ToggleTranslationLayout => self.toggle_translation_layout(cx),
            CommandKind::OpenOriginal => {
                if let Some(url) = self
                    .reader
                    .article
                    .as_ref()
                    .and_then(|article| article.url.clone())
                {
                    cx.open_url(&url);
                }
            }
            CommandKind::CopyLink => {
                if let Some(url) = self
                    .reader
                    .article
                    .as_ref()
                    .and_then(|article| article.url.clone())
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                    self.set_flash(self.t("Link copied"), cx);
                }
            }
            CommandKind::CopyTitle => {
                if let Some(title) = self
                    .reader
                    .article
                    .as_ref()
                    .map(|a| a.summary.title.clone())
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(title));
                    self.set_flash(self.t("Title copied"), cx);
                }
            }
            CommandKind::Refresh => self.refresh(cx),
            CommandKind::ForceRefreshArticle => {
                if let Some(id) = self
                    .reader
                    .article
                    .as_ref()
                    .map(|article| article.summary.id)
                {
                    self.extract(id, true, cx);
                } else {
                    self.set_flash(self.t("Select an article first"), cx);
                }
            }
            CommandKind::AddFeed => {
                self.settings.open = true;
                self.settings.page = SettingsPage::General;
                self.settings.theme_picker_open = false;
                let _ = self
                    .settings
                    .add_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
            }
            CommandKind::MarkAllRead => self.mark_all_read(cx),
            CommandKind::LoadMoreArticles => self.load_more_articles(cx),
            CommandKind::ToggleSidebar => self.toggle_sidebar(cx),
            CommandKind::OpenSettings => {
                self.settings.open = true;
                self.settings.page = SettingsPage::General;
                self.settings.theme_picker_open = false;
                cx.notify();
            }
            CommandKind::OpenAppearance => {
                self.settings.open = true;
                self.settings.page = SettingsPage::Appearance;
                self.settings.theme_picker_open = false;
                cx.notify();
            }
            CommandKind::OpenReading => {
                self.settings.open = true;
                self.settings.page = SettingsPage::Reading;
                self.settings.theme_picker_open = false;
                cx.notify();
            }
            CommandKind::OpenAbout => {
                self.settings.open = true;
                self.settings.page = SettingsPage::About;
                self.settings.theme_picker_open = false;
                cx.notify();
            }
            CommandKind::FocusSearch => {
                self.settings.open = false;
                let _ = self
                    .list
                    .search_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
            }
            CommandKind::ShowKeyboardShortcuts => self.open_palette(window, cx),
        }
    }

    pub(in crate::ui) fn typing_in_input(&self, window: &Window, cx: &App) -> bool {
        [
            &self.list.search_input,
            &self.settings.add_input,
            &self.settings.provider_url_input,
            &self.settings.provider_username_input,
            &self.settings.provider_secret_input,
            &self.settings.azure_key_input,
            &self.settings.azure_region_input,
            &self.settings.volcengine_ak_input,
            &self.settings.volcengine_sk_input,
            &self.palette.input,
            &self.editor.title_input,
            &self.editor.folder_input,
            &self.editor.url_input,
        ]
        .into_iter()
        .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    pub(in crate::ui) fn vim_allowed(&self, window: &Window, cx: &App) -> bool {
        self.preferences.vim_navigation && !self.palette.open && !self.typing_in_input(window, cx)
    }

    pub(in crate::ui) fn handle_chord(
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

    pub(in crate::ui) fn handle_vim(
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
