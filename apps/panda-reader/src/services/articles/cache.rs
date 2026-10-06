//! Bounded cache for prepared article bodies.

use super::content::prepare_body;
use panda_core::{Article, TranslationLayout};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodyPrepKey {
    pub article_id: i64,
    pub has_extracted: bool,
    pub show_translation: bool,
    pub immersive: bool,
    pub hide_images: bool,
    /// Cheap fingerprint so extract/translate invalidates the cache.
    pub content_rev: [u8; 32],
}

impl BodyPrepKey {
    pub fn from_article(
        article: &Article,
        show_translation: bool,
        layout: TranslationLayout,
        hide_images: bool,
    ) -> Self {
        Self {
            article_id: article.summary.id,
            has_extracted: article.extracted_html.is_some(),
            show_translation,
            immersive: layout == TranslationLayout::Immersive,
            hide_images,
            content_rev: content_revision(article),
        }
    }
}

fn content_revision(article: &Article) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for value in [
        article.content_html.as_str(),
        article.extracted_html.as_deref().unwrap_or_default(),
        article.translated_html.as_deref().unwrap_or_default(),
        article
            .translation_source_hash
            .as_deref()
            .unwrap_or_default(),
    ] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.finalize().into()
}

#[derive(Default)]
pub struct BodyCache {
    entries: HashMap<BodyPrepKey, String>,
    order: Vec<BodyPrepKey>,
}

impl BodyCache {
    const CAPACITY: usize = 64;

    pub fn get_or_insert(
        &mut self,
        key: BodyPrepKey,
        article: &Article,
        show_translation: bool,
        layout: TranslationLayout,
        hide_images: bool,
    ) -> String {
        if let Some(body) = self.entries.get(&key) {
            return body.clone();
        }
        let body = prepare_body(article, show_translation, layout, hide_images);
        if self.order.len() >= Self::CAPACITY {
            if let Some(old) = self.order.first().cloned() {
                self.order.remove(0);
                self.entries.remove(&old);
            }
        }
        self.order.push(key);
        self.entries.insert(key, body.clone());
        body
    }

    pub fn invalidate_article(&mut self, article_id: i64) {
        self.order.retain(|key| key.article_id != article_id);
        self.entries.retain(|key, _| key.article_id != article_id);
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_article;
    use super::*;

    #[test]
    fn cache_distinguishes_display_options_and_invalidates_every_article_variant() {
        let mut article = test_article();
        let mut cache = BodyCache::default();
        for (translated, layout, hide_images) in [
            (false, TranslationLayout::Immersive, false),
            (false, TranslationLayout::Immersive, true),
            (true, TranslationLayout::Immersive, true),
            (true, TranslationLayout::Replaced, true),
        ] {
            let key = BodyPrepKey::from_article(&article, translated, layout, hide_images);
            let prepared = cache.get_or_insert(key, &article, translated, layout, hide_images);
            assert_eq!(
                prepared,
                prepare_body(&article, translated, layout, hide_images)
            );
        }
        assert_eq!(cache.entries.len(), 4);
        cache.invalidate_article(article.summary.id);
        assert!(cache.entries.is_empty());
        assert!(cache.order.is_empty());
        article.content_html = "<p>Updated body</p>".into();
        let key = BodyPrepKey::from_article(&article, false, TranslationLayout::Immersive, false);
        let prepared =
            cache.get_or_insert(key, &article, false, TranslationLayout::Immersive, false);
        assert!(prepared.contains("Updated body"));
    }

    #[test]
    fn cache_key_changes_when_equal_length_content_changes() {
        let mut article = test_article();
        article.content_html = "<p>First</p>".into();
        let original =
            BodyPrepKey::from_article(&article, false, TranslationLayout::Immersive, false);
        let mut cache = BodyCache::default();
        assert!(
            cache
                .get_or_insert(
                    original,
                    &article,
                    false,
                    TranslationLayout::Immersive,
                    false,
                )
                .contains("First")
        );

        article.content_html = "<p>Other</p>".into();
        let updated =
            BodyPrepKey::from_article(&article, false, TranslationLayout::Immersive, false);
        assert_ne!(original, updated);
        assert!(
            cache
                .get_or_insert(
                    updated,
                    &article,
                    false,
                    TranslationLayout::Immersive,
                    false,
                )
                .contains("Other")
        );
    }
}
