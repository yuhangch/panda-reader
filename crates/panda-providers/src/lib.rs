mod freshrss;

use anyhow::Context as _;
use panda_miniflux::{Connection as MinifluxConnection, Entry, Feed, Miniflux};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

pub use freshrss::{FreshRss, FreshRssConnection};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    Miniflux,
    FreshRss,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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

    pub async fn feeds(&self) -> anyhow::Result<Vec<Feed>> {
        match self {
            Self::Miniflux(client) => client.feeds().await,
            Self::FreshRss(client) => client.feeds().await,
        }
    }

    pub async fn all_entries(&self) -> anyhow::Result<Vec<Entry>> {
        match self {
            Self::Miniflux(client) => {
                let mut offset = 0;
                let mut entries = Vec::new();
                loop {
                    let page = client.entries(offset, 500).await?;
                    let count = page.entries.len();
                    entries.extend(page.entries);
                    offset += count;
                    if count == 0 || offset >= page.total {
                        break;
                    }
                }
                Ok(entries)
            }
            Self::FreshRss(client) => client.all_entries().await,
        }
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

    pub async fn refresh(&self) -> anyhow::Result<()> {
        match self {
            Self::Miniflux(client) => client.refresh_feeds().await,
            Self::FreshRss(_) => Ok(()),
        }
    }

    pub async fn mark_entries_status(&self, ids: &[i64], read: bool) -> anyhow::Result<()> {
        match self {
            Self::Miniflux(client) => client.mark_entries_status(ids, read).await,
            Self::FreshRss(client) => client.mark_entries_status(ids, "read", read).await,
        }
    }

    pub async fn set_starred(&self, id: i64, starred: bool) -> anyhow::Result<()> {
        match self {
            Self::Miniflux(client) => client.set_starred(id, starred).await,
            Self::FreshRss(client) => client.set_starred(id, starred).await,
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

#[derive(Clone, Debug)]
pub struct ProviderIdentity {
    pub account: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ProviderEntryPage {
    pub entries: Vec<Entry>,
    pub next_cursor: Option<String>,
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
