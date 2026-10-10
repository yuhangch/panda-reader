//! Worker lifecycle and background task execution.

use panda_providers::{ProviderKind, ProviderSettings, save_settings};

use super::articles::cache::RenderCache;
use super::database::DbWriter;
use super::diagnostics;
use super::{Command, dispatch};

use anyhow::{Context as _, bail};
use panda_plugins::{
    CommunityCatalog, CommunityPlugin, PluginRegistry, PluginSettings, PluginSummary,
};
use panda_providers::ProviderSettingsMap;
use std::{
    any::Any,
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard,
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc,
        mpsc::{SyncSender, TrySendError},
    },
    thread,
};
use tokio::sync::{mpsc, oneshot};

const MAX_PLUGIN_DOWNLOAD_BYTES: usize = 8 * 1024 * 1024;
const COMMUNITY_PLUGIN_CATALOG_URL: &str = "https://raw.githubusercontent.com/yuhangch/panda-reader/refs/heads/main/plugins/community/index.json";
const MAX_PLUGIN_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const COMMAND_QUEUE_CAPACITY: usize = 512;

#[derive(Clone)]
pub struct AppServices {
    sender: mpsc::Sender<Command>,
    workspace: Arc<RwLock<String>>,
    detailed_sync_logging: Arc<AtomicBool>,
    database_path: PathBuf,
    pub(super) article_work_cancellations: ArticleWorkCancellations,
}

type ArticleWorkKey = (String, i64);
type ArticleWorkCancellations =
    Arc<Mutex<HashMap<ArticleWorkKey, Vec<(u64, tokio::sync::watch::Sender<bool>)>>>>;

pub(super) fn register_article_work(
    cancellations: &ArticleWorkCancellations,
    workspace: &str,
    article_id: i64,
) -> (u64, tokio::sync::watch::Receiver<bool>) {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = tokio::sync::watch::channel(false);
    lock_mutex(cancellations, "article task cancellation")
        .entry((workspace.to_owned(), article_id))
        .or_default()
        .push((id, sender));
    (id, receiver)
}

pub(super) fn finish_article_work(
    cancellations: &ArticleWorkCancellations,
    workspace: &str,
    article_id: i64,
    id: u64,
) {
    let mut cancellations = lock_mutex(cancellations, "article task cancellation");
    let key = (workspace.to_owned(), article_id);
    if let Some(tasks) = cancellations.get_mut(&key) {
        tasks.retain(|(task_id, _)| *task_id != id);
        if tasks.is_empty() {
            cancellations.remove(&key);
        }
    }
}

fn cancel_article_work(cancellations: &ArticleWorkCancellations, workspace: &str, article_id: i64) {
    if let Some(tasks) = lock_mutex(cancellations, "article task cancellation")
        .get(&(workspace.to_owned(), article_id))
    {
        for (_, sender) in tasks {
            sender.send_replace(true);
        }
    }
}

pub(super) async fn wait_for_article_work_cancel(mut receiver: tokio::sync::watch::Receiver<bool>) {
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
}

