use super::articles::cache::RenderCache;
use super::command::ConnectOutcome;
use super::worker::{WorkerState, job, read_lock, write_lock};
use panda_providers::{ProviderClient, ProviderKind, ProviderSettings, save_settings};
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
    let database = state.database.clone();
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
        let mut all = read_lock(&settings_map, "provider settings").clone();
        all.insert(kind, settings);
        save_settings(&settings_path, &all).map_err(|e| e.to_string())?;
        *write_lock(&settings_map, "provider settings") = all;
        *RenderCache::lock(&cache) = RenderCache::default();
        let initial_sync_error =
            super::sync::sync_provider(&path, &workspace, &database, &remote, kind, runtime).err();
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
        let mut all = read_lock(&state.provider_settings, "provider settings").clone();
        all.remove(&kind);
        state
            .save_provider_settings(&all)
            .map_err(|e| e.to_string())?;
        *write_lock(&state.provider_settings, "provider settings") = all;
        Ok(())
    })();
    let _ = reply.send(result);
}
