pub(super) mod cache;
mod content;

use super::command::{TitleTranslationInput, TitleTranslationOutcome, TitleTranslationStatus};
use super::database::DbWriter;
use super::worker::{
    TranslationJobKey, WorkerState, finish_article_work, job, lock_mutex, read_lock,
    register_article_work, remote_mark_lock, try_job, wait_for_article_work_cancel,
};
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

type TranslationWaiters = Mutex<
    std::collections::HashMap<
        TranslationJobKey,
        Vec<(
            oneshot::Sender<Result<PreparedArticle, String>>,
            Option<tokio::sync::mpsc::UnboundedSender<PreparedArticle>>,
        )>,
    >,
>;

fn add_translation_waiter(
    jobs: &TranslationWaiters,
    key: TranslationJobKey,
    reply: oneshot::Sender<Result<PreparedArticle, String>>,
    progress: tokio::sync::mpsc::UnboundedSender<PreparedArticle>,
) -> bool {
    let mut jobs = lock_mutex(jobs, "translation task deduplication");
    if let Some(waiters) = jobs.get_mut(&key) {
        waiters.push((reply, Some(progress)));
        false
    } else {
        jobs.insert(key, vec![(reply, Some(progress))]);
        true
    }
}

fn send_translation_progress(
    jobs: &TranslationWaiters,
    key: &TranslationJobKey,
    prepared: PreparedArticle,
) {
    if let Some(waiters) = lock_mutex(jobs, "translation task progress").get_mut(key) {
        for (_, progress) in waiters {
            if progress
                .as_ref()
                .is_some_and(|sender| sender.send(prepared.clone()).is_err())
            {
                *progress = None;
            }
        }
    }
}

fn finish_translation_job(
    jobs: &TranslationWaiters,
    key: &TranslationJobKey,
    result: Result<PreparedArticle, String>,
) {
    let waiters = lock_mutex(jobs, "translation task deduplication")
        .remove(key)
        .unwrap_or_default();
    for (waiter, _) in waiters {
        let _ = waiter.send(result.clone());
    }
}

fn translation_is_latest(
    latest: &Mutex<std::collections::HashMap<(String, i64), TranslationJobKey>>,
    key: &TranslationJobKey,
) -> bool {
    lock_mutex(latest, "latest translation task").get(&(key.0.clone(), key.1)) == Some(key)
}

