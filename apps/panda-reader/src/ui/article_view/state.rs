use crate::request_epoch::RequestEpoch;
use gpui_kit::*;
use panda_core::{Article, TranslationLayout};

#[derive(Default)]
pub(in crate::ui) struct ArticleView {
    pub(in crate::ui) article: Option<Article>,
    pub(in crate::ui) body_html: SharedString,
    pub(in crate::ui) body_markdown: SharedString,
    pub(in crate::ui) image_urls: Vec<String>,
    pub(in crate::ui) image_viewer_url: Option<String>,
    pub(in crate::ui) scroll: ScrollHandle,
    pub(in crate::ui) restore_progress: Option<f32>,
    pub(in crate::ui) progress_epoch: u64,
    pub(in crate::ui) showing_translation: bool,
    pub(in crate::ui) request_epoch: RequestEpoch,
    pub(in crate::ui) is_extracting: bool,
}

impl ArticleView {
    pub(in crate::ui) fn has_translation_for_ui(
        &self,
        article: &Article,
        preferences: &crate::app::preferences::Preferences,
        provider: &str,
    ) -> bool {
        let target = preferences.translation_language.translator_code();
        let source = article
            .extracted_html
            .as_deref()
            .filter(|html| !html.trim().is_empty())
            .unwrap_or(article.content_html.as_str());
        let hash =
            panda_translate::translation_cache_hash(source, article.summary.title.trim(), provider);
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
        if article.summary.feed_auto_translate_titles
            && article.summary.auto_translated_title_lang.as_deref()
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
        if article.summary.feed_auto_translate_titles
            && article.summary.auto_translated_title_lang.as_deref()
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
