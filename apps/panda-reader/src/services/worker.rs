//! Worker lifecycle and background task execution.

use panda_providers::{ProviderKind, ProviderSettings, save_settings};

use super::articles::cache::BodyCache;
use super::{Command, dispatch};

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
pub struct AppServices {
    sender: mpsc::UnboundedSender<Command>,
    workspace: Arc<RwLock<String>>,
}

impl AppServices {
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
            .name("panda-reader-services".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to start services runtime");
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
                        title_translation_lock: Arc::new(Mutex::new(())),
                        title_translation_attempted: Arc::new(Mutex::new(HashMap::new())),
                    };
                    loop {
                        let Some(command) = receiver.recv().await else {
                            break;
                        };
                        dispatch::handle(command, &state);
                    }
                });
            })
            .expect("failed to start Panda Reader services thread");
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

pub struct WorkerState {
    pub path: PathBuf,
    pub provider_settings_path: PathBuf,
    pub translator_path: PathBuf,
    pub provider_settings: Arc<RwLock<ProviderSettingsMap>>,
    pub active_workspace: Arc<RwLock<String>>,
    pub body_cache: Arc<Mutex<BodyCache>>,
    pub favicon_inflight: Arc<Mutex<HashMap<String, ()>>>,
    pub title_translation_lock: Arc<Mutex<()>>,
    pub title_translation_attempted: Arc<Mutex<HashMap<String, std::time::Instant>>>,
}

impl WorkerState {
    pub fn workspace(&self) -> String {
        self.active_workspace
            .read()
            .map(|v| v.clone())
            .unwrap_or_else(|_| "local".into())
    }

    pub fn provider_kind(&self) -> Option<ProviderKind> {
        match self.workspace().as_str() {
            "provider:miniflux" => Some(ProviderKind::Miniflux),
            "provider:freshrss" => Some(ProviderKind::FreshRss),
            _ => None,
        }
    }

    pub fn provider_settings(&self, kind: ProviderKind) -> Option<ProviderSettings> {
        self.provider_settings.read().ok()?.get(&kind).cloned()
    }

    pub(super) fn save_provider_settings(
        &self,
        values: &ProviderSettingsMap,
    ) -> anyhow::Result<()> {
        save_settings(&self.provider_settings_path, values)
    }
}

pub(super) fn job<T: Send + 'static>(
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
