use super::articles::cache::BodyCache;
use super::command::ConnectOutcome;
use super::worker::{WorkerState, job};
use panda_providers::{ProviderClient, ProviderKind, ProviderSettings, save_settings};
use panda_store::Store;
use tokio::sync::oneshot;

pub(super) fn connect(
    kind: ProviderKind,
    endpoint: String,
    username: String,
    secret: String,
    reply: oneshot::Sender<Result<ConnectOutcome, String>>,
    state: &WorkerState,
) {
    let path = state.path.clone();
    let workspace = match kind {
        ProviderKind::Miniflux => "provider:miniflux",
        ProviderKind::FreshRss => "provider:freshrss",
    }
    .to_owned();
    let settings_path = state.provider_settings_path.clone();
    let settings_map = state.provider_settings.clone();
    let cache = state.body_cache.clone();
    job(reply, move |runtime| {
        let settings = ProviderSettings {
            endpoint,
            username,
            secret,
        };
        let remote = ProviderClient::new(kind, &settings).map_err(|e| e.to_string())?;
        let identity = runtime
            .block_on(remote.identity())
            .map_err(|e| e.to_string())?;
        let mut all = settings_map.read().map_err(|e| e.to_string())?.clone();
        all.insert(kind, settings);
        save_settings(&settings_path, &all).map_err(|e| e.to_string())?;
        *settings_map.write().map_err(|e| e.to_string())? = all;
        if let Ok(mut cache) = cache.lock() {
            *cache = BodyCache::default();
        }
        let mut store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
        let initial_sync_error = runtime
            .block_on(store.sync_provider(&remote, kind))
            .err()
            .map(|error| error.to_string());
        Ok(ConnectOutcome {
            account_name: identity.name,
            initial_sync_error,
        })
    });
}

pub(super) fn disconnect(
    kind: ProviderKind,
    reply: oneshot::Sender<Result<(), String>>,
    state: &WorkerState,
) {
    let result = (|| {
        let mut all = state
            .provider_settings
            .read()
            .map_err(|e| e.to_string())?
            .clone();
        all.remove(&kind);
        state
            .save_provider_settings(&all)
            .map_err(|e| e.to_string())?;
        *state.provider_settings.write().map_err(|e| e.to_string())? = all;
        Ok(())
    })();
    let _ = reply.send(result);
}
