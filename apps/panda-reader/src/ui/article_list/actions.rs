use crate::services::Command;
use crate::ui::window::ReaderWindow;
use gpui_kit::*;
use panda_core::{ArticleCursor, Scope};
use std::sync::Arc;
use tokio::sync::oneshot;

const ARTICLE_PAGE: i64 = 20;

impl ReaderWindow {
    pub(in crate::ui) fn select_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.reader.request_epoch.next();
        self.reader.is_extracting = false;
        self.list.scope = scope;
        self.reader.article = None;
        self.reader.body_html = SharedString::default();
        self.reader.showing_translation = false;
        self.list.articles = Arc::new(Vec::new());
        self.list.has_more = false;
        self.load_snapshot(cx);
    }

    pub(in crate::ui) fn load_snapshot(&mut self, cx: &mut Context<Self>) {
        self.list.is_loading_more = false;
        self.load_snapshot_page(false, cx);
    }

    pub(in crate::ui) fn load_snapshot_page(&mut self, append: bool, cx: &mut Context<Self>) {
        self.list.snapshot_revision = self.list.snapshot_revision.wrapping_add(1);
        let revision = self.list.snapshot_revision;
        let after = if append {
            self.list.articles.last().map(ArticleCursor::from_summary)
        } else {
            None
        };
        let (reply, response) = oneshot::channel();
        self.services.send(Command::Snapshot {
            scope: self.list.scope.clone(),
            search: self.list.search.clone(),
            limit: ARTICLE_PAGE,
            after,
            include_feeds: !append,
            reply,
        });
        if !append {
            self.list.is_loading = self.list.articles.is_empty();
        }
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if revision != this.list.snapshot_revision {
                    return;
                }
                this.list.is_loading = false;
                this.list.is_loading_more = false;
                match result {
                    Ok(snapshot) => {
                        let newly_loaded = snapshot.articles.clone();
                        if !append {
                            this.sidebar.feeds = snapshot.feeds;
                            this.sync_folder_order_from_feeds();
                            this.ensure_favicons(cx);
                            this.list.articles = Arc::new(snapshot.articles);
                        } else if !snapshot.articles.is_empty() {
                            let articles = Arc::make_mut(&mut this.list.articles);
                            articles.extend(snapshot.articles);
                        }
                        this.list.has_more = snapshot.has_more;
                        if let Some(article) = &mut this.reader.article {
                            article.summary = this
                                .list
                                .articles
                                .iter()
                                .find(|row| row.id == article.summary.id)
                                .cloned()
                                .unwrap_or_else(|| article.summary.clone());
                        }
                        this.queue_title_translations(&newly_loaded, cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl ReaderWindow {
    pub(in crate::ui) fn queue_title_translations(
        &mut self,
        rows: &[panda_core::ArticleSummary],
        cx: &mut Context<Self>,
    ) {
        if !self.preferences.auto_translate_titles || rows.is_empty() {
            return;
        }
        let items: Vec<_> = rows
            .iter()
            .filter(|r| r.feed_auto_translate_titles)
            .map(|r| crate::services::TitleTranslationInput {
                id: r.id,
                title: r.title.clone(),
            })
            .collect();
        if items.is_empty() {
            return;
        }
        let target = self
            .preferences
            .translation_language
            .translator_code()
            .to_owned();
        let (reply, response) = oneshot::channel();
        self.services.send(Command::TranslateTitles {
            items,
            target_lang: target.clone(),
            reply,
        });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".into()));
            let _ = this.update(cx, |this, cx| match result {
                Ok(outcome) => {
                    this.title_translation_usage = outcome.usage;
                    let status = outcome.status.clone();
                    if this.preferences.auto_translate_titles
                        && this.preferences.translation_language.translator_code() == target
                    {
                        for (id, original, translated, lang) in outcome.translated {
                            if let Some(row) = Arc::make_mut(&mut this.list.articles)
                                .iter_mut()
                                .find(|r| r.id == id && r.title == original)
                            {
                                row.auto_translated_title = Some(translated.clone());
                                row.auto_translated_title_lang = Some(lang.clone());
                                row.auto_translated_title_source_hash =
                                    Some(panda_translate::title_source_hash(&original));
                            }
                            if let Some(article) = this
                                .reader
                                .article
                                .as_mut()
                                .filter(|a| a.summary.id == id && a.summary.title == original)
                            {
                                article.summary.auto_translated_title = Some(translated);
                                article.summary.auto_translated_title_lang = Some(lang);
                                article.summary.auto_translated_title_source_hash =
                                    Some(panda_translate::title_source_hash(&original));
                            }
                        }
                    }
                    if let Some(status) = status {
                        match status {
                            crate::services::TitleTranslationStatus::Info(message) => {
                                this.set_flash(message, cx);
                            }
                            crate::services::TitleTranslationStatus::Error(message) => {
                                this.set_error(message);
                            }
                        }
                    }
                    cx.notify();
                }
                Err(error) => this.set_error(format!("Title translation failed: {error}")),
            });
        })
        .detach();
    }

    pub(in crate::ui) fn load_title_translation_usage(&mut self, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.services.send(Command::TranslationUsage { reply });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(usage)) = response.await {
                let _ = this.update(cx, |this, cx| {
                    this.title_translation_usage = usage;
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

impl ReaderWindow {
    pub(in crate::ui) fn move_article(
        &mut self,
        delta: isize,
        unread_only: bool,
        cx: &mut Context<Self>,
    ) {
        if self.list.articles.is_empty() {
            return;
        }
        let current = self
            .reader
            .article
            .as_ref()
            .map(|article| article.summary.id);
        let current_index =
            current.and_then(|id| self.list.articles.iter().position(|row| row.id == id));
        let next_id = if unread_only {
            match (delta > 0, current_index) {
                (true, Some(index)) => self.list.articles[index + 1..]
                    .iter()
                    .find(|row| !row.is_read)
                    .map(|row| row.id),
                (true, None) => self
                    .list
                    .articles
                    .iter()
                    .find(|row| !row.is_read)
                    .map(|row| row.id),
                (false, Some(index)) => self.list.articles[..index]
                    .iter()
                    .rev()
                    .find(|row| !row.is_read)
                    .map(|row| row.id),
                (false, None) => self
                    .list
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
                    if next < 0 || next >= self.list.articles.len() as isize {
                        return;
                    }
                    next as usize
                }
                None if delta > 0 => 0,
                None => self.list.articles.len().saturating_sub(1),
            };
            Some(self.list.articles[next_index].id)
        };
        if let Some(id) = next_id {
            if let Some(index) = self.list.articles.iter().position(|row| row.id == id) {
                self.list
                    .scroll
                    .scroll_to_item(index, ScrollStrategy::Center);
            }
            self.open_article(id, cx);
        }
    }

    pub(in crate::ui) fn load_more_articles(&mut self, cx: &mut Context<Self>) {
        if !self.can_load_more() || self.list.is_loading_more {
            return;
        }
        self.list.is_loading_more = true;
        self.load_snapshot_page(true, cx);
    }

    pub(in crate::ui) fn can_load_more(&self) -> bool {
        self.list.has_more
    }
}
