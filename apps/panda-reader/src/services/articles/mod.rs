pub(super) mod cache;
mod content;

use super::command::{TitleTranslationInput, TitleTranslationOutcome, TitleTranslationStatus};
use super::worker::{WorkerState, job};
use panda_core::{
    ArticleCursor, MarkField, PreparedArticle, ReaderSnapshot, Scope, TranslationLayout,
};
use panda_plugins::{PluginRegistry, PluginStage};
use panda_providers::ProviderClient;
use panda_store::Store;
use panda_translate::TranslatorConfig;
use tokio::sync::oneshot;

use cache::{BodyCache, BodyPrepKey};

use std::sync::Mutex;

pub fn prepare_article(
    store: &Store,
    cache: &Mutex<BodyCache>,
    id: i64,
    show_translation: bool,
    translation_layout: TranslationLayout,
    hide_images: bool,
    paragraph_indent: bool,
    plugins: &PluginRegistry,
) -> Result<PreparedArticle, String> {
    let mut article = store.article(id).map_err(|e| e.to_string())?;
    let reading_progress = store.reading_progress(id).map_err(|e| e.to_string())?;
    article.effective_html = Some(effective_article_body(store, &article, plugins)?);
    let key = BodyPrepKey::from_article(
        &article,
        show_translation,
        translation_layout,
        hide_images,
        paragraph_indent,
    );
    let body = cache.lock().map_err(|e| e.to_string())?.get_or_insert(
        key,
        &article,
        show_translation,
        translation_layout,
        hide_images,
        paragraph_indent,
    );
    Ok(PreparedArticle {
        article,
        body_html: body.html,
        body_markdown: body.markdown,
        reading_progress,
        image_urls: body.image_urls,
    })
}

fn effective_article_body(
    store: &Store,
    article: &panda_core::Article,
    plugins: &PluginRegistry,
) -> Result<String, String> {
    let source = article
        .extracted_html
        .as_deref()
        .filter(|html| !html.trim().is_empty())
        .or(article.source_html.as_deref())
        .unwrap_or(&article.content_html);
    let source_url = article
        .url
        .as_deref()
        .or(article.summary.url.as_deref())
        .unwrap_or("");
    let pipeline = plugins.cache_key();
    if let Some(cached) = store
        .processed_content(
            article.summary.id,
            source,
            source_url,
            &article.summary.title,
            &pipeline,
        )
        .map_err(|error| format!("could not read processed article cache: {error:#}"))?
    {
        return Ok(cached);
    }
    let processed = plugins.process(
        source,
        source_url,
        &article.summary.title,
        PluginStage::Cleanup,
    );
    for diagnostic in &processed.diagnostics {
        eprintln!(
            "article plugin {} failed: {}",
            diagnostic.plugin_id, diagnostic.message
        );
    }
    let body = if article.extracted_html.is_some() {
        panda_content::remove_leading_duplicate_title(&processed.html, &article.summary.title)
    } else {
        processed.html
    };
    let effective = panda_content::sanitize_html(&body, Some(source_url));
    store
        .save_processed_content(
            article.summary.id,
            source,
            source_url,
            &article.summary.title,
            &pipeline,
            &effective,
        )
        .map_err(|error| format!("could not save processed article cache: {error:#}"))?;
    Ok(effective)
}

pub(super) fn snapshot(
    scope: Scope,
    search: String,
    limit: i64,
    after: Option<ArticleCursor>,
    include_feeds: bool,
    reply: oneshot::Sender<Result<ReaderSnapshot, String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    job(reply, move |_| {
        let store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        store
            .snapshot(scope, &search, limit, after.as_ref(), include_feeds)
            .map_err(|e| e.to_string())
    });
}

pub(super) fn article(
    id: i64,
    show_translation: bool,
    translation_layout: TranslationLayout,
    hide_images: bool,
    paragraph_indent: bool,
    reply: oneshot::Sender<Result<PreparedArticle, String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let cache = state.body_cache.clone();
    let plugins = state.plugin_registry.clone();
    job(reply, move |_| {
        let store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let plugins = plugins
            .read()
            .map_err(|_| "plugin registry lock poisoned".to_string())?;
        prepare_article(
            &store,
            &cache,
            id,
            show_translation,
            translation_layout,
            hide_images,
            paragraph_indent,
            &plugins,
        )
    });
}

