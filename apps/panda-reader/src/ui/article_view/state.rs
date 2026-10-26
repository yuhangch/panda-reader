use crate::app::preferences::Preferences;
use crate::request_epoch::RequestEpoch;
use crate::ui::context_menus::PaneMenuEvent;
use gpui_kit::*;
use panda_core::{Article, MarkField, TranslationLayout};
use panda_translate::TranslatorConfig;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(in crate::ui) enum ArticleViewEvent {
    Mark {
        article_id: i64,
        field: MarkField,
        value: bool,
    },
    ToggleHideImages,
    ExtractFullText(i64),
    ToggleTranslation(i64),
    ToggleTranslationLayout,
    ShareArticle,
    SelectFeed(i64),
    SaveReadingProgress,
    OpenImage(usize),
    CopyImage(Arc<gpui::Image>),
}

impl EventEmitter<ArticleViewEvent> for ArticleView {}
impl EventEmitter<PaneMenuEvent> for ArticleView {}

pub(in crate::ui) struct ArticleView {
    pub(in crate::ui) preferences: Preferences,
    pub(in crate::ui) translator_config: TranslatorConfig,
    pub(in crate::ui) article: Option<Article>,
    pub(in crate::ui) body_html: SharedString,
    pub(in crate::ui) body_markdown: SharedString,
    pub(in crate::ui) image_urls: Vec<String>,
    pub(in crate::ui) image_viewer_url: Option<String>,
    pub(in crate::ui) scroll: ScrollHandle,
    pub(in crate::ui) restore_progress: Option<f32>,
    pub(in crate::ui) progress_epoch: u64,
    pub(in crate::ui) showing_translation: bool,
    pub(in crate::ui) translating_article_id: Option<i64>,
    pub(in crate::ui) translation_incomplete: bool,
    pub(in crate::ui) request_epoch: RequestEpoch,
    pub(in crate::ui) translation_epoch: RequestEpoch,
    pub(in crate::ui) translation_spinner_frame: u32,
    pub(in crate::ui) is_extracting: bool,
    pub(in crate::ui) requested_article_id: Option<i64>,
    pub(in crate::ui) show_loading_skeleton: bool,
    pub(in crate::ui) pending_context_image: Option<String>,
    pub(in crate::ui) context_menu_image: Option<String>,
    pub(in crate::ui) article_image_cache: HashMap<String, Arc<gpui::Image>>,
    pub(in crate::ui) article_image_cache_order: VecDeque<String>,
    pub(in crate::ui) article_images_loading: HashSet<String>,
}

impl ArticleView {
    pub(in crate::ui) fn new(
        preferences: Preferences,
        translator_config: TranslatorConfig,
    ) -> Self {
        Self {
            preferences,
            translator_config,
            article: None,
            body_html: SharedString::default(),
            body_markdown: SharedString::default(),
            image_urls: Vec::new(),
            image_viewer_url: None,
            scroll: ScrollHandle::new(),
            restore_progress: None,
            progress_epoch: 0,
            showing_translation: false,
            translating_article_id: None,
            translation_incomplete: false,
            request_epoch: RequestEpoch::default(),
            translation_epoch: RequestEpoch::default(),
            translation_spinner_frame: 0,
            is_extracting: false,
            requested_article_id: None,
            show_loading_skeleton: false,
            pending_context_image: None,
            context_menu_image: None,
            article_image_cache: HashMap::new(),
            article_image_cache_order: VecDeque::new(),
            article_images_loading: HashSet::new(),
        }
    }

    pub(in crate::ui) fn presentation_matches(
        &self,
        preferences: &Preferences,
        translator_config: &TranslatorConfig,
    ) -> bool {
        let current = &self.preferences;
        current.language == preferences.language
            && current.translation_language == preferences.translation_language
            && current.translation_layout == preferences.translation_layout
            && current.hide_images == preferences.hide_images
            && current.paragraph_indent == preferences.paragraph_indent
            && current.reader_font_family == preferences.reader_font_family
            && current.reader_font_size == preferences.reader_font_size
            && current.reader_line_height == preferences.reader_line_height
            && current.reader_paragraph_spacing == preferences.reader_paragraph_spacing
            && current.reader_content_width == preferences.reader_content_width
            && self.translator_config.revision_cache_id() == translator_config.revision_cache_id()
    }

    pub(in crate::ui) fn has_translation_for_ui(
        &self,
        article: &Article,
        preferences: &crate::app::preferences::Preferences,
        provider: &str,
    ) -> bool {
        if self.translation_incomplete {
            return false;
        }
        let target = preferences.translation_language.translator_code();
        let Some(canonical) = article.canonical.as_ref() else {
            return false;
        };
        let hash = panda_translate::translation_revision_hash(
            canonical.revision.0.as_ref(),
            provider,
            target,
        );
        article
            .translated_lang
            .as_deref()
            .is_some_and(|lang| lang == target)
            && article
                .translated_html
                .as_deref()
                .is_some_and(|html| !html.trim().is_empty())
            && article
                .translation_source_hash
                .as_deref()
                .is_some_and(|stored| stored == hash)
    }

    pub(in crate::ui) fn display_title(
        &self,
        article: &Article,
        preferences: &crate::app::preferences::Preferences,
    ) -> String {
        let source_hash = panda_translate::title_source_hash(&article.summary.title);
        if article.summary.auto_translated_title_lang.as_deref()
            == Some(preferences.translation_language.translator_code())
            && article.summary.auto_translated_title_source_hash.as_deref()
                == Some(source_hash.as_str())
            && let Some(title) = article
                .summary
                .auto_translated_title
                .as_deref()
                .filter(|s| !s.trim().is_empty())
        {
            return title.to_owned();
        }
        if !self.showing_translation {
            return article.summary.title.clone();
        }
        let translated = article
            .translated_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty());
        match (preferences.translation_layout, translated) {
            (TranslationLayout::Immersive, Some(translated))
                if translated != article.summary.title =>
            {
                // Heading shows original; translated title is rendered separately.
                article.summary.title.clone()
            }
            (_, Some(translated)) => translated.to_owned(),
            (_, None) => article.summary.title.clone(),
        }
    }

    pub(in crate::ui) fn display_translated_title(
        &self,
        article: &Article,
        preferences: &crate::app::preferences::Preferences,
    ) -> Option<String> {
        let hash = panda_translate::title_source_hash(&article.summary.title);
        if article.summary.auto_translated_title_lang.as_deref()
            == Some(preferences.translation_language.translator_code())
            && article.summary.auto_translated_title_source_hash.as_deref() == Some(hash.as_str())
            && article
                .summary
                .auto_translated_title
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
        {
            return None;
        }
        if !self.showing_translation
            || preferences.translation_layout != TranslationLayout::Immersive
        {
            return None;
        }
        article
            .translated_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty() && *title != article.summary.title)
            .map(str::to_owned)
    }
}
