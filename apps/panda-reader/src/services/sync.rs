use super::articles::cache::RenderCache;
use super::database::DbWriter;
use super::diagnostics::{LogDetail, write_sync_log};
use super::worker::{WorkerState, job, remote_mark_lock};
use panda_providers::{ProviderClient, ProviderKind, SyncMode};
use panda_store::{PendingRemoteMark, Store};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tokio::sync::oneshot;

fn sync_log(
    database_path: &Path,
    detailed_logging: &AtomicBool,
    detail: LogDetail,
    level: &str,
    message: impl AsRef<str>,
) {
    write_sync_log(database_path, detailed_logging, detail, level, message);
}

pub(super) fn refresh_feed(
    id: i64,
    reply: oneshot::Sender<Result<usize, String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let provider = state.provider_kind();
    let config = provider.and_then(|kind| state.provider_settings(kind));
    let database = state.database.clone();
    let detailed_logging = state.detailed_sync_logging.clone();
    job(reply, move |runtime| {
        let started = Instant::now();
        let result = (|| {
            if let (Some(kind), Some(config)) = (provider, config) {
                let remote = match ProviderClient::new(kind, &config) {
                    Ok(remote) => remote,
                    Err(error) => {
                        let message = format!("{} provider setup failed: {error:#}", kind.key());
                        sync_log(
                            &path,
                            &detailed_logging,
                            LogDetail::Summary,
                            "ERROR",
                            &message,
                        );
                        return Err(message);
                    }
                };
                let input = Store::open_read_workspace(&path, &workspace)
                    .and_then(|store| store.remote_feed_id(id))
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "Provider feed has no remote ID".to_owned())?;
                let started = Instant::now();
                if let Err(error) = runtime.block_on(remote.refresh_feed(input)) {
                    let message = format!("{} single-feed refresh failed: {error:#}", kind.key());
                    sync_log(
                        &path,
                        &detailed_logging,
                        LogDetail::Summary,
                        "ERROR",
                        &message,
                    );
                    return Err(message);
                }
                sync_log(
                    &path,
                    &detailed_logging,
                    LogDetail::Summary,
                    "INFO",
                    format!(
                        "provider={} single-feed refresh accepted elapsed_ms={}",
                        kind.key(),
                        started.elapsed().as_millis()
                    ),
                );
                sync_provider(
                    &path,
                    &workspace,
                    &database,
                    &remote,
                    kind,
                    &detailed_logging,
                    runtime,
                )
            } else if provider.is_some() {
                Err("Connect the selected provider before syncing".into())
            } else {
                let input = Store::open_read_workspace(&path, &workspace)
                    .and_then(|store| store.local_feed_refresh_input(id))
                    .map_err(|e| e.to_string())?;
                match runtime.block_on(Store::fetch_feed_data(
                    &input.url,
                    input.etag.as_deref(),
                    input.modified.as_deref(),
                )) {
                    Ok(fetched) => {
                        let revisions = Store::open_read_workspace(&path, &workspace)
                            .and_then(|store| store.article_source_revisions(input.id))
                            .map_err(|error| error.to_string())?;
                        let prepared = Store::prepare_fetched_feed_with_revisions(
                            &input.url, fetched, &revisions,
                        )
                        .map_err(|error| error.to_string())?;
                        let changed = !prepared.is_not_modified();
                        database.write(workspace, move |store| {
                            store.persist_prepared_feed(&input.url, Some(input.id), prepared)?;
                            Ok(usize::from(changed))
                        })
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let failure = message.clone();
                        database.enqueue(workspace, move |store| {
                            store.set_feed_refresh_error(id, Some(&failure))
                        })?;
                        Err(message)
                    }
                }
            }
        })();
        match &result {
            Ok(count) => sync_log(
                &path,
                &detailed_logging,
                LogDetail::Summary,
                "INFO",
                format!(
                    "single-feed refresh completed feed_id={id} saved={count} elapsed_ms={}",
                    started.elapsed().as_millis()
                ),
            ),
            Err(error) => sync_log(
                &path,
                &detailed_logging,
                LogDetail::Summary,
                "ERROR",
                format!(
                    "single-feed refresh failed feed_id={id} elapsed_ms={} error={error}",
                    started.elapsed().as_millis()
                ),
            ),
        }
        result
    });
}