impl AppServices {
    pub fn start(
        path: PathBuf,
        provider_settings_path: PathBuf,
        translator_path: PathBuf,
        plugin_dir: PathBuf,
        active_workspace: &str,
        initial_settings: ProviderSettingsMap,
        detailed_sync_logging: bool,
        log_retention_days: u16,
    ) -> Self {
        diagnostics::set_log_retention_days(log_retention_days, &path);
        let database_path = path.clone();
        let settings = Arc::new(RwLock::new(initial_settings));
        let workspace = Arc::new(RwLock::new(active_workspace.to_owned()));
        let worker_workspace = workspace.clone();
        let article_work_cancellations = Arc::new(Mutex::new(HashMap::new()));
        let worker_article_work_cancellations = article_work_cancellations.clone();
        let detailed_sync_logging = Arc::new(AtomicBool::new(detailed_sync_logging));
        let worker_detailed_sync_logging = detailed_sync_logging.clone();
        let body_cache = Arc::new(Mutex::new(RenderCache::default()));
        let (sender, mut receiver) = mpsc::channel::<Command>(COMMAND_QUEUE_CAPACITY);
        thread::Builder::new()
            .name("panda-reader-services".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to start services runtime");
                runtime.block_on(async move {
                    let writer = match DbWriter::start(path.clone()) {
                        Ok(writer) => writer,
                        Err(error) => {
                            eprintln!("could not start Panda Reader database writer: {error:#}");
                            return;
                        }
                    };
                    let index_writer = writer.clone();
                    thread::Builder::new()
                        .name("panda-reader-search-index".into())
                        .spawn(move || {
                            // Keep startup and synchronization responsive while an older library is indexed.
                            thread::sleep(std::time::Duration::from_secs(2));
                            loop {
                                let indexed = index_writer
                                    .write_low_priority("local".into(), |store| {
                                        store.index_search_batch(50)
                                    });
                                match indexed {
                                    Ok(true) => break,
                                    Ok(false) => {
                                        thread::sleep(std::time::Duration::from_millis(50))
                                    }
                                    Err(error) => {
                                        eprintln!(
                                            "could not build article search index: {error:#}"
                                        );
                                        break;
                                    }
                                }
                            }
                        })
                        .expect("failed to start article search index worker");
                    let state = WorkerState {
                        path: path.clone(),
                        translator_path,
                        database: writer,
                        plugin_dir: plugin_dir.clone(),
                        plugin_registry: Arc::new(RwLock::new(load_plugin_registry(&plugin_dir))),
                        provider_settings_path,
                        provider_settings: settings.clone(),
                        active_workspace: worker_workspace,
                        detailed_sync_logging: worker_detailed_sync_logging,
                        body_cache,
                        title_translation_lock: Arc::new(Mutex::new(())),
                        title_translation_attempted: Arc::new(Mutex::new(HashMap::new())),
                        article_work_cancellations: worker_article_work_cancellations,
                    };
                    loop {
                        let Some(command) = receiver.recv().await else {
                            break;
                        };
                        if let Err(panic) =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                dispatch::handle(command, &state)
                            }))
                        {
                            eprintln!(
                                "service command dispatcher recovered from panic: {}",
                                panic_message(&*panic)
                            );
                        }
                    }
                });
            })
            .expect("failed to start Panda Reader services thread");
        Self {
            sender,
            workspace,
            detailed_sync_logging,
            database_path,
            article_work_cancellations,
        }
    }

    pub fn send(&self, command: Command) {
        if let Err(error) = self.sender.try_send(command) {
            let (command, message) = match error {
                mpsc::error::TrySendError::Full(command) => (
                    command,
                    "Panda Reader is busy; the command queue is full".to_owned(),
                ),
                mpsc::error::TrySendError::Closed(command) => (
                    command,
                    "Panda Reader background services are unavailable".to_owned(),
                ),
            };
            command.reject(message);
        }
    }

    pub fn cancel_article_work(&self, article_id: i64) {
        let workspace = read_lock(&self.workspace, "active workspace").clone();
        cancel_article_work(&self.article_work_cancellations, &workspace, article_id);
    }

    pub fn set_workspace(&self, workspace: &str) {
        *write_lock(&self.workspace, "active workspace") = workspace.to_owned();
    }

    pub fn set_detailed_sync_logging(&self, enabled: bool) {
        self.detailed_sync_logging.store(enabled, Ordering::Relaxed);
    }

    pub fn set_log_retention_days(&self, days: u16) {
        diagnostics::set_log_retention_days(days, &self.database_path);
    }
}

