use super::worker::{WorkerState, job};
use panda_providers::ProviderClient;
use panda_store::Store;
use std::sync::Arc;
use tokio::sync::oneshot;

pub(super) fn add_feed(
    url: String,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let provider = state.provider_kind();
    let config = provider.and_then(|kind| state.provider_settings(kind));
    let database = state.database.clone();
    let detailed_logging = state.detailed_sync_logging.clone();
    job(reply, move |runtime| {
        if let (Some(kind), Some(config)) = (provider, config) {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime
                .block_on(remote.add_feed(&url))
                .map_err(|e| e.to_string())?;
            super::sync::sync_provider(
                &path,
                &workspace,
                &database,
                &remote,
                kind,
                &detailed_logging,
                runtime,
            )?;
        } else if provider.is_some() {
            return Err("Connect the selected provider before adding feeds".into());
        } else {
            let normalized = panda_store::normalize_http_url(&url).map_err(|e| e.to_string())?;
            let read = Store::open_read_workspace(&path, &workspace).map_err(|e| e.to_string())?;
            if !read.has_feed_url(&normalized).map_err(|e| e.to_string())? {
                let fetched = runtime
                    .block_on(Store::fetch_feed_data(&normalized, None, None))
                    .map_err(|e| e.to_string())?;
                let prepared =
                    Store::prepare_fetched_feed(&normalized, fetched).map_err(|e| e.to_string())?;
                database.write(workspace, move |store| {
                    store
                        .persist_prepared_feed(&normalized, None, prepared)
                        .map(|_| ())
                })?;
            }
        }
        Ok(())
    });
}

pub(super) fn remove_feed(
    id: i64,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let provider = state.provider_kind();
    let config = provider.and_then(|kind| state.provider_settings(kind));
    let database = state.database.clone();
    job(reply, move |runtime| {
        let read = Store::open_read_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let (remote_id, _, source) = read.feed_removal_info(id).map_err(|e| e.to_string())?;
        if let (Some(kind), Some(config)) = (provider, config) {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            let remote_id = remote_id.ok_or("Provider feed has no remote ID")?;
            runtime
                .block_on(remote.remove_feed(remote_id))
                .map_err(|e| e.to_string())?;
        } else if provider.is_some() {
            return Err("Connect the selected provider before removing feeds".into());
        } else if source != "local" {
            return Err("Connect the matching provider before removing this feed".into());
        }
        database.write(workspace, move |store| {
            store.remove_feed(id).map_err(Into::into)
        })
    });
}

pub(super) fn update_feed(
    id: i64,
    title: String,
    folder: Option<String>,
    feed_url: String,
    auto_translate_titles: bool,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let workspace = state.workspace();
    let database = state.database.clone();
    job(reply, move |_| {
        if workspace != "local" {
            return Err("Feed editing is currently available in local mode only".into());
        }
        database.write(workspace, move |store| {
            store.update_feed(
                id,
                &title,
                folder.as_deref(),
                &feed_url,
                auto_translate_titles,
            )
        })
    });
}

pub(super) fn set_feed_auto_translate_titles(
    id: i64,
    enabled: bool,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let workspace = state.workspace();
    let database = state.database.clone();
    job(reply, move |_| {
        database.write(workspace, move |store| {
            store.set_feed_auto_translate_titles(id, enabled)
        })
    });
}

pub(super) fn reset_title_translation_cutoffs(
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let workspace = state.workspace();
    let database = state.database.clone();
    job(reply, move |_| {
        database.write(workspace, |store| {
            store.reset_auto_title_translation_cutoffs()
        })
    });
}

pub(super) fn import_opml(
    content: String,
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
        if let (Some(kind), Some(config)) = (provider, config) {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime
                .block_on(remote.import_opml(&content))
                .map_err(|e| e.to_string())?;
            super::sync::sync_provider(
                &path,
                &workspace,
                &database,
                &remote,
                kind,
                &detailed_logging,
                runtime,
            )
        } else if provider.is_some() {
            Err("Connect the selected provider before importing subscriptions".into())
        } else {
            let entries = Store::parse_opml_feeds(&content).map_err(|e| e.to_string())?;
            let mut candidates = Vec::new();
            for (url, title, folder) in entries {
                let Ok(url) = panda_store::normalize_http_url(&url) else {
                    continue;
                };
                let exists = Store::open_read_workspace(&path, &workspace)
                    .and_then(|store| store.has_feed_url(&url))
                    .map_err(|e| e.to_string())?;
                if exists {
                    continue;
                }
                candidates.push((url, title, folder));
            }
            let fetched = runtime.block_on(async {
                let semaphore = Arc::new(tokio::sync::Semaphore::new(8));
                let mut tasks = tokio::task::JoinSet::new();
                for (url, title, folder) in candidates {
                    let semaphore = semaphore.clone();
                    tasks.spawn(async move {
                        let permit = semaphore
                            .acquire_owned()
                            .await
                            .map_err(|error| error.to_string())?;
                        let result = Store::fetch_feed_data(&url, None, None).await;
                        drop(permit);
                        Ok::<_, String>((url, title, folder, result))
                    });
                }
                let mut output = Vec::new();
                while let Some(result) = tasks.join_next().await {
                    output.push(
                        result.map_err(|error| format!("OPML import task failed: {error}"))??,
                    );
                }
                Ok::<_, String>(output)
            })?;
            let mut added = 0;
            for (url, title, folder, result) in fetched {
                match result {
                    Ok(fetched) if !fetched.is_not_modified() => {
                        let prepared = Store::prepare_fetched_feed(&url, fetched)
                            .map_err(|error| error.to_string())?;
                        let candidate_url = url.clone();
                        if database.write(workspace.clone(), move |store| {
                            store.persist_imported_feed_response(
                                &candidate_url,
                                prepared,
                                title,
                                folder,
                            )
                        })? {
                            added += 1;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("skipping OPML feed {url}: {error}"),
                }
            }
            Ok(added)
        }
    });
}

pub(super) fn export_opml(reply: oneshot::Sender<Result<String, String>>, state: &WorkerState) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let provider = state.provider_kind();
    let config = provider.and_then(|kind| state.provider_settings(kind));
    job(reply, move |runtime| {
        if let (Some(kind), Some(config)) = (provider, config) {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime
                .block_on(remote.export_opml())
                .map_err(|e| e.to_string())
        } else if provider.is_some() {
            Err("Connect the selected provider before exporting subscriptions".into())
        } else {
            Store::open_read_workspace(&path, &workspace)
                .map_err(|e| e.to_string())?
                .export_opml()
                .map_err(|e| e.to_string())
        }
    });
}