pub(super) fn refresh(
    force: bool,
    reply: oneshot::Sender<Result<usize, String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let provider = state.provider_kind();
    let config = provider.and_then(|kind| state.provider_settings(kind));
    let cache = state.body_cache.clone();
    let database = state.database.clone();
    let detailed_logging = state.detailed_sync_logging.clone();
    job(reply, move |runtime| {
        let started = Instant::now();
        sync_log(
            &path,
            &detailed_logging,
            LogDetail::Summary,
            "INFO",
            format!(
                "sync started workspace={workspace} provider={} force={force}",
                provider.map_or("local", ProviderKind::key),
            ),
        );
        let result = if let (Some(kind), Some(config)) = (provider, config) {
            let remote = match ProviderClient::new(kind, &config) {
                Ok(remote) => remote,
                Err(error) => {
                    let message = format!("{} provider setup failed: {error:#}", kind.key());
                    sync_log(
                        &path,
                        &detailed_logging,
                        LogDetail::Summary,
                        "ERROR",
                        &message,
                    );
                    return Err(message);
                }
            };
            if force {
                let refresh_started = Instant::now();
                if let Err(error) = runtime.block_on(remote.refresh_all()) {
                    let message = format!("{} refresh-all request failed: {error:#}", kind.key());
                    sync_log(
                        &path,
                        &detailed_logging,
                        LogDetail::Summary,
                        "ERROR",
                        &message,
                    );
                    return Err(message);
                }
                sync_log(
                    &path,
                    &detailed_logging,
                    LogDetail::Detailed,
                    "INFO",
                    format!(
                        "provider={} forced refresh-all accepted elapsed_ms={}",
                        kind.key(),
                        refresh_started.elapsed().as_millis()
                    ),
                );
            } else {
                sync_log(
                    &path,
                    &detailed_logging,
                    LogDetail::Detailed,
                    "INFO",
                    format!(
                        "provider={} using current server state; refresh-all skipped",
                        kind.key()
                    ),
                );
            }
            sync_provider(
                &path,
                &workspace,
                &database,
                &remote,
                kind,
                &detailed_logging,
                runtime,
            )
        } else if provider.is_some() {
            Err("Connect the selected provider before syncing".into())
        } else {
            refresh_local_feeds(&path, &workspace, &database, runtime, force)
        };
        if result.is_ok() {
            *RenderCache::lock(&cache) = RenderCache::default();
        }
        match &result {
            Ok(count) => sync_log(
                &path,
                &detailed_logging,
                LogDetail::Summary,
                "INFO",
                format!(
                    "sync completed saved={count} elapsed_ms={}",
                    started.elapsed().as_millis()
                ),
            ),
            Err(error) => sync_log(
                &path,
                &detailed_logging,
                LogDetail::Summary,
                "ERROR",
                format!(
                    "sync failed elapsed_ms={} error={error}",
                    started.elapsed().as_millis()
                ),
            ),
        }
        result
    });
}

fn refresh_local_feeds(
    path: &Path,
    workspace: &str,
    database: &DbWriter,
    runtime: &tokio::runtime::Runtime,
    force: bool,
) -> Result<usize, String> {
    let feeds = Store::open_read_workspace(path, workspace)
        .and_then(|store| store.local_feeds_to_refresh())
        .map_err(|error| error.to_string())?;
    let (refreshed, failures) = runtime.block_on(async {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(8));
        let mut tasks = tokio::task::JoinSet::new();
        for feed in feeds {
            let semaphore = semaphore.clone();
            tasks.spawn(async move {
                let permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                let result = Store::fetch_feed_data(
                    &feed.url,
                    if force { None } else { feed.etag.as_deref() },
                    if force {
                        None
                    } else {
                        feed.modified.as_deref()
                    },
                )
                .await;
                drop(permit);
                Ok::<_, String>((feed, result))
            });
        }
        let mut refreshed = 0;
        let mut failures = Vec::new();
        while let Some(result) = tasks.join_next().await {
            let (feed, result) =
                result.map_err(|error| format!("Feed refresh task failed: {error}"))??;
            match result {
                Ok(fetched) => {
                    let revisions = Store::open_read_workspace(path, workspace)
                        .and_then(|store| store.article_source_revisions(feed.id))
                        .map_err(|error| error.to_string());
                    let prepared = revisions.and_then(|revisions| {
                        Store::prepare_fetched_feed_with_revisions(&feed.url, fetched, &revisions)
                            .map_err(|error| error.to_string())
                    });
                    match prepared {
                        Ok(prepared) => {
                            let changed = !prepared.is_not_modified();
                            let url = feed.url.clone();
                            let display_url = url.clone();
                            let id = feed.id;
                            match database.write(workspace.to_owned(), move |store| {
                                store.persist_prepared_feed(&url, Some(id), prepared)?;
                                Ok(usize::from(changed))
                            }) {
                                Ok(count) => refreshed += count,
                                Err(error) => failures.push(format!("{display_url}: {error}")),
                            }
                        }
                        Err(error) => failures.push(format!("{}: {error}", feed.url)),
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    let failure = message.clone();
                    database
                        .enqueue(workspace.to_owned(), move |store| {
                            store.set_feed_refresh_error(feed.id, Some(&failure))
                        })
                        .map_err(|enqueue_error| format!("{message}; {enqueue_error}"))?;
                    failures.push(format!("{}: {message}", feed.url));
                }
            }
        }
        Ok::<_, String>((refreshed, failures))
    })?;
    if refreshed == 0 && !failures.is_empty() {
        return Err(format!(
            "All feeds failed to refresh: {}",
            failures.join("; ")
        ));
    }
    Ok(refreshed)
}