pub struct WorkerState {
    pub path: PathBuf,
    pub provider_settings_path: PathBuf,
    pub translator_path: PathBuf,
    pub database: DbWriter,
    pub plugin_dir: PathBuf,
    pub plugin_registry: Arc<RwLock<PluginRegistry>>,
    pub provider_settings: Arc<RwLock<ProviderSettingsMap>>,
    pub active_workspace: Arc<RwLock<String>>,
    pub detailed_sync_logging: Arc<AtomicBool>,
    pub body_cache: Arc<Mutex<RenderCache>>,
    pub title_translation_lock: Arc<Mutex<()>>,
    pub title_translation_attempted: Arc<Mutex<HashMap<String, std::time::Instant>>>,
    pub(super) article_work_cancellations: ArticleWorkCancellations,
}

pub(super) fn remote_mark_lock(workspace: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    lock_mutex(locks, "remote mark serialization")
        .entry(workspace.to_owned())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

fn load_plugin_registry(plugin_dir: &std::path::Path) -> PluginRegistry {
    let settings_path = plugin_dir.join("settings.json");
    let settings = std::fs::read(&settings_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PluginSettings>(&bytes).ok())
        .unwrap_or_default();
    PluginRegistry::load(plugin_dir, &settings, 0)
}

impl WorkerState {
    pub fn workspace(&self) -> String {
        read_lock(&self.active_workspace, "active workspace").clone()
    }

    pub fn provider_kind(&self) -> Option<ProviderKind> {
        match self.workspace().as_str() {
            "provider:miniflux" => Some(ProviderKind::Miniflux),
            "provider:freshrss" => Some(ProviderKind::FreshRss),
            _ => None,
        }
    }

    pub fn provider_settings(&self, kind: ProviderKind) -> Option<ProviderSettings> {
        read_lock(&self.provider_settings, "provider settings")
            .get(&kind)
            .cloned()
    }

    pub(super) fn save_provider_settings(
        &self,
        values: &ProviderSettingsMap,
    ) -> anyhow::Result<()> {
        save_settings(&self.provider_settings_path, values)
    }

    fn plugin_settings(&self) -> anyhow::Result<PluginSettings> {
        let path = self.plugin_dir.join("settings.json");
        Ok(std::fs::read(path)
            .ok()
            .map(|bytes| serde_json::from_slice::<PluginSettings>(&bytes))
            .transpose()?
            .unwrap_or_default())
    }

    fn save_plugin_settings(&self, settings: &PluginSettings) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.plugin_dir)?;
        let path = self.plugin_dir.join("settings.json");
        let bytes = serde_json::to_vec_pretty(settings)?;
        let temp = self.plugin_dir.join("settings.json.tmp");
        std::fs::write(&temp, bytes)?;
        std::fs::rename(temp, path)?;
        Ok(())
    }

    pub(super) fn plugin_list(&self) -> anyhow::Result<Vec<PluginSummary>> {
        Ok(read_lock(&self.plugin_registry, "plugin registry").plugins())
    }

    pub(super) fn reload_plugins(&self) -> anyhow::Result<Vec<PluginSummary>> {
        let generation = read_lock(&self.plugin_registry, "plugin registry")
            .generation()
            .wrapping_add(1);
        let settings = self.plugin_settings()?;
        let next = PluginRegistry::load(&self.plugin_dir, &settings, generation);
        let plugins = next.plugins();
        *write_lock(&self.plugin_registry, "plugin registry") = next;
        RenderCache::lock(&self.body_cache).clear();
        Ok(plugins)
    }

    pub(super) fn set_plugin_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> anyhow::Result<Vec<PluginSummary>> {
        let mut settings = self.plugin_settings()?;
        settings.set_enabled(id, enabled);
        self.save_plugin_settings(&settings)?;
        self.reload_plugins()
    }

    pub(super) fn remove_plugin(&self, id: &str) -> anyhow::Result<Vec<PluginSummary>> {
        PluginRegistry::remove_user_plugin(&self.plugin_dir, id)?;
        let mut settings = self.plugin_settings()?;
        settings.remove(id);
        self.save_plugin_settings(&settings)?;
        self.reload_plugins()
    }
}

