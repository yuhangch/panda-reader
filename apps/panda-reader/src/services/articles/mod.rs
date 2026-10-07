pub(super) mod cache;
mod content;

use super::command::{TitleTranslationInput, TitleTranslationOutcome, TitleTranslationStatus};
use super::database::DbWriter;
use super::worker::{WorkerState, job, lock_mutex, read_lock};
use panda_core::{
    ArticleCursor, CanonicalArticle, CanonicalHtml, MarkField, PreparedArticle, RawHtml,
    ReaderSnapshot, RenderOptions, Scope, TranslationLayout,
};
use panda_plugins::{PluginRegistry, PluginStage};
use panda_providers::ProviderClient;
use panda_store::Store;
use panda_translate::TranslatorConfig;
use tokio::sync::oneshot;

use cache::{RenderCache, RenderCacheKey};

use std::sync::Mutex;

pub fn prepare_article(
    store: &Store,
    cache: &Mutex<RenderCache>,
    id: i64,
    show_translation: bool,
    translation_layout: TranslationLayout,
    hide_images: bool,
    paragraph_indent: bool,
    plugins: &PluginRegistry,
    database: &DbWriter,
    workspace: &str,
) -> Result<PreparedArticle, String> {
    let mut article = store.article(id).map_err(|e| e.to_string())?;
    let reading_progress = store.reading_progress(id).map_err(|e| e.to_string())?;
    article.canonical = Some(canonical_article_body(
        store, &article, plugins, database, workspace,
    )?);
    let options = RenderOptions {
        show_translation,
        translation_layout,
        hide_images,
        paragraph_indent,
    };
    let key = RenderCacheKey::from_article(
        &article,
        show_translation,
        translation_layout,
        hide_images,
        paragraph_indent,
    );
    let body = if let Some(body) = RenderCache::lock(cache).get(&key) {
        body
    } else {
        let body = RenderCache::prepare(&article, options)
            .map_err(|error| format!("Article {id}: {error}"))?;
        RenderCache::lock(cache).insert(key, body)
    };
    Ok(PreparedArticle {
        article,
        body_html: body.html.clone(),
        body_markdown: body.markdown.clone(),
        reading_progress,
        image_urls: body.image_urls.clone(),
    })
}

fn canonical_article_body(
    store: &Store,
    article: &panda_core::Article,
    plugins: &PluginRegistry,
    database: &DbWriter,
    workspace: &str,
) -> Result<CanonicalArticle, String> {
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
    let pipeline = format!("canonical-html-v1:{}", plugins.cache_key());
    let revision = Store::canonical_revision(source, source_url, &article.summary.title, &pipeline);
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
        return Ok(CanonicalArticle {
            html: CanonicalHtml::new(cached),
            revision,
            pipeline_revision: pipeline.into(),
        });
    }
    let raw = RawHtml::new(source.to_owned());
    let processed = plugins.process(
        raw.as_str(),
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
        panda_content::remove_leading_duplicate_title(
            processed.html.as_ref(),
            &article.summary.title,
        )
    } else {
        processed.html.into_owned()
    };
    let canonical_html = panda_content::sanitize_html(&body, Some(source_url));
    let id = article.summary.id;
    let source = raw.as_str().to_owned();
    let source_url = source_url.to_owned();
    let title = article.summary.title.clone();
    let pipeline_for_write = pipeline.clone();
    let cached = canonical_html.clone();
    database.enqueue(workspace.to_owned(), move |store| {
        store.save_processed_content(
            id,
            &source,
            &source_url,
            &title,
            &pipeline_for_write,
            &cached,
        )
    })?;
    Ok(CanonicalArticle {
        html: CanonicalHtml::new(canonical_html),
        revision,
        pipeline_revision: pipeline.into(),
    })
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
        let store = Store::open_read_workspace(&path, &workspace).map_err(|e| e.to_string())?;
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
    let database = state.database.clone();
    job(reply, move |_| {
        let store = Store::open_read_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let plugins = read_lock(&plugins, "plugin registry");
        prepare_article(
            &store,
            &cache,
            id,
            show_translation,
            translation_layout,
            hide_images,
            paragraph_indent,
            &plugins,
            &database,
            &workspace,
        )
    });
}

