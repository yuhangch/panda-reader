mod freshrss;

use anyhow::Context as _;
use panda_miniflux::{Connection as MinifluxConnection, Miniflux};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Mutex, OnceLock},
};

pub use freshrss::{FreshRss, FreshRssConnection};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    Miniflux,
    FreshRss,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCategory {
    pub id: i64,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFeed {
    pub id: i64,
    pub title: String,
    pub feed_url: String,
    pub site_url: String,
    pub language: Option<String>,
    pub category: Option<RemoteCategory>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntry {
    pub id: i64,
    pub feed_id: i64,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub content: String,
    pub status: String,
    pub starred: bool,
    pub changed_at: Option<String>,
    pub revision: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCursor {
    /// Provider-owned opaque data. Store persists this value without interpreting it.
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    Initial,
    Incremental,
    Reconcile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPage {
    pub entries: Vec<RemoteEntry>,
    pub next_cursor: Option<SyncCursor>,
    pub has_more: bool,
    /// Whether this page belongs to a full account reconciliation.
    pub full_sync: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub incremental_sync: bool,
    pub change_tracking: bool,
    pub remote_refresh: bool,
    pub feed_refresh: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshResult {
    Requested,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum MinifluxCursor {
    Full {
        offset: usize,
        #[serde(default)]
        fallback: bool,
    },
    Changed {
        since: i64,
        until: i64,
        offset: usize,
    },
    Checkpoint {
        changed_at: i64,
    },
}

impl ProviderKind {
    pub const ALL: [Self; 2] = [Self::Miniflux, Self::FreshRss];

    pub fn key(self) -> &'static str {
        match self {
            Self::Miniflux => "miniflux",
            Self::FreshRss => "freshrss",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Miniflux => "Miniflux",
            Self::FreshRss => "FreshRSS",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSettings {
    pub endpoint: String,
    pub username: String,
    pub secret: String,
}

pub type ProviderSettingsMap = HashMap<ProviderKind, ProviderSettings>;

pub fn load_settings(path: &Path) -> anyhow::Result<ProviderSettingsMap> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(error.into()),
    }
}

pub fn save_settings(path: &Path, settings: &ProviderSettingsMap) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(settings)?;
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(&bytes)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, bytes)?;
    Ok(())
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            username: String::new(),
            secret: String::new(),
        }
    }
}

#[derive(Clone)]
pub enum ProviderClient {
    Miniflux(Miniflux),
    FreshRss(freshrss::FreshRss),
}

impl ProviderClient {
    pub fn new(kind: ProviderKind, settings: &ProviderSettings) -> anyhow::Result<Self> {
        static CLIENTS: OnceLock<Mutex<HashMap<ProviderKind, (ProviderSettings, ProviderClient)>>> =
            OnceLock::new();
        let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut clients = clients
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((cached_settings, client)) = clients.get(&kind)
            && cached_settings == settings
        {
            return Ok(client.clone());
        }
        let client = Self::build(kind, settings)?;
        clients.insert(kind, (settings.clone(), client.clone()));
        Ok(client)
    }

    fn build(kind: ProviderKind, settings: &ProviderSettings) -> anyhow::Result<Self> {
        match kind {
            ProviderKind::Miniflux => Ok(Self::Miniflux(Miniflux::new(MinifluxConnection::new(
                &settings.endpoint,
                &settings.secret,
            )?)?)),
            ProviderKind::FreshRss => Ok(Self::FreshRss(freshrss::FreshRss::new(
                FreshRssConnection::new(&settings.endpoint, &settings.username, &settings.secret)?,
            )?)),
        }
    }

    pub async fn identity(&self) -> anyhow::Result<ProviderIdentity> {
        match self {
            Self::Miniflux(client) => {
                let user = client.me().await?;
                Ok(ProviderIdentity {
                    account: format!("{}:{}", client.endpoint(), user.id),
                    name: user.username,
                })
            }
            Self::FreshRss(client) => client.identity().await,
        }
    }

    pub fn capabilities(&self) -> ProviderCapabilities {
        match self {
            Self::Miniflux(_) => ProviderCapabilities {
                incremental_sync: true,
                change_tracking: true,
                remote_refresh: true,
                feed_refresh: true,
                ..ProviderCapabilities::default()
            },
            Self::FreshRss(_) => ProviderCapabilities {
                incremental_sync: true,
                change_tracking: false,
                ..ProviderCapabilities::default()
            },
        }
    }

    pub async fn feeds(&self) -> anyhow::Result<Vec<RemoteFeed>> {
        match self {
            Self::Miniflux(client) => Ok(client
                .feeds()
                .await?
                .into_iter()
                .map(|feed| RemoteFeed {
                    id: feed.id,
                    title: feed.title,
                    feed_url: feed.feed_url,
                    site_url: feed.site_url,
                    language: feed.language,
                    category: feed.category.map(|category| RemoteCategory {
                        id: category.id,
                        title: category.title,
                    }),
                })
                .collect()),
            Self::FreshRss(client) => client.feeds().await,
        }
    }

    pub async fn entries_page(
        &self,
        mode: SyncMode,
        cursor: Option<&SyncCursor>,
    ) -> anyhow::Result<SyncPage> {
        match self {
            Self::Miniflux(client) => {
                let stored = cursor
                    .map(|cursor| serde_json::from_str::<MinifluxCursor>(&cursor.value))
                    .transpose()?;
                let changed_window = match (mode, stored.as_ref()) {
                    (
                        SyncMode::Incremental,
                        Some(MinifluxCursor::Changed {
                            since,
                            until,
                            offset,
                        }),
                    ) => Some((*since, *until, *offset)),
                    (SyncMode::Incremental, Some(MinifluxCursor::Checkpoint { changed_at })) => {
                        Some((
                            changed_at.saturating_sub(2),
                            chrono::Utc::now().timestamp(),
                            0,
                        ))
                    }
                    (SyncMode::Incremental, None) => Some((0, chrono::Utc::now().timestamp(), 0)),
                    _ => None,
                };
                let fallback_offset = match (mode, stored.as_ref()) {
                    (
                        SyncMode::Incremental,
                        Some(MinifluxCursor::Full {
                            offset,
                            fallback: true,
                        }),
                    ) => Some(*offset),
                    _ => None,
                };
                let offset = changed_window
                    .map(|(_, _, offset)| offset)
                    .or(fallback_offset)
                    .or_else(|| match stored.as_ref() {
                        Some(MinifluxCursor::Full { offset, .. }) => Some(*offset),
                        _ => None,
                    })
                    .unwrap_or(0);
                let mut full_sync =
                    !matches!(mode, SyncMode::Incremental) || fallback_offset.is_some();
                let page = if let Some((since, until, _)) = changed_window {
                    match client
                        .entries_changed(offset, 500, Some(since), Some(until))
                        .await
                    {
                        Ok(page) => page,
                        Err(error) if unsupported_change_filter(&error) => {
                            full_sync = true;
                            client.entries(offset, 500).await?
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    client.entries(offset, 500).await?
                };
                let next_offset = offset + page.entries.len();
                let has_more = next_offset < page.total && !page.entries.is_empty();
                let next_cursor = if has_more {
                    let next = if full_sync {
                        MinifluxCursor::Full {
                            offset: next_offset,
                            fallback: fallback_offset.is_some()
                                || (changed_window.is_some() && full_sync),
                        }
                    } else if let Some((since, until, _)) = changed_window {
                        MinifluxCursor::Changed {
                            since,
                            until,
                            offset: next_offset,
                        }
                    } else {
                        MinifluxCursor::Full {
                            offset: next_offset,
                            fallback: false,
                        }
                    };
                    next
                } else if let Some((_, until, _)) = changed_window.filter(|_| !full_sync) {
                    MinifluxCursor::Checkpoint { changed_at: until }
                } else {
                    MinifluxCursor::Checkpoint {
                        changed_at: chrono::Utc::now().timestamp(),
                    }
                };
                let entries = page
                    .entries
                    .into_iter()
                    .map(|entry| RemoteEntry {
                        id: entry.id,
                        feed_id: entry.feed_id,
                        title: entry.title,
                        url: entry.url,
                        author: entry.author,
                        published_at: entry.published_at,
                        content: entry.content,
                        status: entry.status,
                        starred: entry.starred,
                        changed_at: entry.changed_at,
                        revision: entry.hash,
                    })
                    .collect();
                Ok(SyncPage {
                    entries,
                    next_cursor: Some(SyncCursor {
                        value: serde_json::to_string(&next_cursor)?,
                    }),
                    has_more,
                    full_sync,
                })
            }
            Self::FreshRss(client) => client.entries_page(mode, cursor).await,
        }
    }

    pub async fn all_entries(&self) -> anyhow::Result<Vec<RemoteEntry>> {
        if let Self::FreshRss(client) = self {
            return client.all_entries().await;
        }
        let mut mode = SyncMode::Initial;
        let mut cursor = None;
        let mut entries = Vec::new();
        loop {
            let page = self.entries_page(mode, cursor.as_ref()).await?;
            entries.extend(page.entries);
            if !page.has_more {
                break;
            }
            cursor = page.next_cursor;
            mode = SyncMode::Initial;
        }
        Ok(entries)
    }

    pub async fn add_feed(&self, url: &str) -> anyhow::Result<()> {
        match self {
            Self::Miniflux(client) => client.add_feed(url).await,
            Self::FreshRss(client) => client.add_feed(url).await,
        }
    }

    pub async fn remove_feed(&self, id: i64) -> anyhow::Result<()> {
        match self {
            Self::Miniflux(client) => client.remove_feed(id).await,
            Self::FreshRss(client) => client.remove_feed(id).await,
        }
    }

    pub async fn refresh_all(&self) -> anyhow::Result<RefreshResult> {
        match self {
            Self::Miniflux(client) => {
                client.refresh_feeds().await?;
                Ok(RefreshResult::Requested)
            }
            Self::FreshRss(_) => Ok(RefreshResult::Unsupported),
        }
    }

    pub async fn refresh_feed(&self, remote_feed_id: i64) -> anyhow::Result<RefreshResult> {
        match self {
            Self::Miniflux(client) => {
                client.refresh_feed(remote_feed_id).await?;
                Ok(RefreshResult::Requested)
            }
            Self::FreshRss(_) => Ok(RefreshResult::Unsupported),
        }
    }

    pub async fn mark_entries_status(&self, ids: &[i64], read: bool) -> anyhow::Result<()> {
        match self {
            Self::Miniflux(client) => client.mark_entries_status(ids, read).await,
            Self::FreshRss(client) => client.mark_entries_status(ids, "read", read).await,
        }
    }

    pub async fn set_starred(&self, id: i64, starred: bool) -> anyhow::Result<()> {
        self.set_starred_entries(&[id], starred).await
    }

    pub async fn set_starred_entries(&self, ids: &[i64], starred: bool) -> anyhow::Result<()> {
        match self {
            Self::Miniflux(client) => {
                for id in ids {
                    client.set_starred(*id, starred).await?;
                }
                Ok(())
            }
            Self::FreshRss(client) => client.mark_entries_status(ids, "starred", starred).await,
        }
    }

    pub async fn import_opml(&self, xml: &str) -> anyhow::Result<()> {
        match self {
            Self::Miniflux(client) => client.import_opml(xml).await,
            Self::FreshRss(client) => client.import_opml(xml).await,
        }
    }

    pub async fn export_opml(&self) -> anyhow::Result<String> {
        match self {
            Self::Miniflux(client) => client.export_opml().await,
            Self::FreshRss(client) => client.export_opml().await,
        }
    }
}

fn unsupported_change_filter(error: &anyhow::Error) -> bool {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .and_then(reqwest::Error::status)
        .is_some_and(|status| matches!(status.as_u16(), 400 | 404 | 422))
}

#[derive(Clone, Debug)]
pub struct ProviderIdentity {
    pub account: String,
    pub name: String,
}

fn numeric_id(value: &str) -> anyhow::Result<i64> {
    value
        .rsplit('/')
        .next()
        .context("Provider returned an unsupported non-numeric item ID")?
        .parse::<i64>()
        .context("Provider returned an unsupported non-numeric item ID")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_has_stable_config_keys() {
        assert_eq!(ProviderKind::Miniflux.key(), "miniflux");
        assert_eq!(ProviderKind::FreshRss.key(), "freshrss");
        assert_eq!(
            serde_json::to_string(&ProviderKind::FreshRss).unwrap(),
            "\"fresh_rss\""
        );
    }

    #[test]
    fn parses_google_reader_numeric_ids() {
        assert_eq!(
            numeric_id("tag:google.com,2005:reader/item/123").unwrap(),
            123
        );
        assert!(numeric_id("tag:other/item/x").is_err());
    }

    #[test]
    fn miniflux_older_versions_fall_back_to_resumable_full_sync() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let responses = [
                (
                    400,
                    "{\"error_message\":\"unknown changed_after parameter\"}",
                ),
                (
                    200,
                    r#"{"total":2,"entries":[{"id":2,"feed_id":1,"title":"Second","status":"unread"}]}"#,
                ),
                (
                    200,
                    r#"{"total":2,"entries":[{"id":1,"feed_id":1,"title":"First","status":"unread"}]}"#,
                ),
            ];
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 2048];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]).to_string();
                let reason = if status == 200 { "OK" } else { "Bad Request" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                requests.push(request);
            }
            requests
        });

        let remote = ProviderClient::new(
            ProviderKind::Miniflux,
            &ProviderSettings {
                endpoint: format!("http://{address}"),
                username: String::new(),
                secret: "secret".into(),
            },
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let first = runtime
            .block_on(remote.entries_page(SyncMode::Incremental, None))
            .unwrap();
        assert!(first.full_sync);
        assert!(first.has_more);
        assert_eq!(first.entries[0].id, 2);
        let second = runtime
            .block_on(remote.entries_page(SyncMode::Incremental, first.next_cursor.as_ref()))
            .unwrap();
        assert!(second.full_sync);
        assert!(!second.has_more);
        assert_eq!(second.entries[0].id, 1);

        let requests = server.join().unwrap();
        assert!(
            requests[0]
                .lines()
                .next()
                .unwrap()
                .contains("changed_after")
        );
        assert!(requests[1].lines().next().unwrap().contains("offset=0"));
        assert!(requests[2].lines().next().unwrap().contains("offset=1"));
        assert!(
            !requests[1]
                .lines()
                .next()
                .unwrap()
                .contains("changed_after")
        );
        assert!(
            !requests[2]
                .lines()
                .next()
                .unwrap()
                .contains("changed_after")
        );
    }

    #[cfg(unix)]
    #[test]
    fn provider_settings_are_restricted_even_when_file_already_exists() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("providers.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        save_settings(&path, &ProviderSettingsMap::new()).unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