pub(super) fn import_plugin(
    source: String,
    reply: oneshot::Sender<Result<Vec<PluginSummary>, String>>,
    state: &WorkerState,
) {
    let plugin_dir = state.plugin_dir.clone();
    let registry = state.plugin_registry.clone();
    let body_cache = state.body_cache.clone();
    job(reply, move |runtime| {
        (|| -> anyhow::Result<Vec<PluginSummary>> {
            let source = source.trim();
            if source.is_empty() {
                bail!("enter a plugin folder path, ZIP path, or HTTPS ZIP URL");
            }
            if source
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
                || source
                    .get(..7)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
            {
                let bytes = runtime.block_on(download_plugin_archive(source))?;
                PluginRegistry::install_archive(&bytes, &plugin_dir)?;
            } else {
                let path = PathBuf::from(source);
                if path.is_dir() {
                    PluginRegistry::install_directory(&path, &plugin_dir)?;
                } else if path.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
                {
                    let metadata = std::fs::metadata(&path)?;
                    if metadata.len() > MAX_PLUGIN_DOWNLOAD_BYTES as u64 {
                        bail!("plugin archive exceeds the 8 MiB size limit");
                    }
                    let bytes = std::fs::read(&path)?;
                    PluginRegistry::install_archive(&bytes, &plugin_dir)?;
                } else {
                    bail!("select a plugin folder, a .zip archive, or an HTTPS .zip URL");
                }
            }

            let settings_path = plugin_dir.join("settings.json");
            let settings = std::fs::read(&settings_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PluginSettings>(&bytes).ok())
                .unwrap_or_default();
            let generation = read_lock(&registry, "plugin registry")
                .generation()
                .wrapping_add(1);
            let next = PluginRegistry::load(&plugin_dir, &settings, generation);
            let plugins = next.plugins();
            *write_lock(&registry, "plugin registry") = next;
            RenderCache::lock(&body_cache).clear();
            Ok(plugins)
        })()
        .map_err(|error| format!("{error:#}"))
    });
}

pub(super) fn community_plugin_catalog(
    reply: oneshot::Sender<Result<Vec<CommunityPlugin>, String>>,
) {
    job(reply, move |runtime| {
        load_community_catalog(runtime)
            .map(|catalog| catalog.plugins)
            .map_err(|error| format!("{error:#}"))
    });
}

pub(super) fn install_community_plugin(
    id: String,
    reply: oneshot::Sender<Result<Vec<PluginSummary>, String>>,
    state: &WorkerState,
) {
    let plugin_dir = state.plugin_dir.clone();
    let registry = state.plugin_registry.clone();
    let body_cache = state.body_cache.clone();
    job(reply, move |runtime| {
        (|| -> anyhow::Result<Vec<PluginSummary>> {
            let catalog = load_community_catalog(runtime)?;
            let plugin = catalog
                .plugins
                .iter()
                .find(|plugin| plugin.id == id)
                .with_context(|| format!("community plugin {id} is not in the catalog"))?;
            let app_version = semver::Version::parse(env!("CARGO_PKG_VERSION"))?;
            if semver::Version::parse(&plugin.min_app_version)? > app_version {
                bail!(
                    "{} requires Panda Reader {}, but this app is {}",
                    plugin.name,
                    plugin.min_app_version,
                    app_version
                );
            }
            if plugin.api_version != 1 {
                bail!("{} requires an unsupported plugin API version", plugin.name);
            }
            let manifest_file = plugin.files.get("manifest.toml").unwrap();
            let payload_file = plugin.files.get(plugin.payload_name()).unwrap();
            let manifest_bytes = community_plugin_file(
                runtime,
                plugin,
                "manifest.toml",
                &manifest_file.url,
                MAX_PLUGIN_MANIFEST_BYTES,
            )?;
            let payload_bytes = community_plugin_file(
                runtime,
                plugin,
                plugin.payload_name(),
                &payload_file.url,
                MAX_PLUGIN_DOWNLOAD_BYTES,
            )?;
            PluginRegistry::install_community_plugin(
                plugin,
                &manifest_bytes,
                &payload_bytes,
                &plugin_dir,
            )?;

            let settings_path = plugin_dir.join("settings.json");
            let settings = std::fs::read(&settings_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PluginSettings>(&bytes).ok())
                .unwrap_or_default();
            let generation = read_lock(&registry, "plugin registry")
                .generation()
                .wrapping_add(1);
            let next = PluginRegistry::load(&plugin_dir, &settings, generation);
            let plugins = next.plugins();
            *write_lock(&registry, "plugin registry") = next;
            RenderCache::lock(&body_cache).clear();
            Ok(plugins)
        })()
        .map_err(|error| format!("{error:#}"))
    });
}

