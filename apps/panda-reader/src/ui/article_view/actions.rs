use crate::services::Command;
use crate::ui::i18n;
use crate::ui::settings::SettingsPage;
use crate::ui::window::ReaderWindow;
use gpui_kit::*;
use panda_core::{MarkField, Scope, TranslationLayout};
use std::{
    borrow::Cow,
    io::Cursor,
    sync::{Arc, OnceLock},
};
use tokio::sync::oneshot;

impl ReaderWindow {
    pub(in crate::ui) fn copy_article_image(&mut self, url: String, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let result = match fetch_article_image(&url).await {
                Ok(image) => {
                    cx.background_executor()
                        .spawn(async move {
                            let mut clipboard =
                                arboard::Clipboard::new().map_err(|error| error.to_string())?;
                            clipboard
                                .set_image(image)
                                .map_err(|error| error.to_string())
                        })
                        .await
                }
                Err(error) => Err(error),
            };
            let _ = weak.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.set_flash(this.t("Image copied"), cx);
                }
                Err(error) => {
                    eprintln!("could not copy article image: {error}");
                    this.set_flash(this.t("Could not copy image"), cx);
                }
            });
        })
        .detach();
    }

    pub(in crate::ui) fn share_article(&mut self, cx: &mut Context<Self>) {
        let Some(article) = self.reader.article.as_ref() else {
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
        self.save_current_reading_progress();
        let revision = self.reader.request_epoch.next();
        self.reader.progress_epoch = self.reader.progress_epoch.wrapping_add(1);
        self.reader.restore_progress = None;
        self.reader.image_viewer_url = None;
        self.reader.image_urls.clear();
        self.reader.scroll.set_offset(point(px(0.), px(0.)));
        self.reader.is_extracting = false;
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
        if let Some(summary) = Arc::make_mut(&mut self.list.articles)
            .iter_mut()
            .find(|article| article.id == id)
        {
            if !summary.is_read {
                summary.is_read = true;
                if let Some(feed) = self
                    .sidebar
                    .feeds
                    .iter_mut()
                    .find(|feed| feed.title == summary.feed_title)
                {
                    feed.unread = feed.unread.saturating_sub(1);
                }
            }
        }
        self.reader.showing_translation = false;
        self.reader.body_html = SharedString::default();
        self.reader.body_markdown = SharedString::default();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if !this.reader.request_epoch.is_current(revision) {
                    return;
                }
                match result {
                    Ok(prepared) => {
                        let reading_progress = prepared.reading_progress;
                        let mut article = prepared.article;
                        article.summary.is_read = true;
                        this.reader.showing_translation = false;
                        this.reader.body_html = prepared.body_html.into();
                        this.reader.body_markdown = prepared.body_markdown.into();
                        this.reader.image_urls = prepared.image_urls.to_vec();
                        let should_auto_extract = this.preferences.auto_extract_full_text
                            && article.extracted_html.is_none()
                            && article.url.is_some();
                        this.reader.article = Some(article);
                        if this.preferences.remember_reading_position && reading_progress > 0. {
                            this.reader.restore_progress = Some(reading_progress);
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
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn schedule_reading_progress_save(&mut self, cx: &mut Context<Self>) {
        if !self.preferences.remember_reading_position || self.reader.article.is_none() {
            return;
        }
        self.reader.progress_epoch = self.reader.progress_epoch.wrapping_add(1);
        let epoch = self.reader.progress_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(700))
                .await;
            let _ = this.update(cx, |this, _| {
                if this.reader.progress_epoch == epoch {
                    this.save_current_reading_progress();
                }
            });
        })
        .detach();
    }

    fn save_current_reading_progress(&self) {
        if !self.preferences.remember_reading_position {
            return;
        }
        let Some(article) = &self.reader.article else {
            return;
        };
        let max = self.reader.scroll.max_offset().y;
        if max <= px(0.) {
            return;
        }
        let progress = (-self.reader.scroll.offset().y / max).clamp(0., 1.);
        let (reply, _response) = oneshot::channel();
        self.services
            .send(crate::services::Command::SaveReadingProgress {
                id: article.summary.id,
                progress,
                reply,
            });
    }

    fn restore_reading_progress(&mut self, id: i64, progress: f32, cx: &mut Context<Self>) {
        self.reader.progress_epoch = self.reader.progress_epoch.wrapping_add(1);
        let epoch = self.reader.progress_epoch;
        let scroll = self.reader.scroll.clone();
        cx.spawn(async move |this, cx| {
            for _ in 0..20 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let restored = this
                    .update(cx, |this, cx| {
                        if this.reader.progress_epoch != epoch
                            || this
                                .reader
                                .article
                                .as_ref()
                                .is_none_or(|article| article.summary.id != id)
                        {
                            return true;
                        }
                        let max = scroll.max_offset().y;
                        if max > px(0.) {
                            scroll.set_offset(point(px(0.), -max * progress));
                            this.reader.restore_progress = None;
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
        let Some(article) = &self.reader.article else {
            return;
        };
        let id = article.summary.id;
        let revision = self.reader.request_epoch.next();
        let (reply, response) = oneshot::channel();
        self.services.send(Command::Article {
            id,
            show_translation: self.reader.showing_translation,
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
                if !this.reader.request_epoch.is_current(revision) {
                    return;
                }
                match result {
                    Ok(prepared) => {
                        this.reader.article = Some(prepared.article);
                        this.reader.body_html = prepared.body_html.into();
                        this.reader.body_markdown = prepared.body_markdown.into();
                        this.reader.image_urls = prepared.image_urls.to_vec();
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
            .reader
            .article
            .as_ref()
            .filter(|article| article.summary.id == id)
            .map(|article| (article.summary.feed_id, article.summary.is_read))
            .or_else(|| {
                self.list
                    .articles
                    .iter()
                    .find(|article| article.id == id)
                    .map(|article| (article.feed_id, article.is_read))
            });
        let feed_id = current.map(|(feed_id, _)| feed_id);
        let old_read = current.map(|(_, is_read)| is_read).unwrap_or(!value);
        let remove_from_scope = matches!(
            (&self.list.scope, field, value),
            (Scope::Unread, MarkField::Read, true)
                | (Scope::Starred, MarkField::Starred, false)
                | (Scope::Later, MarkField::Later, false)
        );
        if let Some(article) = &mut self.reader.article {
            if article.summary.id == id {
                match field {
                    MarkField::Read => article.summary.is_read = value,
                    MarkField::Starred => article.summary.is_starred = value,
                    MarkField::Later => article.summary.read_later = value,
                }
            }
        }
        if let Some(summary) = Arc::make_mut(&mut self.list.articles)
            .iter_mut()
            .find(|article| article.id == id)
        {
            match field {
                MarkField::Read => summary.is_read = value,
                MarkField::Starred => summary.is_starred = value,
                MarkField::Later => summary.read_later = value,
            }
        }
        if field == MarkField::Read
            && old_read != value
            && let Some(feed_id) = feed_id
            && let Some(feed) = self
                .sidebar
                .feeds
                .iter_mut()
                .find(|feed| feed.id == feed_id)
        {
            feed.unread = (feed.unread + if value { -1 } else { 1 }).max(0);
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
                if let Err(error) = result {
                    if let Some(article) = &mut this.reader.article {
                        if article.summary.id == id {
                            match field {
                                MarkField::Read => article.summary.is_read = !value,
                                MarkField::Starred => article.summary.is_starred = !value,
                                MarkField::Later => article.summary.read_later = !value,
                            }
                        }
                    }
                    if let Some(summary) = Arc::make_mut(&mut this.list.articles)
                        .iter_mut()
                        .find(|article| article.id == id)
                    {
                        match field {
                            MarkField::Read => summary.is_read = !value,
                            MarkField::Starred => summary.is_starred = !value,
                            MarkField::Later => summary.read_later = !value,
                        }
                    }
                    if field == MarkField::Read
                        && old_read != value
                        && let Some(feed_id) = feed_id
                        && let Some(feed) = this
                            .sidebar
                            .feeds
                            .iter_mut()
                            .find(|feed| feed.id == feed_id)
                    {
                        feed.unread = (feed.unread + if old_read { 1 } else { -1 }).max(0);
                    }
                    this.set_error(error);
                } else if remove_from_scope {
                    Arc::make_mut(&mut this.list.articles).retain(|article| article.id != id);
                    this.load_snapshot(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn extract(&mut self, id: i64, force: bool, cx: &mut Context<Self>) {
        if self.reader.is_extracting {
            return;
        }
        if !force
            && self.reader.article.as_ref().is_some_and(|article| {
                article.summary.id == id
                    && article
                        .extracted_html
                        .as_deref()
                        .is_some_and(|html| !html.trim().is_empty())
            })
        {
            return;
        }
        let (reply, response) = oneshot::channel();
        let extractor = self.preferences.content_extractor;
        let show_translation = self.reader.showing_translation;
        let translation_layout = self.preferences.translation_layout;
        let hide_images = self.preferences.hide_images;
        let paragraph_indent = self.preferences.paragraph_indent;
        let revision = self.reader.request_epoch.next();
        self.services.send(Command::Extract {
            id,
            force,
            extractor,
            reply,
        });
        self.reader.is_extracting = true;
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
                if !this.reader.request_epoch.is_current(revision) {
                    return;
                }
                this.reader.is_extracting = false;
                match result {
                    Ok(()) => {
                        this.set_flash(this.t("Full text extracted"), cx);
                        cx.spawn(async move |this, cx| {
                            if let Ok(Ok(prepared)) = article_response.await {
                                let _ = this.update(cx, |this, cx| {
                                    if this.reader.request_epoch.is_current(revision)
                                        && this
                                            .reader
                                            .article
                                            .as_ref()
                                            .is_some_and(|current| current.summary.id == id)
                                    {
                                        this.reader.body_html = prepared.body_html.into();
                                        this.reader.body_markdown = prepared.body_markdown.into();
                                        this.reader.image_urls = prepared.image_urls.to_vec();
                                        this.reader.article = Some(prepared.article);
                                        cx.notify();
                                    }
                                });
                            }
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
        let Some(article) = self.reader.article.as_ref() else {
            return;
        };
        if article.summary.id != id {
            return;
        }
        if self.reader.showing_translation {
            self.reader.showing_translation = false;
            self.request_prepared_body(cx);
            return;
        }
        if self.reader.has_translation_for_ui(
            article,
            &self.preferences,
            self.translator_config.provider.id(),
        ) {
            self.reader.showing_translation = true;
            self.request_prepared_body(cx);
            return;
        }
        if !self.translator_config.is_configured() {
            self.set_flash(
                self.t("Configure a translation provider in Settings before translating."),
                cx,
            );
            self.settings.open = true;
            self.settings.page = SettingsPage::Reading;
            cx.notify();
            return;
        }
        let target_lang = self.translation_target_code().to_owned();
        let translation_layout = self.preferences.translation_layout;
        let hide_images = self.preferences.hide_images;
        let paragraph_indent = self.preferences.paragraph_indent;
        let revision = self.reader.request_epoch.next();
        let (reply, response) = oneshot::channel();
        self.services.send(Command::Translate {
            id,
            target_lang,
            translation_layout,
            hide_images,
            paragraph_indent,
            reply,
        });
        self.set_busy(self.t("Translating…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                if !this.reader.request_epoch.is_current(revision) {
                    return;
                }
                match result {
                    Ok(prepared) => {
                        if this
                            .reader
                            .article
                            .as_ref()
                            .is_some_and(|current| current.summary.id == id)
                        {
                            this.reader.showing_translation = true;
                            this.reader.body_html = prepared.body_html.into();
                            this.reader.body_markdown = prepared.body_markdown.into();
                            this.reader.image_urls = prepared.image_urls.to_vec();
                            if let Some(row) = Arc::make_mut(&mut this.list.articles)
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
                            this.reader.article = Some(prepared.article);
                            this.set_flash(this.t("Translation ready"), cx);
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
        if self.reader.article.is_some() {
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
        if self.reader.article.is_some() {
            self.request_prepared_body(cx);
        }
        cx.notify();
    }

    pub(in crate::ui) fn translation_target_code(&self) -> &'static str {
        self.preferences.translation_language.translator_code()
    }
}

const MAX_CLIPBOARD_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CLIPBOARD_IMAGE_PIXELS: u64 = 40_000_000;

async fn fetch_article_image(url: &str) -> Result<arboard::ImageData<'static>, String> {
    let parsed = url::Url::parse(url).map_err(|error| error.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("only HTTP and HTTPS images can be copied".into());
    }

    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("PandaReader/0.1")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("article image client configuration is valid")
    });
    let mut response = client
        .get(parsed)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_CLIPBOARD_IMAGE_BYTES as u64)
    {
        return Err("image exceeds the 32 MiB copy limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_CLIPBOARD_IMAGE_BYTES {
            return Err("image exceeds the 32 MiB copy limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }

    let png = match image::load_from_memory(&bytes) {
        Ok(decoded) => {
            let (width, height) = image::GenericImageView::dimensions(&decoded);
            if u64::from(width) * u64::from(height) > MAX_CLIPBOARD_IMAGE_PIXELS {
                return Err("image dimensions exceed the copy limit".into());
            }
            let mut png = Cursor::new(Vec::new());
            decoded
                .write_to(&mut png, image::ImageFormat::Png)
                .map_err(|error| error.to_string())?;
            png.into_inner()
        }
        Err(raster_error) => {
            let svg = std::str::from_utf8(&bytes).map_err(|_| raster_error.to_string())?;
            let tree =
                resvg::usvg::Tree::from_data(svg.as_bytes(), &resvg::usvg::Options::default())
                    .map_err(|_| raster_error.to_string())?;
            let size = tree.size().to_int_size();
            if u64::from(size.width()) * u64::from(size.height()) > MAX_CLIPBOARD_IMAGE_PIXELS {
                return Err("image dimensions exceed the copy limit".into());
            }
            let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
                .ok_or_else(|| "could not allocate image for copying".to_owned())?;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::identity(),
                &mut pixmap.as_mut(),
            );
            pixmap.encode_png().map_err(|error| error.to_string())?
        }
    };
    let decoded = image::load_from_memory(&png).map_err(|error| error.to_string())?;
    let rgba = decoded.to_rgba8();
    Ok(arboard::ImageData {
        width: rgba.width() as usize,
        height: rgba.height() as usize,
        bytes: Cow::Owned(rgba.into_raw()),
    })
}

impl ReaderWindow {
    pub(in crate::ui) fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.services.send(Command::MarkAllRead {
            scope: self.list.scope.clone(),
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
