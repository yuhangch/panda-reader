use panda_providers::{ProviderClient, ProviderKind, ProviderSettings};
use panda_store::Store;
use std::{path::PathBuf, thread};
use tokio::sync::oneshot;

pub fn job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) {
    thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
            .and_then(|runtime| run(&runtime));
        let _ = reply.send(result);
    });
}

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
