use crate::services::Command;
use crate::ui::article_view::ArticleView;
use crate::ui::i18n;
use crate::ui::settings::SettingsPage;
use crate::ui::window::ReaderWindow;
use gpui_kit::*;
use panda_core::{MarkField, Scope, TranslationLayout};
use std::sync::Arc;
use tokio::sync::oneshot;

impl ReaderWindow {
    pub(in crate::ui) fn copy_article_image(
        &mut self,
        image: Arc<gpui_kit::gpui::Image>,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_image(&image));
        self.set_flash(self.t("Image copied"), cx);
    }
}

impl ArticleView {
    pub(in crate::ui) fn load_article_image(
        &mut self,
        url: &str,
        cx: &mut Context<ArticleView>,
    ) -> Option<Arc<gpui_kit::gpui::Image>> {
        if let Some(image) = self.article_image_cache.get(url) {
            return Some(image.clone());
        }
        if !self.article_images_loading.insert(url.to_owned()) {
            return None;
        }

        const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
        use futures::AsyncReadExt as _;
        use gpui_kit::gpui::http_client::{AsyncBody, HttpRequestExt};

        let client = cx.http_client();
        let url = url.to_owned();
        cx.spawn(async move |this, cx| {
            let result = async {
                let request = gpui_kit::gpui::http_client::Request::builder()
                    .uri(&url)
                    .timeout(std::time::Duration::from_secs(15))
                    .body(AsyncBody::empty())
                    .map_err(|error| error.to_string())?;
                let mut response = client
                    .send(request)
                    .await
                    .map_err(|error| error.to_string())?;
                if !response.status().is_success() {
                    return Err(format!("image request returned {}", response.status()));
                }
                if response
                    .headers()
                    .get("content-length")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<usize>().ok())
                    .is_some_and(|size| size > MAX_IMAGE_BYTES)
                {
                    return Err("Image exceeds the 16 MiB copy limit".to_owned());
                }
                let mut bytes = Vec::new();
                response
                    .body_mut()
                    .take((MAX_IMAGE_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|error| error.to_string())?;
                if bytes.len() > MAX_IMAGE_BYTES {
                    return Err("Image exceeds the 16 MiB copy limit".to_owned());
                }
                cx.background_executor()
                    .spawn(async move {
                        let png = crate::services::image_decode::decode_image_png(&bytes)?;
                        Ok(Arc::new(gpui_kit::gpui::Image::from_bytes(
                            gpui_kit::gpui::ImageFormat::Png,
                            png,
                        )))
                    })
                    .await
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.article_images_loading.remove(&url);
                if let Ok(image) = result {
                    if this.article_image_cache.len() >= 32 {
                        if let Some(oldest) = this.article_image_cache_order.pop_front() {
                            this.article_image_cache.remove(&oldest);
                        }
                    }
                    this.article_image_cache_order.push_back(url.clone());
                    this.article_image_cache.insert(url, image);
                    cx.notify();
                }
            });
        })
        .detach();
        None
    }
}

impl ReaderWindow {
    pub(in crate::ui) fn share_article(&mut self, cx: &mut Context<Self>) {
        let Some(article) = self.read_article_reader(cx, |reader| reader.article.clone()) else {
            return;
        };
        let Some(url) = article.url.as_deref().or(article.summary.url.as_deref()) else {
            return;
        };
        let title = article.summary.title.as_str();
        let default_template = "{title}\\n{url}";
        let template = if self.preferences.share_template.trim().is_empty() {
            default_template
        } else {
            self.preferences.share_template.as_str()
        };
        let text = template
            .replace("{title}", title)
            .replace("{url}", url)
            .replace("\\n", "\n");
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
        self.set_flash(self.t("Share text copied"), cx);
    }

