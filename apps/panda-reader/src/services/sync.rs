use super::articles::cache::BodyCache;
use super::worker::{WorkerState, job};
use panda_providers::{ProviderClient, ProviderKind, ProviderSettings};
use panda_store::Store;
use std::path::PathBuf;
use tokio::sync::oneshot;

pub fn sync(
    path: &PathBuf,
    workspace: &str,
    kind: Option<ProviderKind>,
    config: Option<ProviderSettings>,
    runtime: &tokio::runtime::Runtime,
) -> Result<usize, String> {
    let mut store = Store::open_workspace(path, workspace).map_err(|error| error.to_string())?;
    if let (Some(kind), Some(config)) = (kind, config) {
        let remote = ProviderClient::new(kind, &config).map_err(|error| error.to_string())?;
        runtime
            .block_on(async {
                remote.refresh().await?;
                store.sync_provider(&remote, kind).await
            })
            .map_err(|error| error.to_string())
    } else if kind.is_some() {
        Err("Connect the selected provider before syncing".into())
    } else {
        runtime
            .block_on(store.refresh_all())
            .map_err(|error| error.to_string())
    }
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
    job(reply, move |runtime| {
        let mut store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        if let (Some(kind), Some(config)) = (provider, config) {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime
                .block_on(remote.refresh())
                .map_err(|e| e.to_string())?;
            runtime
                .block_on(store.sync_provider(&remote, kind))
                .map_err(|e| e.to_string())
        } else if provider.is_some() {
            Err("Connect the selected provider before syncing".into())
        } else {
            runtime
                .block_on(store.refresh_feed(id))
                .map_err(|e| e.to_string())
        }
    });
}

pub(super) fn refresh(reply: oneshot::Sender<Result<usize, String>>, state: &WorkerState) {
    let path = state.path.clone();
    let workspace = state.workspace();
    let provider = state.provider_kind();
    let config = provider.and_then(|kind| state.provider_settings(kind));
    let cache = state.body_cache.clone();
    job(reply, move |runtime| {
        let result = sync(&path, &workspace, provider, config, runtime);
        if result.is_ok() {
            if let Ok(mut cache) = cache.lock() {
                *cache = BodyCache::default();
            }
        }
        result
    });
}