pub(super) fn sync_provider(
    path: &Path,
    workspace: &str,
    database: &DbWriter,
    remote: &ProviderClient,
    kind: ProviderKind,
    detailed_logging: &AtomicBool,
    runtime: &tokio::runtime::Runtime,
) -> Result<usize, String> {
    runtime.block_on(sync_provider_async(
        path,
        workspace,
        database,
        remote,
        kind,
        detailed_logging,
    ))
}

async fn sync_provider_async(
    path: &Path,
    workspace: &str,
    database: &DbWriter,
    remote: &ProviderClient,
    kind: ProviderKind,
    detailed_logging: &AtomicBool,
) -> Result<usize, String> {
    let stage_started = Instant::now();
    let identity = match remote.identity().await {
        Ok(identity) => identity,
        Err(error) => {
            let message = format!("provider={} identity request failed: {error:#}", kind.key());
            sync_log(
                path,
                detailed_logging,
                LogDetail::Detailed,
                "ERROR",
                &message,
            );
            return Err(message);
        }
    };
    sync_log(
        path,
        detailed_logging,
        LogDetail::Detailed,
        "INFO",
        format!(
            "provider={} identity request completed elapsed_ms={}",
            kind.key(),
            stage_started.elapsed().as_millis()
        ),
    );
    let stage_started = Instant::now();
    let feeds = match remote.feeds().await {
        Ok(feeds) => feeds,
        Err(error) => {
            let message = format!("provider={} feeds request failed: {error:#}", kind.key());
            sync_log(
                path,
                detailed_logging,
                LogDetail::Detailed,
                "ERROR",
                &message,
            );
            return Err(message);
        }
    };
    sync_log(
        path,
        detailed_logging,
        LogDetail::Detailed,
        "INFO",
        format!(
            "provider={} feeds request completed feeds={} elapsed_ms={}",
            kind.key(),
            feeds.len(),
            stage_started.elapsed().as_millis()
        ),
    );
    let prior = Store::open_read_workspace(path, workspace)
        .and_then(|store| store.provider_sync_state())
        .map_err(|e| e.to_string())?;
    let account_changed = prior
        .as_ref()
        .is_some_and(|state| state.account != identity.account);
    let provider_changed = prior
        .as_ref()
        .is_some_and(|state| !state.provider.is_empty() && state.provider != kind.key());
    let reset = account_changed || provider_changed;
    let staging_workspace = format!("{workspace}:staging");
    let sync_workspace = if reset {
        staging_workspace.as_str()
    } else {
        workspace
    };
    let cursor = if reset {
        None
    } else {
        prior.as_ref().and_then(|state| state.cursor.clone())
    };
    let full_sync_at = if reset {
        None
    } else {
        prior
            .as_ref()
            .and_then(|state| state.last_full_sync_at.clone())
    };
    let last_full_sync = full_sync_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc));
    let mode = if reset || prior.is_none() || cursor.is_none() {
        SyncMode::Initial
    } else if !remote.capabilities().incremental_sync
        || last_full_sync
            .is_none_or(|last| chrono::Utc::now().signed_duration_since(last).num_days() >= 7)
    {
        SyncMode::Reconcile
    } else {
        SyncMode::Incremental
    };
    sync_log(
        path,
        detailed_logging,
        LogDetail::Detailed,
        "INFO",
        format!(
            "provider={} sync mode={mode:?} feeds={} reset={reset} elapsed_ms={}",
            kind.key(),
            feeds.len(),
            stage_started.elapsed().as_millis()
        ),
    );

    let account = identity.account;
    let cursor_for_write = cursor.clone();
    let full_sync_for_write = full_sync_at.clone();
    database
        .write(sync_workspace.to_owned(), move |store| {
            store.begin_provider_sync(
                kind,
                &account,
                cursor_for_write.as_ref(),
                full_sync_for_write.as_deref(),
                reset,
            )
        })
        .map_err(|e| e.to_string())?;

    if !reset {
        let remote_lock = remote_mark_lock(workspace);
        let _guard = remote_lock.lock().await;
        flush_remote_marks(path, workspace, database, remote).await?;
    }

    let feed_ids = database
        .write(sync_workspace.to_owned(), move |store| {
            store.save_remote_feeds(&feeds, kind)
        })
        .map_err(|e| e.to_string())?;
    let mut cursor = cursor;
    let mut total = 0usize;
    let mut full_sync = matches!(mode, SyncMode::Initial | SyncMode::Reconcile);
    let mut page_number = 0usize;
    loop {
        page_number += 1;
        let page_started = Instant::now();
        let page = match remote.entries_page(mode, cursor.as_ref()).await {
            Ok(page) => page,
            Err(error) => {
                let message = format!(
                    "provider={} entries page {page_number} failed: {error:#}",
                    kind.key()
                );
                return Err(message);
            }
        };
        sync_log(
            path,
            detailed_logging,
            LogDetail::Detailed,
            "INFO",
            format!(
                "provider page={page_number} entries={} has_more={} fetch_ms={}",
                page.entries.len(),
                page.has_more,
                page_started.elapsed().as_millis()
            ),
        );
        full_sync |= page.full_sync;
        let next_cursor = page.next_cursor.clone();
        let cursor_for_write = next_cursor.clone();
        let full_at = (!page.has_more && full_sync).then(|| chrono::Utc::now().to_rfc3339());
        let full_at_for_write = full_at.clone();
        let known_revisions = Store::open_read_workspace(path, sync_workspace)
            .and_then(|store| store.remote_content_revisions())
            .map_err(|error| error.to_string())?;
        let entries =
            Store::prepare_remote_entries_with_revisions(page.entries, kind, &known_revisions)
                .map_err(|error| error.to_string())?;
        let prepared_count = entries.len();
        let ids = feed_ids.clone();
        let persist_started = Instant::now();
        let saved = database
            .write(sync_workspace.to_owned(), move |store| {
                let saved = store.save_prepared_remote_entries(&entries, &ids)?;
                store.update_provider_sync_cursor(
                    cursor_for_write.as_ref(),
                    full_at_for_write.as_deref(),
                )?;
                Ok(saved)
            })
            .map_err(|e| e.to_string())?;
        total += saved;
        sync_log(
            path,
            detailed_logging,
            LogDetail::Detailed,
            "INFO",
            format!(
                "provider page={page_number} prepared={prepared_count} saved={saved} persist_ms={}",
                persist_started.elapsed().as_millis()
            ),
        );
        if !page.has_more {
            break;
        }
        if next_cursor.is_none() {
            return Err("Provider returned another sync page without a continuation cursor".into());
        }
        cursor = next_cursor;
    }
    if reset {
        let staging_workspace = staging_workspace.to_owned();
        database
            .write(workspace.to_owned(), move |store| {
                store.promote_provider_workspace(kind, &staging_workspace)
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(total)
}

async fn flush_remote_marks(
    path: &Path,
    workspace: &str,
    database: &DbWriter,
    remote: &ProviderClient,
) -> Result<(), String> {
    let pending: Vec<PendingRemoteMark> = Store::open_read_workspace(path, workspace)
        .and_then(|store| store.pending_remote_marks())
        .map_err(|error| error.to_string())?;
    let mut read = Vec::new();
    let mut unread = Vec::new();
    let mut starred = Vec::new();
    for mark in &pending {
        match mark.field.as_str() {
            "is_read" if mark.value => read.push(mark.remote_id),
            "is_read" => unread.push(mark.remote_id),
            "is_starred" => starred.push((mark.remote_id, mark.value)),
            _ => {}
        }
    }
    remote
        .mark_entries_status(&read, true)
        .await
        .map_err(|e| e.to_string())?;
    remote
        .mark_entries_status(&unread, false)
        .await
        .map_err(|e| e.to_string())?;
    for (id, value) in starred {
        remote
            .set_starred(id, value)
            .await
            .map_err(|e| e.to_string())?;
    }
    database
        .write(workspace.to_owned(), move |store| {
            store.acknowledge_remote_marks(&pending).map(|_| ())
        })
        .map_err(|e| e.to_string())
}