    pub(in crate::ui) fn open_article(&mut self, id: i64, cx: &mut Context<Self>) {
        self.save_current_reading_progress(cx);
        let (translating_id, previous_id, revision) = self.update_article_reader(cx, |reader| {
            reader.translation_epoch.next();
            let translating_id = reader.translating_article_id.take();
            let previous_id = reader.requested_article_id;
            reader.requested_article_id = Some(id);
            reader.show_loading_skeleton = false;
            let revision = reader.request_epoch.next();
            reader.progress_epoch = reader.progress_epoch.wrapping_add(1);
            reader.restore_progress = None;
            reader.image_viewer_url = None;
            reader.image_urls.clear();
            reader.scroll.set_offset(point(px(0.), px(0.)));
            reader.is_extracting = false;
            reader.translating_article_id = None;
            reader.translation_incomplete = false;
            reader.showing_translation = false;
            reader.body_html = SharedString::default();
            reader.body_markdown = SharedString::default();
            reader.article = None;
            (translating_id, previous_id, revision)
        });
        let reader = self.reader.clone();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            let _ = reader.update(cx, |reader, cx| {
                if reader.request_epoch.is_current(revision) && reader.article.is_none() {
                    reader.show_loading_skeleton = true;
                    cx.notify();
                }
            });
        })
        .detach();
        if let Some(translating_id) = translating_id {
            self.services.cancel_article_work(translating_id);
            self.clear_status();
        }
        if let Some(previous_id) = previous_id
            && previous_id != id
        {
            self.services.cancel_article_work(previous_id);
        }
        let (reply, response) = oneshot::channel();
        self.services.send(Command::Article {
            id,
            show_translation: false,
            translation_layout: self.preferences.translation_layout,
            hide_images: self.preferences.hide_images,
            paragraph_indent: self.preferences.paragraph_indent,
            reply,
        });
        let services = self.services.clone();
        // Optimistic list update — avoid a full snapshot round-trip.
        let newly_read_feed = self.update_article_list(cx, |list, _| {
            list.selected_article = Some(id);
            if let Some(summary) = Arc::make_mut(&mut list.articles)
                .iter_mut()
                .find(|article| article.id == id)
            {
                if !summary.is_read {
                    summary.is_read = true;
                    Some(summary.feed_id)
                } else {
                    None
                }
            } else {
                None
            }
        });
        if let Some(feed_id) = newly_read_feed {
            self.sidebar.update(cx, |sidebar, cx| {
                if let Some(feed) = sidebar.feeds.iter_mut().find(|feed| feed.id == feed_id) {
                    feed.unread = feed.unread.saturating_sub(1);
                }
                cx.notify();
            });
        }
        self.sync_status_bar(cx);
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if !this.read_article_reader(cx, |reader| reader.request_epoch.is_current(revision))
                {
                    return;
                }
                let needs_status_refresh = result.is_err();
                match result {
                    Ok(prepared) => {
                        let reading_progress = prepared.reading_progress;
                        let mut article = prepared.article;
                        article.summary.is_read = true;
                        let should_auto_extract = this.preferences.auto_extract_full_text
                            && article.extracted_html.is_none()
                            && article.url.is_some();
                        this.update_article_reader(cx, |reader| {
                            reader.show_loading_skeleton = false;
                            reader.showing_translation = false;
                            reader.body_html = prepared.body_html.into();
                            reader.body_markdown = prepared.body_markdown.into();
                            reader.image_urls = prepared.image_urls.to_vec();
                            reader.article = Some(article);
                        });
                        if this.preferences.remember_reading_position && reading_progress > 0. {
                            this.update_article_reader(cx, |reader| {
                                reader.restore_progress = Some(reading_progress)
                            });
                            this.restore_reading_progress(id, reading_progress, cx);
                        }
                        let (reply, _) = oneshot::channel();
                        services.send(Command::Mark {
                            id,
                            field: MarkField::Read,
                            value: true,
                            reply,
                        });
                        if should_auto_extract {
                            this.extract(id, false, cx);
                        }
                    }
                    Err(error) => {
                        this.update_article_reader(cx, |reader| {
                            reader.requested_article_id = None;
                            reader.show_loading_skeleton = false;
                        });
                        this.set_error(error);
                    }
                }
                if needs_status_refresh {
                    this.sync_status_bar(cx);
                }
            });
        })
        .detach();
    }

    pub(in crate::ui) fn schedule_reading_progress_save(&mut self, cx: &mut Context<Self>) {
        if !self.preferences.remember_reading_position
            || !self.read_article_reader(cx, |reader| reader.article.is_some())
        {
            return;
        }
        // The debounce revision is not visual state. Notifying the reader for
        // every wheel event rebuilds its rich-text view while the page scrolls.
        let epoch = self.reader.update(cx, |reader, _| {
            reader.progress_epoch = reader.progress_epoch.wrapping_add(1);
            reader.progress_epoch
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(700))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.read_article_reader(cx, |reader| reader.progress_epoch == epoch) {
                    this.save_current_reading_progress(cx);
                }
            });
        })
        .detach();
    }

    fn save_current_reading_progress(&self, cx: &App) {
        if !self.preferences.remember_reading_position {
            return;
        }
        let Some((id, scroll)) = self.read_article_reader(cx, |reader| {
            reader
                .article
                .as_ref()
                .map(|article| (article.summary.id, reader.scroll.clone()))
        }) else {
            return;
        };
        let max = scroll.max_offset().y;
        if max <= px(0.) {
            return;
        }
        let progress = (-scroll.offset().y / max).clamp(0., 1.);
        let (reply, _response) = oneshot::channel();
        self.services
            .send(crate::services::Command::SaveReadingProgress {
                id,
                progress,
                reply,
            });
    }

    fn restore_reading_progress(&mut self, id: i64, progress: f32, cx: &mut Context<Self>) {
        let (epoch, scroll) = self.update_article_reader(cx, |reader| {
            reader.progress_epoch = reader.progress_epoch.wrapping_add(1);
            (reader.progress_epoch, reader.scroll.clone())
        });
        cx.spawn(async move |this, cx| {
            for _ in 0..20 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let restored = this
                    .update(cx, |this, cx| {
                        if !this.read_article_reader(cx, |reader| {
                            reader.progress_epoch == epoch
                                && reader
                                    .article
                                    .as_ref()
                                    .is_some_and(|article| article.summary.id == id)
                        }) {
                            return true;
                        }
                        let max = scroll.max_offset().y;
                        if max > px(0.) {
                            scroll.set_offset(point(px(0.), -max * progress));
                            this.update_article_reader(cx, |reader| reader.restore_progress = None);
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(true);
                if restored {
                    break;
                }
            }
        })
        .detach();
    }

    pub(in crate::ui) fn request_prepared_body(&mut self, cx: &mut Context<Self>) {
        let Some((article, show_translation)) = self.read_article_reader(cx, |reader| {
            reader
                .article
                .clone()
                .map(|article| (article, reader.showing_translation))
        }) else {
            return;
        };
        let id = article.summary.id;
        let revision = self.update_article_reader(cx, |reader| reader.request_epoch.next());
        let (reply, response) = oneshot::channel();
        self.services.send(Command::Article {
            id,
            show_translation,
            translation_layout: self.preferences.translation_layout,
            hide_images: self.preferences.hide_images,
            paragraph_indent: self.preferences.paragraph_indent,
            reply,
        });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if !this.read_article_reader(cx, |reader| reader.request_epoch.is_current(revision))
                {
                    return;
                }
                match result {
                    Ok(prepared) => {
                        this.update_article_reader(cx, |reader| {
                            reader.article = Some(prepared.article);
                            reader.body_html = prepared.body_html.into();
                            reader.body_markdown = prepared.body_markdown.into();
                            reader.image_urls = prepared.image_urls.to_vec();
                        });
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn mark(
        &mut self,
        id: i64,
        field: MarkField,
        value: bool,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .read_article_reader(cx, |reader| {
                reader
                    .article
                    .as_ref()
                    .filter(|article| article.summary.id == id)
                    .map(|article| (article.summary.feed_id, article.summary.is_read))
            })
            .or_else(|| {
                self.list
                    .read(cx)
                    .articles
                    .iter()
                    .find(|article| article.id == id)
                    .map(|article| (article.feed_id, article.is_read))
            });
        let feed_id = current.map(|(feed_id, _)| feed_id);
        let old_read = current.map(|(_, is_read)| is_read).unwrap_or(!value);
        self.next_mark_revision = self.next_mark_revision.wrapping_add(1);
        let mark_revision = self.next_mark_revision;
        self.mark_revisions.insert((id, field), mark_revision);
        let scope = self.list.read(cx).scope.clone();
        let remove_from_scope = matches!(
            (&scope, field, value),
            (Scope::Unread, MarkField::Read, true)
                | (Scope::Starred, MarkField::Starred, false)
                | (Scope::Later, MarkField::Later, false)
        );
        self.update_article_reader(cx, |reader| {
            if let Some(article) = &mut reader.article {
                if article.summary.id == id {
                    match field {
                        MarkField::Read => article.summary.is_read = value,
                        MarkField::Starred => article.summary.is_starred = value,
                        MarkField::Later => article.summary.read_later = value,
                    }
                }
            }
        });
        self.update_article_list(cx, |list, _| {
            if let Some(summary) = Arc::make_mut(&mut list.articles)
                .iter_mut()
                .find(|article| article.id == id)
            {
                match field {
                    MarkField::Read => summary.is_read = value,
                    MarkField::Starred => summary.is_starred = value,
                    MarkField::Later => summary.read_later = value,
                }
            }
        });
        if field == MarkField::Read
            && old_read != value
            && let Some(feed_id) = feed_id
        {
            self.sidebar.update(cx, |sidebar, cx| {
                if let Some(feed) = sidebar.feeds.iter_mut().find(|feed| feed.id == feed_id) {
                    feed.unread = (feed.unread + if value { -1 } else { 1 }).max(0);
                }
                cx.notify();
            });
        }
        cx.notify();

        let (reply, response) = oneshot::channel();
        self.services.send(Command::Mark {
            id,
            field,
            value,
            reply,
        });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if this.mark_revisions.get(&(id, field)) != Some(&mark_revision) {
                    return;
                }
                if let Err(error) = result {
                    this.update_article_reader(cx, |reader| {
                        if let Some(article) = &mut reader.article {
                            if article.summary.id == id {
                                match field {
                                    MarkField::Read => article.summary.is_read = !value,
                                    MarkField::Starred => article.summary.is_starred = !value,
                                    MarkField::Later => article.summary.read_later = !value,
                                }
                            }
                        }
                    });
                    this.update_article_list(cx, |list, _| {
                        if let Some(summary) = Arc::make_mut(&mut list.articles)
                            .iter_mut()
                            .find(|article| article.id == id)
                        {
                            match field {
                                MarkField::Read => summary.is_read = !value,
                                MarkField::Starred => summary.is_starred = !value,
                                MarkField::Later => summary.read_later = !value,
                            }
                        }
                    });
                    if field == MarkField::Read
                        && old_read != value
                        && let Some(feed_id) = feed_id
                    {
                        this.sidebar.update(cx, |sidebar, cx| {
                            if let Some(feed) =
                                sidebar.feeds.iter_mut().find(|feed| feed.id == feed_id)
                            {
                                feed.unread = (feed.unread + if old_read { 1 } else { -1 }).max(0);
                            }
                            cx.notify();
                        });
                    }
                    this.set_error(error);
                } else if remove_from_scope {
                    this.update_article_list(cx, |list, _| {
                        Arc::make_mut(&mut list.articles).retain(|article| article.id != id);
                    });
                    this.load_snapshot(cx);
                }
                if this.mark_revisions.get(&(id, field)) == Some(&mark_revision) {
                    this.mark_revisions.remove(&(id, field));
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn extract(&mut self, id: i64, force: bool, cx: &mut Context<Self>) {
        let (is_extracting, has_full_text, show_translation) =
            self.read_article_reader(cx, |reader| {
                (
                    reader.is_extracting,
                    reader.article.as_ref().is_some_and(|article| {
                        article.summary.id == id
                            && article
                                .extracted_html
                                .as_deref()
                                .is_some_and(|html| !html.trim().is_empty())
                    }),
                    reader.showing_translation,
                )
            });
        if is_extracting {
            return;
        }
        if !force && has_full_text {
            return;
        }
        let (reply, response) = oneshot::channel();
        let extractor = self.preferences.content_extractor;
        let translation_layout = self.preferences.translation_layout;
        let hide_images = self.preferences.hide_images;
        let paragraph_indent = self.preferences.paragraph_indent;
        let revision = self.update_article_reader(cx, |reader| {
            reader.is_extracting = true;
            reader.request_epoch.next()
        });
        self.services.send(Command::Extract {
            id,
            force,
            extractor,
            reply,
        });
        self.set_busy(self.t("Extracting full text…"));
        cx.notify();
        let (article_reply, article_response) = oneshot::channel();
        let services = self.services.clone();
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            if result.is_ok() {
                services.send(Command::Article {
                    id,
                    show_translation,
                    translation_layout,
                    hide_images,
                    paragraph_indent,
                    reply: article_reply,
                });
            }
            let _ = this.update(cx, |this, cx| {
                if !this.read_article_reader(cx, |reader| reader.request_epoch.is_current(revision))
                {
                    return;
                }
                this.update_article_reader(cx, |reader| reader.is_extracting = false);
                match result {
                    Ok(()) => {
                        cx.spawn(async move |this, cx| {
                            let prepared = article_response
                                .await
                                .map_err(|_| "Background service stopped".to_owned())
                                .and_then(|result| result);
                            let _ = this.update(cx, |this, cx| {
                                if !this.read_article_reader(cx, |reader| {
                                    reader.request_epoch.is_current(revision)
                                        && reader
                                            .article
                                            .as_ref()
                                            .is_some_and(|current| current.summary.id == id)
                                }) {
                                    return;
                                }
                                match prepared {
                                    Ok(prepared) => {
                                        this.update_article_reader(cx, |reader| {
                                            reader.body_html = prepared.body_html.into();
                                            reader.body_markdown = prepared.body_markdown.into();
                                            reader.image_urls = prepared.image_urls.to_vec();
                                            reader.article = Some(prepared.article);
                                        });
                                        this.set_flash(this.t("Full text extracted"), cx);
                                    }
                                    Err(error) => this.set_error(error),
                                }
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn toggle_or_translate(&mut self, id: i64, cx: &mut Context<Self>) {
        let translating =
            self.read_article_reader(cx, |reader| reader.translating_article_id == Some(id));
        if translating {
            self.services.cancel_article_work(id);
            self.update_article_reader(cx, |reader| {
                reader.translation_epoch.next();
                reader.translating_article_id = None;
                reader.translation_incomplete = true;
            });
            self.clear_status();
            self.set_flash(self.t("Translation cancelled"), cx);
            cx.notify();
            return;
        }
        let Some(article) = self.read_article_reader(cx, |reader| reader.article.clone()) else {
            return;
        };
        if article.summary.id != id {
            return;
        }
        if self.read_article_reader(cx, |reader| reader.showing_translation) {
            self.update_article_reader(cx, |reader| reader.showing_translation = false);
            self.request_prepared_body(cx);
            return;
        }
        if self.read_article_reader(cx, |reader| {
            reader.has_translation_for_ui(
                &article,
                &self.preferences,
                &self.translator_config.revision_cache_id(),
            )
        }) {
            self.update_article_reader(cx, |reader| reader.showing_translation = true);
            self.request_prepared_body(cx);
            return;
        }
        if !self.translator_config.is_configured() {
            self.set_flash(
                self.t("Configure a translation provider in Settings before translating."),
                cx,
            );
            self.settings.open = true;
            self.settings.select_page(SettingsPage::Reading);
            cx.notify();
            return;
        }
        let target_lang = self.translation_target_code().to_owned();
        let content_revision = article.content_revision;
        let translator_id = self.translator_config.cache_id();
        let translation_layout = self.preferences.translation_layout;
        let hide_images = self.preferences.hide_images;
        let paragraph_indent = self.preferences.paragraph_indent;
        let revision = self.update_article_reader(cx, |reader| {
            reader.request_epoch.next();
            let revision = reader.translation_epoch.next();
            reader.translating_article_id = Some(id);
            reader.translation_spinner_frame = 0;
            reader.translation_incomplete = true;
            revision
        });
        let translation_started = std::time::Instant::now();
        let (progress, mut progress_receiver) = tokio::sync::mpsc::unbounded_channel();
        let (reply, response) = oneshot::channel();
        self.services.send(Command::Translate {
            id,
            target_lang,
            content_revision,
            translator_id,
            translation_layout,
            hide_images,
            paragraph_indent,
            progress,
            reply,
        });
        self.set_busy(self.t("Translating…"));
        cx.spawn(async move |this, cx| {
            let response = response;
            tokio::pin!(response);
            let result = loop {
                let animation_tick = cx.background_executor()
                    .timer(std::time::Duration::from_millis(125));
                tokio::select! {
                    biased;
                    result = &mut response => {
                        break result.unwrap_or_else(|_| Err("Background service stopped".to_string()));
                    }
                    Some(mut prepared) = progress_receiver.recv() => {
                        // Only render the newest snapshot when the UI falls behind.
                        while let Ok(newer) = progress_receiver.try_recv() {
                            prepared = newer;
                        }
                        let _ = this.update(cx, |this, cx| {
                            if !this.read_article_reader(cx, |reader| {
                                reader.translation_epoch.is_current(revision)
                                    && reader.article.as_ref().is_some_and(|article| article.summary.id == id)
                            })
                                || this.preferences.translation_layout != translation_layout
                                || this.preferences.hide_images != hide_images
                                || this.preferences.paragraph_indent != paragraph_indent
                            {
                                return;
                            }
                            this.update_article_reader(cx, |reader| {
                                reader.showing_translation = true;
                                reader.translation_incomplete = true;
                                reader.body_html = prepared.body_html.into();
                                reader.body_markdown = prepared.body_markdown.into();
                                reader.image_urls = prepared.image_urls.to_vec();
                                reader.article = Some(prepared.article);
                            });
                        });
                    }
                    _ = animation_tick => {
                        let _ = this.update(cx, |this, cx| {
                            if this.read_article_reader(cx, |reader| {
                                reader.translation_epoch.is_current(revision)
                                    && reader.translating_article_id == Some(id)
                            }) {
                                this.update_article_reader(cx, |reader| {
                                    reader.translation_spinner_frame =
                                        reader.translation_spinner_frame.wrapping_add(1);
                                });
                            }
                        });
                    }
                }
            };
            let _ = this.update(cx, |this, cx| {
                if !this.read_article_reader(cx, |reader| reader.translation_epoch.is_current(revision)) {
                    return;
                }
                this.update_article_reader(cx, |reader| reader.translating_article_id = None);
                this.clear_status();
                match result {
                    Ok(prepared) => {
                        let is_current = this.read_article_reader(cx, |reader| {
                            reader.article.as_ref().is_some_and(|current| current.summary.id == id)
                        });
                        this.update_article_reader(cx, |reader| {
                            reader.translation_incomplete = false;
                            reader.request_epoch.next();
                        });
                        if is_current {
                            this.update_article_reader(cx, |reader| {
                                reader.showing_translation = true;
                                reader.body_html = prepared.body_html.clone().into();
                                reader.body_markdown = prepared.body_markdown.clone().into();
                                reader.image_urls = prepared.image_urls.to_vec();
                            });
                            this.update_article_list(cx, |list, _| {
                                if let Some(row) = Arc::make_mut(&mut list.articles)
                                    .iter_mut()
                                    .find(|row| {
                                        row.id == id && row.title == prepared.article.summary.title
                                    })
                                {
                                    row.auto_translated_title =
                                        prepared.article.summary.auto_translated_title.clone();
                                    row.auto_translated_title_lang =
                                        prepared.article.summary.auto_translated_title_lang.clone();
                                    row.auto_translated_title_source_hash = prepared
                                        .article
                                        .summary
                                        .auto_translated_title_source_hash
                                        .clone();
                                }
                            });
                            this.update_article_reader(cx, |reader| {
                                reader.article = Some(prepared.article);
                            });
                            let elapsed = format!(
                                "{:.2} s",
                                translation_started.elapsed().as_secs_f64()
                            );
                            this.set_flash(
                                i18n::format(
                                    this.preferences.language,
                                    "Translation ready · {}",
                                    elapsed,
                                ),
                                cx,
                            );
                            if this.preferences.translation_layout != translation_layout
                                || this.preferences.hide_images != hide_images
                                || this.preferences.paragraph_indent != paragraph_indent
                            {
                                this.request_prepared_body(cx);
                            }
                        }
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn toggle_hide_images(&mut self, cx: &mut Context<Self>) {
        self.preferences.hide_images = !self.preferences.hide_images;
        self.save_preferences();
        if self.read_article_reader(cx, |reader| reader.article.is_some()) {
            self.request_prepared_body(cx);
        }
        cx.notify();
    }

    pub(in crate::ui) fn toggle_translation_layout(&mut self, cx: &mut Context<Self>) {
        self.preferences.translation_layout = match self.preferences.translation_layout {
            TranslationLayout::Immersive => TranslationLayout::Replaced,
            TranslationLayout::Replaced => TranslationLayout::Immersive,
        };
        self.save_preferences();
        if self.read_article_reader(cx, |reader| reader.article.is_some()) {
            self.request_prepared_body(cx);
        }
        cx.notify();
    }

    pub(in crate::ui) fn translation_target_code(&self) -> &'static str {
        self.preferences.translation_language.translator_code()
    }
}

impl ReaderWindow {
    pub(in crate::ui) fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        let list = self.list.read(cx);
        self.services.send(Command::MarkAllRead {
            scope: list.scope.clone(),
            search: list.search.clone(),
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
}