fn load_community_catalog(runtime: &tokio::runtime::Runtime) -> anyhow::Result<CommunityCatalog> {
    if cfg!(debug_assertions) {
        let local_catalog =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/community/index.json");
        if local_catalog.is_file() {
            let bytes = std::fs::read(local_catalog).context("read local community catalog")?;
            return CommunityCatalog::parse(&bytes);
        }
    }
    let bytes = runtime.block_on(download_plugin_file(
        COMMUNITY_PLUGIN_CATALOG_URL,
        MAX_PLUGIN_CATALOG_BYTES,
    ))?;
    CommunityCatalog::parse(&bytes)
}

fn community_plugin_file(
    runtime: &tokio::runtime::Runtime,
    plugin: &CommunityPlugin,
    file_name: &str,
    url: &str,
    max_bytes: usize,
) -> anyhow::Result<Vec<u8>> {
    if cfg!(debug_assertions) {
        let local_file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../plugins/community")
            .join(&plugin.id)
            .join(file_name);
        if local_file.is_file() {
            let bytes = std::fs::read(local_file).context("read local community plugin file")?;
            if bytes.len() > max_bytes {
                bail!("community plugin file exceeds its size limit");
            }
            return Ok(bytes);
        }
    }
    runtime.block_on(download_plugin_file(url, max_bytes))
}

