use crate::reader_body::{BodyCache, BodyPrepKey};
use panda_core::{PreparedArticle, TranslationLayout};
use panda_store::Store;
use std::sync::Mutex;

pub fn prepare_article(
    store: &Store,
    cache: &Mutex<BodyCache>,
    id: i64,
    show_translation: bool,
    translation_layout: TranslationLayout,
    hide_images: bool,
) -> Result<PreparedArticle, String> {
    let article = store.article(id).map_err(|e| e.to_string())?;
    let key =
        BodyPrepKey::from_article(&article, show_translation, translation_layout, hide_images);
    let body_html = cache.lock().map_err(|e| e.to_string())?.get_or_insert(
        key,
        &article,
        show_translation,
        translation_layout,
        hide_images,
    );
    Ok(PreparedArticle { article, body_html })
}