pub(super) fn save_reading_progress(
    id: i64,
    progress: f32,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let database = state.database.clone();
    let workspace = state.workspace();
    job(reply, move |_| {
        database.write(workspace, move |store| {
            store.save_reading_progress(id, progress)
        })
    });
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
    let database = state.database.clone();
    job(reply, move |runtime| {
        let count = database.write(workspace.clone(), move |store| {
            store.mark_all_read(scope).map_err(Into::into)
        })?;
        if let (Some(kind), Some(config)) = (provider, config) {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime.block_on(flush_remote_marks(&path, &workspace, &database, &remote))?;
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
    let database = state.database.clone();
    job(reply, move |runtime| {
        let remote_id = database.write(workspace.clone(), move |store| {
            let remote_id = store.remote_entry_id(id)?;
            store.mark(id, field, value)?;
            Ok(remote_id)
        })?;
        if remote_id.is_some()
            && !matches!(field, MarkField::Later)
            && let (Some(kind), Some(config)) = (provider, config)
        {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime.block_on(flush_remote_marks(&path, &workspace, &database, &remote))?;
        }
        Ok(())
    });
}

async fn flush_remote_marks(
    path: &std::path::Path,
    workspace: &str,
    database: &DbWriter,
    remote: &ProviderClient,
) -> Result<(), String> {
    let pending = Store::open_read_workspace(path, workspace)
        .map_err(|error| error.to_string())?
        .pending_remote_marks()
        .map_err(|error| error.to_string())?;
    let mut read = Vec::new();
    let mut unread = Vec::new();
    let mut starred = Vec::new();
    let mut unstarred = Vec::new();
    for mark in &pending {
        match mark.field.as_str() {
            "is_read" if mark.value => read.push(mark.remote_id),
            "is_read" => unread.push(mark.remote_id),
            "is_starred" if mark.value => starred.push(mark.remote_id),
            "is_starred" => unstarred.push(mark.remote_id),
            _ => {}
        }
    }
    remote
        .mark_entries_status(&read, true)
        .await
        .map_err(|error| error.to_string())?;
    remote
        .mark_entries_status(&unread, false)
        .await
        .map_err(|error| error.to_string())?;
    remote
        .set_starred_entries(&starred, true)
        .await
        .map_err(|error| error.to_string())?;
    remote
        .set_starred_entries(&unstarred, false)
        .await
        .map_err(|error| error.to_string())?;
    database.write(workspace.to_owned(), move |store| {
        store.acknowledge_remote_marks(&pending).map_err(Into::into)
    })?;
    Ok(())
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
    let database = state.database.clone();
    job(reply, move |runtime| {
        let store = Store::open_read_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let plugins = read_lock(&plugins, "plugin registry");
        let pipeline_hash = format!("{}:{extractor:?}", plugins.cache_key());
        let prepared = runtime
            .block_on(store.prepare_extraction_using(
                id,
                force,
                extractor,
                &pipeline_hash,
                |raw, url, title| {
                    let raw = RawHtml::new(raw.to_owned());
                    let prepared = plugins.process(raw.as_str(), url, title, PluginStage::Prepare);
                    for diagnostic in &prepared.diagnostics {
                        eprintln!(
                            "article plugin {} failed: {}",
                            diagnostic.plugin_id, diagnostic.message
                        );
                    }
                    Ok((!prepared.matched_plugins.is_empty())
                        .then_some((prepared.html.into_owned(), prepared.body_selected)))
                },
            ))
            .map_err(|e| e.to_string());
        let result = match prepared {
            Ok(Some(prepared)) => database.write(workspace.clone(), move |store| {
                store.persist_extraction(prepared).map_err(Into::into)
            }),
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        if result.is_ok() {
            RenderCache::lock(&cache).invalidate_article(id);
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
    let database = state.database.clone();
    job(reply, move |runtime| {
        let config = TranslatorConfig::load(&translator_path).map_err(|e| e.to_string())?;
        let translator = panda_translate::build(&config).map_err(|e| e.to_string())?;
        let store = Store::open_read_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let mut article = store.article(id).map_err(|e| e.to_string())?;
        let registry = read_lock(&plugins, "plugin registry");
        let canonical = canonical_article_body(&store, &article, &registry, &database, &workspace)?;
        drop(registry);
        let source_hash = panda_translate::translation_revision_hash(
            canonical.revision.0.as_ref(),
            translator.id(),
            &target_lang,
        );
        let legacy_hash = panda_translate::translation_cache_hash(
            canonical.html.as_str(),
            article.summary.title.trim(),
            translator.id(),
        );
        let cached_for_target = article.translated_lang.as_deref() == Some(target_lang.as_str())
            && article
                .translated_html
                .as_deref()
                .is_some_and(|html| !html.trim().is_empty())
            && article.translation_source_hash.as_deref() == Some(source_hash.as_str());
        let reusable_legacy = article.translated_lang.as_deref() == Some(target_lang.as_str())
            && article
                .translated_html
                .as_deref()
                .is_some_and(|html| !html.trim().is_empty())
            && article.translation_source_hash.as_deref() == Some(legacy_hash.as_str());
        if reusable_legacy {
            let body = article.translated_html.clone().unwrap_or_default();
            let title = article.translated_title.clone();
            let lang = target_lang.clone();
            let hash = source_hash.clone();
            database.write(workspace.clone(), move |store| {
                store.persist_translation(id, &lang, &hash, &body, title.as_deref())
            })?;
            article.translation_source_hash = Some(source_hash.clone());
        } else if !cached_for_target {
            let result = runtime
                .block_on(translator.translate(panda_translate::TranslateRequest {
                    html: canonical.html.as_str().to_owned(),
                    title: Some(article.summary.title.clone()),
                    target_lang: target_lang.clone(),
                }))
                .map_err(|error| error.to_string())?;
            let title = article.summary.title.clone();
            let title_for_write = title.clone();
            let translated_title = result.title.clone();
            let body = result.html.clone();
            let lang = target_lang.clone();
            let hash = source_hash.clone();
            let title_hash = panda_translate::title_source_hash(&title);
            database.write(workspace.clone(), move |store| {
                store.persist_translation(id, &lang, &hash, &body, translated_title.as_deref())?;
                if store.article(id)?.summary.feed_auto_translate_titles
                    && let Some(translated_title) = translated_title.as_deref()
                {
                    store.save_auto_translated_title(
                        id,
                        &title_for_write,
                        translated_title,
                        &lang,
                        &title_hash,
                    )?;
                }
                Ok(())
            })?;
            article.translated_html = Some(result.html);
            article.translated_title = result.title;
            article.translated_lang = Some(target_lang.clone());
            article.translation_source_hash = Some(source_hash);
            if article.summary.feed_auto_translate_titles {
                if let Some(translated_title) = article.translated_title.as_deref() {
                    article.summary.auto_translated_title = Some(translated_title.to_owned());
                    article.summary.auto_translated_title_lang = Some(target_lang.clone());
                    article.summary.auto_translated_title_source_hash =
                        Some(panda_translate::title_source_hash(&title));
                }
            }
        }
        article.canonical = Some(canonical);
        RenderCache::lock(&cache).invalidate_article(id);
        let options = RenderOptions {
            show_translation: true,
            translation_layout,
            hide_images,
            paragraph_indent,
        };
        let key = RenderCacheKey::from_article(
            &article,
            true,
            translation_layout,
            hide_images,
            paragraph_indent,
        );
        let body = if let Some(body) = RenderCache::lock(&cache).get(&key) {
            body
        } else {
            let body = RenderCache::prepare(&article, options)
                .map_err(|error| format!("Article {id}: {error}"))?;
            RenderCache::lock(&cache).insert(key, body)
        };
        Ok(PreparedArticle {
            article,
            body_html: body.html.clone(),
            body_markdown: body.markdown.clone(),
            reading_progress: store.reading_progress(id).map_err(|e| e.to_string())?,
            image_urls: body.image_urls.clone(),
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
        Store::open_read_workspace(&path, &workspace)
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
    let database = state.database.clone();
    job(reply, move |runtime| {
        let _serial = lock_mutex(&lock, "title translation serialization");
        let received = items.len();
        let config = match TranslatorConfig::load(&translator_path) {
            Ok(config) => config,
            Err(error) => {
                log_title_translation(&path, &format!("settings read failed: {error:#}"));
                return Err(error.to_string());
            }
        };
        let provider = config.provider.id().to_owned();
        let store = match Store::open_read_workspace(&path, &workspace) {
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
            if lock_mutex(&attempted, "title translation backoff")
                .get(&key)
                .is_some_and(|last_attempt| {
                    last_attempt.elapsed() < std::time::Duration::from_secs(60)
                })
            {
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
        lock_mutex(&attempted, "title translation backoff").extend(
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
                    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
                    let provider_name = provider.clone();
                    database.write(workspace.clone(), move |store| {
                        store
                            .record_translation_usage(
                                &day,
                                &provider_name,
                                failure.requests as u64,
                                failure.characters as u64,
                            )
                            .map_err(Into::into)
                    })?;
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
            let day = chrono::Local::now().format("%Y-%m-%d").to_string();
            let provider_name = provider.clone();
            let requests = result.requests as u64;
            database.write(workspace.clone(), move |store| {
                store
                    .record_translation_usage(&day, &provider_name, requests, chars)
                    .map_err(Into::into)
            })?;
        }
        let mut empty_results = 0usize;
        let mut newly_translated = 0usize;
        for ((item, hash), text) in eligible.into_iter().zip(hashes).zip(result.translations) {
            if !text.trim().is_empty() {
                let translated_title = text.clone();
                let title = item.title.clone();
                let lang = target_lang.clone();
                database.write(workspace.clone(), move |store| {
                    store
                        .save_auto_translated_title(
                            item.id,
                            &title,
                            &translated_title,
                            &lang,
                            &hash,
                        )
                        .map_err(Into::into)
                })?;
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
    let canonical_revision = Store::canonical_revision(
        "<p>Original</p><img src=\"https://example.com/image.png\">",
        "",
        "Example article",
        "test-pipeline-v1",
    );
    panda_core::Article {
        summary: panda_core::ArticleSummary {
            id: 1,
            feed_id: 1,
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
        content_revision: 0,
        url: None,
        content_html: "<p>Original</p><img src=\"https://example.com/image.png\">".into(),
        source_html: None,
        source_page_html: None,
        extracted_html: None,
        canonical: Some(CanonicalArticle {
            html: CanonicalHtml::new("<p>Original</p><img src=\"https://example.com/image.png\">"),
            revision: canonical_revision.clone(),
            pipeline_revision: "test-pipeline-v1".into(),
        }),
        translated_html: Some("<p>译文</p>".into()),
        translated_title: None,
        translated_lang: Some("zh".into()),
        translation_source_hash: Some(panda_translate::translation_revision_hash(
            canonical_revision.0.as_ref(),
            "test-provider",
            "zh",
        )),
    }
}