async fn download_plugin_file(url: &str, max_bytes: usize) -> anyhow::Result<Vec<u8>> {
    let parsed = url::Url::parse(url).context("invalid community plugin download URL")?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        bail!("community plugin downloads must use HTTPS without credentials");
    }
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5
            || attempt.url().scheme() != "https"
            || !attempt.url().username().is_empty()
            || attempt.url().password().is_some()
        {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    let client = reqwest::Client::builder()
        .redirect(redirect)
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!("PandaReader/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut response = client.get(parsed).send().await?.error_for_status()?;
    if response.url().scheme() != "https" {
        bail!("community plugin URL redirected to a non-HTTPS address");
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        bail!("community plugin download exceeds its size limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            bail!("community plugin download exceeds its size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

const MAX_PLUGIN_MANIFEST_BYTES: usize = 64 * 1024;

async fn download_plugin_archive(source: &str) -> anyhow::Result<Vec<u8>> {
    let url = url::Url::parse(source).context("invalid plugin URL")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("plugin URLs must use HTTPS and cannot include credentials");
    }
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5
            || attempt.url().scheme() != "https"
            || !attempt.url().username().is_empty()
            || attempt.url().password().is_some()
        {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    let client = reqwest::Client::builder()
        .redirect(redirect)
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    if response.url().scheme() != "https" {
        bail!("plugin URL redirected to a non-HTTPS address");
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PLUGIN_DOWNLOAD_BYTES as u64)
    {
        bail!("plugin download exceeds the 8 MiB size limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_PLUGIN_DOWNLOAD_BYTES {
            bail!("plugin download exceeds the 8 MiB size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) fn job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) {
    let _ = try_job(reply, run);
}

pub(super) fn try_job<T: Send + 'static>(
    reply: oneshot::Sender<Result<T, String>>,
    run: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, String> + Send + 'static,
) -> bool {
    type BackgroundJob = Box<dyn FnOnce(&tokio::runtime::Runtime) + Send + 'static>;
    static JOBS: OnceLock<SyncSender<BackgroundJob>> = OnceLock::new();
    let sender = JOBS.get_or_init(|| {
        let (sender, receiver) = std_mpsc::sync_channel::<BackgroundJob>(256);
        let receiver = Arc::new(Mutex::new(receiver));
        for index in 0..4 {
            let receiver = receiver.clone();
            thread::Builder::new()
                .name(format!("panda-reader-worker-{index}"))
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("failed to start background worker runtime");
                    loop {
                        let next = {
                            let receiver = lock_mutex(&receiver, "background job queue");
                            receiver.recv()
                        };
                        let Ok(job) = next else { break };
                        job(&runtime);
                    }
                })
                .expect("failed to start background worker");
        }
        sender
    });
    let reply_slot = Arc::new(Mutex::new(Some(reply)));
    let worker_reply = reply_slot.clone();
    let task: BackgroundJob = Box::new(move |runtime| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(runtime)))
            .unwrap_or_else(|panic| {
                Err(format!(
                    "Background operation panicked: {}",
                    panic_message(&*panic)
                ))
            });
        if let Some(reply) = lock_mutex(&worker_reply, "background job reply").take() {
            let _ = reply.send(result);
        }
    });
    match sender.try_send(task) {
        Ok(()) => true,
        Err(error) => {
            let (task, message) = match error {
                TrySendError::Full(task) => {
                    (task, "Background task queue is full; try again shortly")
                }
                TrySendError::Disconnected(task) => {
                    (task, "Background task workers are unavailable")
                }
            };
            drop(task);
            if let Some(reply) = lock_mutex(&reply_slot, "background job reply").take() {
                let _ = reply.send(Err(message.to_owned()));
            }
            false
        }
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

#[cfg(test)]
mod article_cancellation_tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_an_article_notifies_all_running_tasks_for_that_workspace() {
        let cancellations = Arc::new(Mutex::new(HashMap::new()));
        let (first_id, mut first) = register_article_work(&cancellations, "local", 7);
        let (second_id, mut second) = register_article_work(&cancellations, "local", 7);
        let (_, other_workspace) = register_article_work(&cancellations, "provider:miniflux", 7);

        cancel_article_work(&cancellations, "local", 7);

        assert!(*first.borrow_and_update());
        assert!(*second.borrow_and_update());
        assert!(!*other_workspace.borrow());
        finish_article_work(&cancellations, "local", 7, first_id);
        finish_article_work(&cancellations, "local", 7, second_id);
        assert_eq!(lock_mutex(&cancellations, "test cancellations").len(), 1);
    }

    #[tokio::test]
    async fn provider_mark_flushes_share_a_workspace_serialization_lock() {
        let first = remote_mark_lock("provider:miniflux");
        let second = remote_mark_lock("provider:miniflux");
        let other = remote_mark_lock("provider:freshrss");
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other));

        let guard = first.lock().await;
        let (started_tx, started_rx) = oneshot::channel();
        let (finished_tx, mut finished_rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = started_tx.send(());
            let _guard = second.lock().await;
            let _ = finished_tx.send(());
        });
        started_rx.await.unwrap();
        assert!(matches!(
            finished_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        drop(guard);
        finished_rx.await.unwrap();
    }
}

pub(super) fn lock_mutex<'a, T>(lock: &'a Mutex<T>, name: &str) -> MutexGuard<'a, T> {
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            lock.clear_poison();
            eprintln!("recovered poisoned {name} mutex");
            guard
        }
    }
}

pub(super) fn read_lock<'a, T>(lock: &'a RwLock<T>, name: &str) -> RwLockReadGuard<'a, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            lock.clear_poison();
            eprintln!("recovered poisoned {name} read lock");
            guard
        }
    }
}

pub(super) fn write_lock<'a, T>(lock: &'a RwLock<T>, name: &str) -> RwLockWriteGuard<'a, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            lock.clear_poison();
            eprintln!("recovered poisoned {name} write lock");
            guard
        }
    }
}
