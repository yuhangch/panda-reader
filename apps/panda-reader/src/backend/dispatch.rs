use super::article::prepare_article;
use super::command::Command;
use super::favicon::{fetch_favicon, sanitize_host};
use super::job::{job, sync};
use crate::reader_body::{BodyCache, BodyPrepKey};
use panda_core::{MarkField, PreparedArticle};
use panda_providers::{
    ProviderClient, ProviderKind, ProviderSettings, ProviderSettingsMap, save_settings,
};
use panda_store::Store;
use panda_translate::TranslatorConfig;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};

pub struct WorkerState {
    pub path: PathBuf,
    pub provider_settings_path: PathBuf,
    pub translator_path: PathBuf,
    pub provider_settings: Arc<RwLock<ProviderSettingsMap>>,
    pub active_workspace: Arc<RwLock<String>>,
    pub body_cache: Arc<Mutex<BodyCache>>,
    pub favicon_inflight: Arc<Mutex<HashMap<String, ()>>>,
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

    fn save_provider_settings(&self, values: &ProviderSettingsMap) -> anyhow::Result<()> {
        save_settings(&self.provider_settings_path, values)
    }
}

pub fn handle(command: Command, state: &WorkerState) {
    match command {
        Command::Snapshot {
            scope,
            search,
            limit,
            after,
            include_feeds,
            reply,
        } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            job(reply, move |_| {
                let store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                store
                    .snapshot(scope, &search, limit, after.as_ref(), include_feeds)
                    .map_err(|e| e.to_string())
            });
        }
        Command::Article {
            id,
            show_translation,
            translation_layout,
            hide_images,
            reply,
        } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            let cache = state.body_cache.clone();
            job(reply, move |_| {
                let store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                prepare_article(
                    &store,
                    &cache,
                    id,
                    show_translation,
                    translation_layout,
                    hide_images,
                )
            });
        }
        Command::EnsureFavicon {
            host,
            site_url,
            icons_dir,
            reply,
        } => {
            let inflight = state.favicon_inflight.clone();
            job(reply, move |runtime| {
                {
                    let mut guard = inflight.lock().map_err(|e| e.to_string())?;
                    if guard.contains_key(&host) {
                        let safe = sanitize_host(&host);
                        let path = icons_dir.join(format!("{safe}.png"));
                        if path.is_file() {
                            return Ok(path);
                        }
                    }
                    guard.insert(host.clone(), ());
                }
                let result = runtime.block_on(fetch_favicon(&host, &site_url, &icons_dir));
                let _ = inflight.lock().map(|mut g| g.remove(&host));
                result
            });
        }
        Command::Connect {
            kind,
            endpoint,
            username,
            secret,
            reply,
        } => {
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
                let mut store =
                    Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                let message = match runtime.block_on(store.sync_provider(&remote, kind)) {
                    Ok(_) => identity.name,
                    Err(error) => format!("{}; initial sync failed: {error}", identity.name),
                };
                Ok(message)
            });
        }
        Command::Disconnect { kind, reply } => {
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
        Command::AddFeed { url, reply } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            let provider = state.provider_kind();
            let config = provider.and_then(|kind| state.provider_settings(kind));
            job(reply, move |runtime| {
                let mut store =
                    Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
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
        Command::RemoveFeed { id, reply } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            let provider = state.provider_kind();
            let config = provider.and_then(|kind| state.provider_settings(kind));
            job(reply, move |runtime| {
                let mut store =
                    Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                let (remote_id, _, source) =
                    store.feed_removal_info(id).map_err(|e| e.to_string())?;
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
        Command::UpdateFeed {
            id,
            title,
            folder,
            feed_url,
            reply,
        } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            job(reply, move |_| {
                if workspace != "local" {
                    return Err("Feed editing is currently available in local mode only".into());
                }
                let store = Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                store
                    .update_feed(id, &title, folder.as_deref(), &feed_url)
                    .map_err(|e| e.to_string())
            });
        }
        Command::RefreshFeed { id, reply } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            let provider = state.provider_kind();
            let config = provider.and_then(|kind| state.provider_settings(kind));
            job(reply, move |runtime| {
                let mut store =
                    Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
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
        Command::Refresh { reply } => {
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
        Command::MarkAllRead { scope, reply } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            let provider = state.provider_kind();
            let config = provider.and_then(|kind| state.provider_settings(kind));
            job(reply, move |runtime| {
                let mut store =
                    Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                let count = store.mark_all_read(scope).map_err(|e| e.to_string())?;
                if let (Some(kind), Some(config)) = (provider, config) {
                    let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
                    runtime
                        .block_on(store.flush_provider_marks(&remote))
                        .map_err(|e| e.to_string())?;
                }
                Ok(count)
            });
        }
        Command::Mark {
            id,
            field,
            value,
            reply,
        } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            let provider = state.provider_kind();
            let config = provider.and_then(|kind| state.provider_settings(kind));
            job(reply, move |runtime| {
                let mut store =
                    Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                let remote_id = store.remote_entry_id(id).map_err(|e| e.to_string())?;
                store.mark(id, field, value).map_err(|e| e.to_string())?;
                if remote_id.is_some()
                    && !matches!(field, MarkField::Later)
                    && let (Some(kind), Some(config)) = (provider, config)
                {
                    let remote = ProviderClient::new(kind, &config).map_err(|e| e.to_string())?;
                    runtime
                        .block_on(store.flush_provider_marks(&remote))
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            });
        }
        Command::Extract {
            id,
            force,
            extractor,
            reply,
        } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            let cache = state.body_cache.clone();
            job(reply, move |runtime| {
                let mut store =
                    Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                let result = runtime
                    .block_on(store.extract(id, force, extractor))
                    .map_err(|e| e.to_string());
                if result.is_ok() {
                    if let Ok(mut cache) = cache.lock() {
                        cache.invalidate_article(id);
                    }
                }
                result
            });
        }
        Command::Translate {
            id,
            target_lang,
            translation_layout,
            hide_images,
            reply,
        } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            let translator_path = state.translator_path.clone();
            let cache = state.body_cache.clone();
            job(reply, move |runtime| {
                let config = TranslatorConfig::load(&translator_path).map_err(|e| e.to_string())?;
                let translator = panda_translate::build(&config).map_err(|e| e.to_string())?;
                let mut store =
                    Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
                let article = runtime
                    .block_on(store.translate(id, &target_lang, &translator))
                    .map_err(|e| e.to_string())?;
                if let Ok(mut cache) = cache.lock() {
                    cache.invalidate_article(id);
                }
                let body_html = {
                    let mut guard = cache.lock().map_err(|e| e.to_string())?;
                    let key =
                        BodyPrepKey::from_article(&article, true, translation_layout, hide_images);
                    guard.get_or_insert(key, &article, true, translation_layout, hide_images)
                };
                Ok(PreparedArticle { article, body_html })
            });
        }
        Command::ImportOpml { content, reply } => {
            let path = state.path.clone();
            let workspace = state.workspace();
            let provider = state.provider_kind();
            let config = provider.and_then(|kind| state.provider_settings(kind));
            job(reply, move |runtime| {
                let mut store =
                    Store::open_workspace(&path, &workspace).map_err(|e| e.to_string())?;
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
        Command::ExportOpml { reply } => {
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
    }
}
