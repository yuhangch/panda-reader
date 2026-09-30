mod article;
mod command;
mod dispatch;
mod favicon;
mod job;

pub use command::Command;

use crate::reader_body::BodyCache;
use dispatch::WorkerState;
use job::{job, sync};
use panda_providers::ProviderSettingsMap;
use panda_store::Store;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    thread,
};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub struct Backend {
    sender: mpsc::UnboundedSender<Command>,
    workspace: Arc<RwLock<String>>,
}

impl Backend {
    pub fn start(
        path: PathBuf,
        provider_settings_path: PathBuf,
        translator_path: PathBuf,
        active_workspace: &str,
        initial_settings: ProviderSettingsMap,
    ) -> Self {
        let settings = Arc::new(RwLock::new(initial_settings));
        let workspace = Arc::new(RwLock::new(active_workspace.to_owned()));
        let worker_workspace = workspace.clone();
        let body_cache = Arc::new(Mutex::new(BodyCache::default()));
        let favicon_inflight = Arc::new(Mutex::new(HashMap::<String, ()>::new()));
        let (sender, mut receiver) = mpsc::unbounded_channel::<Command>();
        thread::Builder::new()
            .name("panda-reader-backend".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to start backend runtime");
                runtime.block_on(async move {
                    // Opening once performs any required SQLite schema migration.
                    match Store::open(&path) {
                        Ok(store) => drop(store),
                        Err(error) => {
                            eprintln!("could not open Panda Reader database: {error}");
                            return;
                        }
                    };
                    let state = WorkerState {
                        path: path.clone(),
                        translator_path,
                        provider_settings_path,
                        provider_settings: settings.clone(),
                        active_workspace: worker_workspace,
                        body_cache,
                        favicon_inflight,
                    };
                    let first = tokio::time::Instant::now();
                    let mut timer =
                        tokio::time::interval_at(first, std::time::Duration::from_secs(900));
                    loop {
                        tokio::select! {
                            _ = timer.tick() => {
                                let path = state.path.clone();
                                let current_workspace = state.workspace();
                                let provider = state.provider_kind();
                                let config = provider.and_then(|kind| state.provider_settings(kind));
                                let (reply, _) = oneshot::channel();
                                job(reply, move |runtime| sync(&path, &current_workspace, provider, config, runtime));
                            }
                            command = receiver.recv() => {
                                let Some(command) = command else { break; };
                                dispatch::handle(command, &state);
                            }
                        }
                    }
                });
            })
            .expect("failed to start Panda Reader backend thread");
        Self { sender, workspace }
    }

    pub fn send(&self, command: Command) {
        let _ = self.sender.send(command);
    }

    pub fn set_workspace(&self, workspace: &str) {
        if let Ok(mut active) = self.workspace.write() {
            *active = workspace.to_owned();
        }
    }
}
