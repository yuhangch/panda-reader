use super::worker::{WorkerState, job};
use panda_providers::ProviderClient;
use panda_store::Store;
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
    job(reply, move |runtime| {
        let mut store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        if let (Some(kind), Some(config)) = (provider, config) {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime
                .block_on(remote.add_feed(&url))
                .map_err(|e| e.to_string())?;
            runtime
                .block_on(store.sync_provider(&remote, kind))
                .map_err(|e| e.to_string())?;
        } else if provider.is_some() {
            return Err("Connect the selected provider before adding feeds".into());
        } else {
            runtime
                .block_on(store.add_feed(&url))
                .map_err(|e| e.to_string())?;
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
    job(reply, move |runtime| {
        let mut store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let (remote_id, _, source) = store.feed_removal_info(id).map_err(|e| e.to_string())?;
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
        store.remove_feed(id).map_err(|e| e.to_string())
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
    let path = state.path.clone();
    let workspace = state.workspace();
    job(reply, move |_| {
        if workspace != "local" {
            return Err("Feed editing is currently available in local mode only".into());
        }
        let store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        store
            .update_feed(
                id,
                &title,
                folder.as_deref(),
                &feed_url,
                auto_translate_titles,
            )
            .map_err(|e| e.to_string())
    });
}

pub(super) fn set_feed_auto_translate_titles(
    id: i64,
    enabled: bool,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = state.workspace();
    job(reply, move |_| {
        Store::open_workspace(&path, &workspace)
            .map_err(|e| e.to_string())?
            .set_feed_auto_translate_titles(id, enabled)
            .map_err(|e| e.to_string())
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
    job(reply, move |runtime| {
        let mut store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        if let (Some(kind), Some(config)) = (provider, config) {
            let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
            runtime
                .block_on(remote.import_opml(&content))
                .map_err(|e| e.to_string())?;
            runtime
                .block_on(store.sync_provider(&remote, kind))
                .map_err(|e| e.to_string())
        } else if provider.is_some() {
            Err("Connect the selected provider before importing subscriptions".into())
        } else {
            runtime
                .block_on(store.import_opml(&content))
                .map_err(|e| e.to_string())
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
            Store::open_workspace(&path, &workspace)
                .map_err(|e| e.to_string())?
                .export_opml()
                .map_err(|e| e.to_string())
        }
    });
}
