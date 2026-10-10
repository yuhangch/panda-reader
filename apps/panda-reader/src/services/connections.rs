use super::articles::cache::RenderCache;
use super::command::ConnectOutcome;
use super::diagnostics::{LogDetail, write_sync_log};
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
    let settings_path = state.provider_settings_path.clone();
    let settings_map = state.provider_settings.clone();
    let cache = state.body_cache.clone();
    let log_path = state.path.clone();
    let detailed_logging = state.detailed_sync_logging.clone();
    job(reply, move |runtime| {
        let started = std::time::Instant::now();
        let settings = ProviderSettings {
            endpoint,
            username,
            secret,
        };
        let remote = match ProviderClient::new(kind, &settings) {
            Ok(remote) => remote,
            Err(error) => {
                write_sync_log(
                    &log_path,
                    &detailed_logging,
                    LogDetail::Detailed,
                    "ERROR",
                    format!(
                        "provider={} connection setup failed elapsed_ms={} error={error:#}",
                        kind.key(),
                        started.elapsed().as_millis()
                    ),
                );
                return Err(error.to_string());
            }
        };
        let identity = match runtime.block_on(remote.identity()) {
            Ok(identity) => identity,
            Err(error) => {
                write_sync_log(
                    &log_path,
                    &detailed_logging,
                    LogDetail::Detailed,
                    "ERROR",
                    format!(
                        "provider={} connection validation failed elapsed_ms={} error={error:#}",
                        kind.key(),
                        started.elapsed().as_millis()
                    ),
                );
                return Err(error.to_string());
            }
        };
        let mut all = read_lock(&settings_map, "provider settings").clone();
        all.insert(kind, settings);
        save_settings(&settings_path, &all).map_err(|e| e.to_string())?;
        *write_lock(&settings_map, "provider settings") = all;
        *RenderCache::lock(&cache) = RenderCache::default();
        write_sync_log(
            &log_path,
            &detailed_logging,
            LogDetail::Detailed,
            "INFO",
            format!(
                "provider={} connection validation succeeded elapsed_ms={}",
                kind.key(),
                started.elapsed().as_millis()
            ),
        );
        Ok(ConnectOutcome {
            account_name: identity.name,
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