fn clear_latest_translation(
    latest: &Mutex<std::collections::HashMap<(String, i64), TranslationJobKey>>,
    key: &TranslationJobKey,
) {
    let mut latest = lock_mutex(latest, "latest translation task");
    let article = (key.0.clone(), key.1);
    if latest.get(&article) == Some(key) {
        latest.remove(&article);
    }
}

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
    job(reply, move |_| {
        let count = database.write(workspace.clone(), move |store| {
            store.mark_all_read(scope).map_err(Into::into)
        })?;
        if let (Some(kind), Some(config)) = (provider, config) {
            queue_remote_mark_flush(path, workspace, database, kind, config);
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
    job(reply, move |_| {
        let remote_id = database.write(workspace.clone(), move |store| {
            let remote_id = store.remote_entry_id(id)?;
            store.mark(id, field, value)?;
            Ok(remote_id)
        })?;
        if remote_id.is_some()
            && !matches!(field, MarkField::Later)
            && let (Some(kind), Some(config)) = (provider, config)
        {
            queue_remote_mark_flush(path, workspace, database, kind, config);
        }
        Ok(())
    });
}

fn queue_remote_mark_flush(
    path: std::path::PathBuf,
    workspace: String,
    database: DbWriter,
    kind: panda_providers::ProviderKind,
    config: panda_providers::ProviderSettings,
) {
    // Local read state is already committed; remote sync must not hold the UI response open.
    let (reply, _result) = oneshot::channel();
    let remote_lock = remote_mark_lock(&workspace);
    job(reply, move |runtime| {
        let result = match ProviderClient::new(kind, &config) {
            Ok(remote) => runtime.block_on(async {
                let _guard = remote_lock.lock().await;
                flush_remote_marks(&path, &workspace, &database, &remote).await
            }),
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = &result {
            eprintln!("could not sync article read state to {kind:?}: {error}");
        }
        result
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
    let cancellations = state.article_work_cancellations.clone();
    let (cancel_id, cancel_receiver) = register_article_work(&cancellations, &workspace, id);
    let cleanup_cancellations = cancellations.clone();
    let cleanup_workspace = workspace.clone();
    let submitted = try_job(reply, move |runtime| {
        let result = (|| -> Result<(), String> {
            let store = Store::open_read_workspace(&path, &workspace).map_err(|e| e.to_string())?;
            let plugins = read_lock(&plugins, "plugin registry");
            let pipeline_hash = format!("{}:{extractor:?}", plugins.cache_key());
            let prepared = runtime.block_on(async {
            tokio::select! {
                _ = wait_for_article_work_cancel(cancel_receiver) => Err("Article task cancelled".to_owned()),
                result = store.prepare_extraction_using(
                id,
                force,
                extractor,
                &pipeline_hash,
                |url| {
                    let resolved = plugins.resolve_article_url(url);
                    for diagnostic in &resolved.diagnostics {
                        eprintln!(
                            "article URL resolver {} failed: {}",
                            diagnostic.plugin_id, diagnostic.message
                        );
                    }
                    if let Some(diagnostic) = resolved.diagnostics.first() {
                        return Err(anyhow::anyhow!(diagnostic.message.clone()));
                    }
                    Ok(resolved.url)
                },
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
                ) => result.map_err(|e| e.to_string()),
            }
        });
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
        })();
        finish_article_work(&cleanup_cancellations, &workspace, id, cancel_id);
        result
    });
    if !submitted {
        finish_article_work(&cancellations, &cleanup_workspace, id, cancel_id);
    }
}

pub(super) fn translate(
    id: i64,
    target_lang: String,
    content_revision: i64,
    translator_id: String,
    translation_layout: TranslationLayout,
    hide_images: bool,
    paragraph_indent: bool,
    reply: oneshot::Sender<Result<PreparedArticle, String>>,
    progress: tokio::sync::mpsc::UnboundedSender<PreparedArticle>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let translator_path = state.translator_path.clone();
    let cache = state.body_cache.clone();
    let plugins = state.plugin_registry.clone();
    let database = state.database.clone();
    let translation_jobs = state.translation_jobs.clone();
    let latest_translation = state.latest_translation.clone();
    let plugin_revision = format!(
        "{}:{translation_layout:?}:{hide_images}:{paragraph_indent}",
        read_lock(&plugins, "plugin registry").cache_key()
    );
    let key = (
        workspace.clone(),
        id,
        target_lang.clone(),
        content_revision,
        translator_id,
        plugin_revision,
    );
    if !add_translation_waiter(&translation_jobs, key.clone(), reply, progress) {
        return;
    }
    lock_mutex(&latest_translation, "latest translation task")
        .insert((workspace.clone(), id), key.clone());
    let cancellations = state.article_work_cancellations.clone();
    let (cancel_id, cancel_receiver) = register_article_work(&cancellations, &workspace, id);
    let (completion, _unused) = oneshot::channel();
    let failure_jobs = translation_jobs.clone();
    let failure_key = key.clone();
    let cleanup_cancellations = cancellations.clone();
    let cleanup_workspace = workspace.clone();
    let completion_latest = latest_translation.clone();
    let rejected_latest = latest_translation.clone();
    let submitted = try_job(completion, move |runtime| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (|| -> Result<PreparedArticle, String> {
                if !translation_is_latest(&latest_translation, &key) {
                    return Err("Translation request was superseded".into());
                }
                let config = TranslatorConfig::load(&translator_path).map_err(|e| e.to_string())?;
                let translator = panda_translate::build(&config).map_err(|e| e.to_string())?;
                let store =
                    Store::open_read_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                let mut article = store.article(id).map_err(|e| e.to_string())?;
                let expected_content_revision = article.content_revision;
                let registry = read_lock(&plugins, "plugin registry");
                let canonical =
                    canonical_article_body(&store, &article, &registry, &database, &workspace)?;
                drop(registry);
                let prompt_revision = translator.resumable_prompt_revision();
                let backend_id = translator.cache_id();
                let translation_context_hash =
                    panda_translate::title_source_hash(article.summary.title.trim());
                let translation_backend = prompt_revision
                    .map(|revision| format!("{backend_id}:{revision}"))
                    .unwrap_or_else(|| backend_id.clone());
                let source_hash = panda_translate::translation_revision_hash(
                    canonical.revision.0.as_ref(),
                    &translation_backend,
                    &target_lang,
                );
                let legacy_hash = panda_translate::translation_cache_hash(
                    canonical.html.as_str(),
                    article.summary.title.trim(),
                    &translator.cache_id(),
                );
                let cached_for_target = article.translated_lang.as_deref()
                    == Some(target_lang.as_str())
                    && article
                        .translated_html
                        .as_deref()
                        .is_some_and(|html| !html.trim().is_empty())
                    && article.translation_source_hash.as_deref() == Some(source_hash.as_str());
                let reusable_legacy = prompt_revision.is_none()
                    && article.translated_lang.as_deref()
                    == Some(target_lang.as_str())
                    && article
                        .translated_html
                        .as_deref()
                        .is_some_and(|html| !html.trim().is_empty())
                    && article.translation_source_hash.as_deref() == Some(legacy_hash.as_str());
                if reusable_legacy {
                    if !translation_is_latest(&latest_translation, &key) {
                        return Err("Translation request was superseded".into());
                    }
                    let body = article.translated_html.clone().unwrap_or_default();
                    let title = article.translated_title.clone();
                    let lang = target_lang.clone();
                    let hash = source_hash.clone();
                    let latest = latest_translation.clone();
                    let expected_key = key.clone();
                    database.write(workspace.clone(), move |store| {
                        let latest = lock_mutex(&latest, "latest translation task");
                        if latest.get(&(expected_key.0.clone(), expected_key.1))
                            != Some(&expected_key)
                        {
                            anyhow::bail!("Translation request was superseded");
                        }
                        store.persist_translation(
                            id,
                            expected_content_revision,
                            &lang,
                            &hash,
                            &body,
                            title.as_deref(),
                        )
                    })?;
                    article.translation_source_hash = Some(source_hash.clone());
                } else if !cached_for_target {
                    if !translation_is_latest(&latest_translation, &key) {
                        return Err("Translation request was superseded".into());
                    }
                    let cached_segments = match prompt_revision {
                        Some(prompt_revision) => store
                            .translation_segments(
                                id,
                                &target_lang,
                                &backend_id,
                                prompt_revision,
                                &translation_context_hash,
                            )
                            .map_err(|error| format!("Could not load translation progress: {error:#}"))?,
                        None => Default::default(),
                    };
                    let database_for_segments = database.clone();
                    let workspace_for_segments = workspace.clone();
                    let lang_for_segments = target_lang.clone();
                    let backend_for_segments = backend_id.clone();
                    let context_hash_for_segments = translation_context_hash.clone();
                    let progress_jobs = translation_jobs.clone();
                    let progress_key = key.clone();
                    let progress_segments = panda_translate::html::prepare_html_translation_segments(
                        canonical.html.as_str(),
                    );
                    let progress_blocks = panda_translate::html::split_blocks(canonical.html.as_str());
                    let source_hash_for_progress = source_hash.clone();
                    let lang_for_progress = target_lang.clone();
                    let mut progressive_translations = std::collections::HashMap::new();
                    let mut progress_article = article.clone();
                    progress_article.canonical = Some(canonical.clone());
                    let translated = runtime.block_on(async {
                        tokio::select! {
                            _ = wait_for_article_work_cancel(cancel_receiver) => {
                                Err("Article task cancelled".to_owned())
                            }
                            result = translator.translate_with_cached_segments_and_progress(
                                panda_translate::TranslateRequest {
                                    html: canonical.html.as_str().to_owned(),
                                    title: Some(article.summary.title.clone()),
                                    target_lang: target_lang.clone(),
                                },
                                cached_segments,
                                move |segment_id, source_hash, translated_html| {
                                    let segment_id = segment_id.to_owned();
                                    let source_hash = source_hash.to_owned();
                                    let translated_html = translated_html.to_owned();
                                    let prompt_revision = prompt_revision
                                        .ok_or_else(|| anyhow::anyhow!("Missing translation prompt revision"))?;
                                    let language = lang_for_segments.clone();
                                    let backend = backend_for_segments.clone();
                                    let context_hash = context_hash_for_segments.clone();
                                    database_for_segments
                                        .write(workspace_for_segments.clone(), move |store| {
                                            store.save_translation_segment(
                                                id,
                                                &language,
                                                &backend,
                                                prompt_revision,
                                                &context_hash,
                                                &segment_id,
                                                &source_hash,
                                                &translated_html,
                                            )
                                        })
                                        .map_err(anyhow::Error::msg)
                                },
                                move |completed| {
                                    progressive_translations.extend(completed.iter().cloned());
                                    // A batch delivers several segments together. Coalesce that
                                    // burst instead of converting the whole article for every item.
                                    // Let the final reply display the complete, persisted result.
                                    if progressive_translations.len() == progress_segments.len() {
                                        return Ok(());
                                    }
                                    let mut blocks = progress_blocks.clone();
                                    for segment in &progress_segments {
                                        if let Some(translated) = progressive_translations.get(&segment.id) {
                                            blocks[segment.block_index] = translated.clone();
                                        }
                                    }
                                    let partial_html = blocks.join("\n");
                                    let mut article = progress_article.clone();
                                    article.translated_html = Some(partial_html);
                                    article.translated_lang = Some(lang_for_progress.clone());
                                    article.translation_source_hash =
                                        Some(source_hash_for_progress.clone());
                                    let options = RenderOptions {
                                        show_translation: true,
                                        translation_layout,
                                        hide_images,
                                        paragraph_indent,
                                    };
                                    let Ok(body) = RenderCache::prepare(&article, options) else {
                                        return Ok(());
                                    };
                                    send_translation_progress(
                                        &progress_jobs,
                                        &progress_key,
                                        PreparedArticle {
                                            article,
                                            body_html: body.html,
                                            body_markdown: body.markdown,
                                            reading_progress: 0.0,
                                            image_urls: body.image_urls,
                                        },
                                    );
                                    Ok(())
                                },
                            ) => result.map_err(|error| error.to_string()),
                        }
                    })?;
                    let result = translated;
                    let title = article.summary.title.clone();
                    let title_for_write = title.clone();
                    let translated_title = result.title.clone();
                    let body = result.html.clone();
                    let lang = target_lang.clone();
                    let hash = source_hash.clone();
                    let title_hash = panda_translate::title_source_hash(&title);
                    let metrics = result.metrics.clone();
                    let usage_day = chrono::Local::now().format("%Y-%m-%d").to_string();
                    let usage_provider = translator.id().to_owned();
                    let usage_characters = (canonical.html.0.chars().count() + title.chars().count()) as u64;
                    if !translation_is_latest(&latest_translation, &key) {
                        return Err("Translation request was superseded".into());
                    }
                    let latest = latest_translation.clone();
                    let expected_key = key.clone();
                    database.write(workspace.clone(), move |store| {
                        let latest = lock_mutex(&latest, "latest translation task");
                        if latest.get(&(expected_key.0.clone(), expected_key.1))
                            != Some(&expected_key)
                        {
                            anyhow::bail!("Translation request was superseded");
                        }
                        store.persist_translation(
                            id,
                            expected_content_revision,
                            &lang,
                            &hash,
                            &body,
                            translated_title.as_deref(),
                        )?;
                        if let Some(prompt_revision) = prompt_revision {
                            store.prune_translation_segments(
                                id,
                                &lang,
                                &backend_id,
                                prompt_revision,
                                &translation_context_hash,
                            )?;
                        }
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
                        if metrics.requests > 0 {
                            store.record_translation_usage_with_tokens(
                                &usage_day,
                                &usage_provider,
                                metrics.requests,
                                usage_characters,
                                metrics.input_tokens.unwrap_or(0),
                                metrics.output_tokens.unwrap_or(0),
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
                            article.summary.auto_translated_title =
                                Some(translated_title.to_owned());
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
            })()
        }))
        .unwrap_or_else(|_| Err("Translation task panicked".to_owned()));
        clear_latest_translation(&completion_latest, &key);
        finish_article_work(&cleanup_cancellations, &workspace, id, cancel_id);
        finish_translation_job(&translation_jobs, &key, result.clone());
        result
    });
    if !submitted {
        clear_latest_translation(&rejected_latest, &failure_key);
        finish_article_work(&cancellations, &cleanup_workspace, id, cancel_id);
        finish_translation_job(
            &failure_jobs,
            &failure_key,
            Err("Background task queue is full; try again shortly".into()),
        );
    }
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
    force: bool,
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
            if !force && !article.summary.feed_auto_translate_titles {
                disabled_feed += 1;
                continue;
            }
            if !force
                && article.summary.auto_translated_title_lang.as_deref()
                    == Some(target_lang.as_str())
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
            if !force
                && lock_mutex(&attempted, "title translation backoff")
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
                            .record_translation_usage_with_tokens(
                                &day,
                                &provider_name,
                                failure.requests as u64,
                                failure.characters as u64,
                                failure.input_tokens.unwrap_or(0),
                                failure.output_tokens.unwrap_or(0),
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
            let input_tokens = result.input_tokens.unwrap_or(0);
            let output_tokens = result.output_tokens.unwrap_or(0);
            database.write(workspace.clone(), move |store| {
                store
                    .record_translation_usage_with_tokens(
                        &day,
                        &provider_name,
                        requests,
                        chars,
                        input_tokens,
                        output_tokens,
                    )
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

#[cfg(test)]
mod translation_dedup_tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn identical_translation_requests_share_one_in_flight_result() {
        let jobs: TranslationWaiters = Mutex::new(HashMap::new());
        let key = (
            "provider:miniflux".into(),
            42,
            "zh-Hans".into(),
            7,
            "translator-v1".into(),
            "pipeline-v1".into(),
        );
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();
        let (first_progress, _first_progress_rx) = tokio::sync::mpsc::unbounded_channel();
        let (second_progress, _second_progress_rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(add_translation_waiter(
            &jobs,
            key.clone(),
            first_tx,
            first_progress
        ));
        assert!(!add_translation_waiter(
            &jobs,
            key.clone(),
            second_tx,
            second_progress
        ));
        finish_translation_job(&jobs, &key, Err("mock translation failure".into()));

        assert!(
            matches!(first_rx.await.unwrap(), Err(error) if error == "mock translation failure")
        );
        assert!(
            matches!(second_rx.await.unwrap(), Err(error) if error == "mock translation failure")
        );
        assert!(lock_mutex(&jobs, "test translation jobs").is_empty());
    }
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
            title_is_future: false,
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