pub(super) fn save_reading_progress(
    id: i64,
    progress: f32,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let result = Store::open_workspace(&state.path, &state.workspace())
        .and_then(|store| store.save_reading_progress(id, progress))
        .map_err(|error| error.to_string());
    let _ = reply.send(result);
}

pub(super) fn mark_all_read(
    scope: Scope,
    reply: oneshot::Sender<Result<usize, String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let provider = state.provider_kind();
    let config = provider.and_then(|kind| state.provider_settings(kind));
    job(reply, move |runtime| {
        let mut store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let count = store.mark_all_read(scope).map_err(|e| e.to_string())?;
        if let (Some(kind), Some(config)) = (provider, config) {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime
                .block_on(store.flush_provider_marks(&remote))
                .map_err(|e| e.to_string())?;
        }
        Ok(count)
    });
}

pub(super) fn mark(
    id: i64,
    field: MarkField,
    value: bool,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let provider = state.provider_kind();
    let config = provider.and_then(|kind| state.provider_settings(kind));
    job(reply, move |runtime| {
        let mut store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let remote_id = store.remote_entry_id(id).map_err(|e| e.to_string())?;
        store.mark(id, field, value).map_err(|e| e.to_string())?;
        if remote_id.is_some()
            && !matches!(field, MarkField::Later)
            && let (Some(kind), Some(config)) = (provider, config)
        {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime
                .block_on(store.flush_provider_marks(&remote))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    });
}

pub(super) fn extract(
    id: i64,
    force: bool,
    extractor: panda_core::ContentExtractor,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let cache = state.body_cache.clone();
    let plugins = state.plugin_registry.clone();
    job(reply, move |runtime| {
        let mut store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let plugins = plugins
            .read()
            .map_err(|_| "plugin registry lock poisoned".to_string())?;
        let pipeline_hash = format!("{}:{extractor:?}", plugins.cache_key());
        let result = runtime
            .block_on(store.extract_using(
                id,
                force,
                extractor,
                &pipeline_hash,
                |raw, url, title| {
                    let prepared = plugins.process(raw, url, title, PluginStage::Prepare);
                    for diagnostic in &prepared.diagnostics {
                        eprintln!(
                            "article plugin {} failed: {}",
                            diagnostic.plugin_id, diagnostic.message
                        );
                    }
                    Ok((!prepared.matched_plugins.is_empty())
                        .then_some((prepared.html, prepared.body_selected)))
                },
            ))
            .map_err(|e| e.to_string());
        if result.is_ok() {
            if let Ok(mut cache) = cache.lock() {
                cache.invalidate_article(id);
            }
        }
        result
    });
}

pub(super) fn translate(
    id: i64,
    target_lang: String,
    translation_layout: TranslationLayout,
    hide_images: bool,
    paragraph_indent: bool,
    reply: oneshot::Sender<Result<PreparedArticle, String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let translator_path = state.translator_path.clone();
    let cache = state.body_cache.clone();
    let plugins = state.plugin_registry.clone();
    job(reply, move |runtime| {
        let config = TranslatorConfig::load(&translator_path).map_err(|e| e.to_string())?;
        let translator = panda_translate::build(&config).map_err(|e| e.to_string())?;
        let mut store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let mut article = store.article(id).map_err(|e| e.to_string())?;
        let registry = plugins
            .read()
            .map_err(|_| "plugin registry lock poisoned".to_string())?;
        let effective = effective_article_body(&store, &article, &registry)?;
        drop(registry);
        article = runtime
            .block_on(store.translate_with_source(id, &target_lang, &effective, &translator))
            .map_err(|e| e.to_string())?;
        article.effective_html = Some(effective);
        if let Ok(mut cache) = cache.lock() {
            cache.invalidate_article(id);
        }
        let body = {
            let mut guard = cache.lock().map_err(|e| e.to_string())?;
            let key = BodyPrepKey::from_article(
                &article,
                true,
                translation_layout,
                hide_images,
                paragraph_indent,
            );
            guard.get_or_insert(
                key,
                &article,
                true,
                translation_layout,
                hide_images,
                paragraph_indent,
            )
        };
        Ok(PreparedArticle {
            article,
            body_html: body.html,
            body_markdown: body.markdown,
            reading_progress: store.reading_progress(id).map_err(|e| e.to_string())?,
            image_urls: body.image_urls,
        })
    });
}

pub(super) fn translation_usage(
    reply: oneshot::Sender<Result<Vec<panda_core::TranslationUsage>, String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    job(reply, move |_| {
        Store::open_workspace(&path, &workspace)
            .map_err(|e| e.to_string())?
            .translation_usage()
            .map_err(|e| e.to_string())
    })
}

pub(super) fn translate_titles(
    items: Vec<TitleTranslationInput>,
    target_lang: String,
    reply: oneshot::Sender<Result<TitleTranslationOutcome, String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let translator_path = state.translator_path.clone();
    let lock = state.title_translation_lock.clone();
    let attempted = state.title_translation_attempted.clone();
    job(reply, move |runtime| {
        let _serial = lock.lock().map_err(|e| e.to_string())?;
        let received = items.len();
        let config = match TranslatorConfig::load(&translator_path) {
            Ok(config) => config,
            Err(error) => {
                log_title_translation(&path, &format!("settings read failed: {error:#}"));
                return Err(error.to_string());
            }
        };
        let provider = config.provider.id().to_owned();
        let store = match Store::open_workspace(&path, &workspace) {
            Ok(store) => store,
            Err(error) => {
                log_title_translation(&path, &format!("store open failed: {error:#}"));
                return Err(error.to_string());
            }
        };
        log_title_translation(
            &path,
            &format!(
                "batch start: received={received}, target={target_lang}, provider={provider}, workspace={workspace}"
            ),
        );
        let mut eligible = Vec::new();
        let mut hashes = Vec::new();
        let mut attempt_keys = Vec::new();
        let mut translated = Vec::new();
        let mut disabled_feed = 0usize;
        let mut cached = 0usize;
        let mut deferred = 0usize;
        let mut stale = 0usize;
        let mut duplicates = 0usize;
        let mut seen = attempted.lock().map_err(|e| e.to_string())?;
        let mut this_batch = std::collections::HashSet::new();
        for item in items {
            let hash = panda_translate::title_source_hash(&item.title);
            let key = format!("{workspace}|{}|{hash}|{target_lang}|{provider}", item.id);
            if !this_batch.insert(key.clone()) {
                duplicates += 1;
                continue;
            }
            let article = store.article(item.id).map_err(|e| e.to_string())?;
            if article.summary.title != item.title {
                stale += 1;
                continue;
            }
            if !article.summary.feed_auto_translate_titles {
                disabled_feed += 1;
                continue;
            }
            if article.summary.auto_translated_title_lang.as_deref() == Some(target_lang.as_str())
                && article.summary.auto_translated_title_source_hash.as_deref()
                    == Some(hash.as_str())
                && article
                    .summary
                    .auto_translated_title
                    .as_deref()
                    .is_some_and(|s| !s.is_empty())
            {
                cached += 1;
                translated.push((
                    item.id,
                    item.title,
                    article.summary.auto_translated_title.unwrap_or_default(),
                    target_lang.clone(),
                ));
                continue;
            }
            if seen.get(&key).is_some_and(|last_attempt| {
                last_attempt.elapsed() < std::time::Duration::from_secs(60)
            }) {
                deferred += 1;
                continue;
            }
            eligible.push(item);
            hashes.push(hash);
            attempt_keys.push(key);
        }
        if eligible.is_empty() {
            let usage = store.translation_usage().map_err(|e| e.to_string())?;
            log_title_translation(
                &path,
                &format!(
                    "batch skipped: feed_disabled={disabled_feed}, cached={cached}, retry_backoff={deferred}, stale={stale}, duplicates={duplicates}"
                ),
            );
            let status = if translated.is_empty() && deferred > 0 {
                Some(TitleTranslationStatus::Info(format!(
                    "Title translation retry deferred for {deferred} title(s) after a recent attempt"
                )))
            } else {
                None
            };
            return Ok(TitleTranslationOutcome {
                translated,
                usage,
                status,
            });
        }
        let translator = match panda_translate::build(&config) {
            Ok(translator) => translator,
            Err(error) => {
                log_title_translation(&path, &format!("translator setup failed: {error:#}"));
                return Err(error.to_string());
            }
        };
        seen.extend(
            attempt_keys
                .into_iter()
                .map(|key| (key, std::time::Instant::now())),
        );
        let titles: Vec<String> = eligible.iter().map(|item| item.title.clone()).collect();
        let result = match runtime.block_on(translator.translate_titles(&titles, &target_lang)) {
            Ok(result) => result,
            Err(failure) => {
                log_title_translation(
                    &path,
                    &format!(
                        "request failed: provider={provider}, titles={}, requests={}, characters={}, error={:#}",
                        eligible.len(),
                        failure.requests,
                        failure.characters,
                        failure.error
                    ),
                );
                if failure.requests > 0 {
                    store
                        .record_translation_usage(
                            &chrono::Local::now().format("%Y-%m-%d").to_string(),
                            &provider,
                            failure.requests as u64,
                            failure.characters as u64,
                        )
                        .map_err(|e| e.to_string())?;
                }
                return Err(failure.error.to_string());
            }
        };
        if result.translations.len() != eligible.len() {
            let error = format!(
                "Translation result count mismatch: sent {}, received {}",
                eligible.len(),
                result.translations.len()
            );
            log_title_translation(&path, &error);
            return Err(error);
        }
        let chars = titles.iter().map(|s| s.chars().count() as u64).sum();
        if result.requests > 0 {
            store
                .record_translation_usage(
                    &chrono::Local::now().format("%Y-%m-%d").to_string(),
                    &provider,
                    result.requests as u64,
                    chars,
                )
                .map_err(|e| e.to_string())?;
        }
        let mut empty_results = 0usize;
        let mut newly_translated = 0usize;
        for ((item, hash), text) in eligible.into_iter().zip(hashes).zip(result.translations) {
            if !text.trim().is_empty() {
                store
                    .save_auto_translated_title(item.id, &item.title, &text, &target_lang, &hash)
                    .map_err(|e| e.to_string())?;
                translated.push((item.id, item.title, text, target_lang.clone()));
                newly_translated += 1;
            } else {
                empty_results += 1;
            }
        }
        let usage = store.translation_usage().map_err(|e| e.to_string())?;
        log_title_translation(
            &path,
            &format!(
                "batch complete: requested={}, newly_translated={newly_translated}, cached={cached}, empty_results={empty_results}, feed_disabled={disabled_feed}, retry_backoff={deferred}, stale={stale}, duplicates={duplicates}, requests={}, characters={chars}",
                newly_translated + empty_results,
                result.requests
            ),
        );
        let status = if empty_results > 0 {
            Some(TitleTranslationStatus::Error(format!(
                "Translation API returned {empty_results} empty result(s) out of {} title(s)",
                newly_translated + empty_results
            )))
        } else if newly_translated > 0 {
            Some(TitleTranslationStatus::Info(format!(
                "Translated {newly_translated} title(s)"
            )))
        } else {
            None
        };
        Ok(TitleTranslationOutcome {
            translated,
            usage,
            status,
        })
    })
}

fn log_title_translation(database_path: &std::path::Path, message: &str) {
    use std::io::Write as _;
    let Some(directory) = database_path.parent() else {
        return;
    };
    let path = directory.join("title-translation.log");
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = writeln!(file, "[{timestamp}] {message}");
}

#[cfg(test)]
fn test_article() -> panda_core::Article {
    panda_core::Article {
        summary: panda_core::ArticleSummary {
            id: 1,
            feed_title: "Example feed".into(),
            feed_language: None,
            feed_auto_translate_titles: false,
            title: "Example article".into(),
            url: None,
            author: None,
            snippet: "<example> & preview".into(),
            published_at: None,
            is_read: false,
            is_starred: false,
            read_later: false,
            auto_translated_title: None,
            auto_translated_title_lang: None,
            auto_translated_title_source_hash: None,
        },
        url: None,
        content_html: "<p>Original</p><img src=\"https://example.com/image.png\">".into(),
        source_html: None,
        source_page_html: None,
        extracted_html: None,
        effective_html: None,
        translated_html: Some("<p>译文</p>".into()),
        translated_title: None,
        translated_lang: None,
        translation_source_hash: None,
    }
}
