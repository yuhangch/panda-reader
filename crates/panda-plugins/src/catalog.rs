//! Validated metadata and payload verification for the community plugin catalog.

use crate::{PluginKind, PluginManifest};
use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
};
use url::Url;

const CATALOG_SCHEMA_VERSION: u32 = 1;
const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_PLUGIN_FILE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommunityCatalog {
    pub schema_version: u32,
    pub plugins: Vec<CommunityPlugin>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommunityPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub min_app_version: String,
    pub kind: PluginKind,
    pub files: BTreeMap<String, CommunityPluginFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommunityPluginFile {
    pub url: String,
    pub sha256: String,
}

impl CommunityCatalog {
    pub fn parse(bytes: &[u8]) -> anyhow::Result<Self> {
        if bytes.len() > MAX_CATALOG_BYTES {
            bail!("community plugin catalog exceeds 2 MiB");
        }
        let catalog: Self =
            serde_json::from_slice(bytes).context("parse community plugin catalog")?;
        if catalog.schema_version != CATALOG_SCHEMA_VERSION {
            bail!("unsupported community plugin catalog schema version");
        }
        let mut ids = HashSet::new();
        for plugin in &catalog.plugins {
            plugin.validate()?;
            if !ids.insert(plugin.id.as_str()) {
                bail!("duplicate community plugin ID: {}", plugin.id);
            }
        }
        Ok(catalog)
    }
}

impl CommunityPlugin {
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_version(&self.version, "plugin")?;
        validate_version(&self.min_app_version, "minimum app")?;
        if !self.id.starts_with("community.")
            || self.id.len() > 96
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            bail!("invalid community plugin ID: {}", self.id);
        }
        if self.name.trim().is_empty() || self.api_version == 0 {
            bail!("community plugin {} has incomplete metadata", self.id);
        }
        let payload_name = match self.kind {
            PluginKind::Rules => "rules.toml",
            PluginKind::Wasm => "plugin.wasm",
        };
        if self.files.len() != 2
            || !self.files.contains_key("manifest.toml")
            || !self.files.contains_key(payload_name)
        {
            bail!("community plugin {} has an invalid file list", self.id);
        }
        for (name, file) in &self.files {
            if name != "manifest.toml" && name != payload_name {
                bail!("community plugin {} lists an unsupported file", self.id);
            }
            validate_file(file).with_context(|| format!("validate {name} for {}", self.id))?;
        }
        Ok(())
    }

    pub fn payload_name(&self) -> &'static str {
        match self.kind {
            PluginKind::Rules => "rules.toml",
            PluginKind::Wasm => "plugin.wasm",
        }
    }
}

pub(crate) fn install_verified(
    source: &Path,
    manifest_bytes: &[u8],
    payload_bytes: &[u8],
    plugin: &CommunityPlugin,
) -> anyhow::Result<()> {
    plugin.validate()?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES || payload_bytes.len() > MAX_PLUGIN_FILE_BYTES {
        bail!("community plugin file exceeds its allowed size");
    }
    verify_hash(
        manifest_bytes,
        &plugin.files["manifest.toml"].sha256,
        "manifest.toml",
    )?;
    verify_hash(
        payload_bytes,
        &plugin.files[plugin.payload_name()].sha256,
        plugin.payload_name(),
    )?;

    let manifest: PluginManifest = toml::from_str(
        std::str::from_utf8(manifest_bytes).context("plugin manifest is not UTF-8")?,
    )
    .context("parse downloaded plugin manifest")?;
    if manifest.id != plugin.id
        || manifest.name != plugin.name
        || manifest.version != plugin.version
        || manifest.api_version != plugin.api_version
        || manifest.kind != plugin.kind
    {
        bail!("downloaded plugin manifest does not match the catalog entry");
    }

    fs::create_dir_all(source)?;
    fs::write(source.join("manifest.toml"), manifest_bytes)?;
    fs::write(source.join(plugin.payload_name()), payload_bytes)?;
    Ok(())
}

fn validate_file(file: &CommunityPluginFile) -> anyhow::Result<()> {
    let url = Url::parse(&file.url).context("invalid plugin file URL")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("plugin file URLs must use HTTPS and cannot include credentials");
    }
    if file.sha256.len() != 64 || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("plugin file SHA-256 must contain 64 hexadecimal characters");
    }
    Ok(())
}

fn validate_version(value: &str, label: &str) -> anyhow::Result<()> {
    semver::Version::parse(value).with_context(|| format!("invalid {label} version {value}"))?;
    Ok(())
}

fn verify_hash(bytes: &[u8], expected: &str, label: &str) -> anyhow::Result<()> {
    let actual = hex::encode(Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("downloaded {label} failed SHA-256 verification");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_https_catalog_urls_and_duplicate_ids() {
        let mut catalog = sample_catalog();
        catalog.plugins[0].files.get_mut("rules.toml").unwrap().url =
            "http://example.com/rules.toml".into();
        assert!(catalog.plugins[0].validate().is_err());

        let catalog = sample_catalog();
        let mut duplicate = catalog.plugins[0].clone();
        duplicate.name = "Second name".into();
        let bytes = serde_json::to_vec(&CommunityCatalog {
            schema_version: CATALOG_SCHEMA_VERSION,
            plugins: vec![catalog.plugins[0].clone(), duplicate],
        })
        .unwrap();
        assert!(CommunityCatalog::parse(&bytes).is_err());
    }

    #[test]
    fn parses_the_repository_community_index() {
        let catalog =
            CommunityCatalog::parse(include_bytes!("../../../plugins/community/index.json"))
                .unwrap();
        assert!(
            catalog
                .plugins
                .iter()
                .any(|plugin| plugin.id == "community.wechat")
        );
        assert!(
            catalog
                .plugins
                .iter()
                .any(|plugin| plugin.id == "community.qbitai")
        );
    }

    #[test]
    fn verifies_catalog_hashes_and_manifest_identity_before_staging() {
        let manifest = br#"id = "community.example"
name = "Example"
version = "1.0.0"
api_version = 1
kind = "rules"
domains = ["example.com"]
stage = "cleanup"
capabilities = ["document_read", "document_write"]
"#;
        let payload = b"[[rules]]\nop = \"remove\"\nselector = \".ad\"\n";
        let plugin = CommunityPlugin {
            id: "community.example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            api_version: 1,
            min_app_version: "0.2.0".into(),
            kind: PluginKind::Rules,
            files: BTreeMap::from([
                (
                    "manifest.toml".into(),
                    CommunityPluginFile {
                        url: "https://example.com/manifest.toml".into(),
                        sha256: hex::encode(Sha256::digest(manifest)),
                    },
                ),
                (
                    "rules.toml".into(),
                    CommunityPluginFile {
                        url: "https://example.com/rules.toml".into(),
                        sha256: hex::encode(Sha256::digest(payload)),
                    },
                ),
            ]),
        };
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        install_verified(&staging, manifest, payload, &plugin).unwrap();
        assert!(staging.join("manifest.toml").is_file());
        assert_eq!(fs::read(staging.join("rules.toml")).unwrap(), payload);

        let wrong = temp.path().join("wrong");
        assert!(install_verified(&wrong, manifest, b"changed", &plugin).is_err());
        assert!(!wrong.exists());

        let mut mismatched = plugin.clone();
        mismatched.name = "Different".into();
        let wrong_identity = temp.path().join("wrong-identity");
        assert!(install_verified(&wrong_identity, manifest, payload, &mismatched).is_err());
        assert!(!wrong_identity.exists());
    }

    fn sample_catalog() -> CommunityCatalog {
        CommunityCatalog {
            schema_version: CATALOG_SCHEMA_VERSION,
            plugins: vec![CommunityPlugin {
                id: "community.example".into(),
                name: "Example".into(),
                version: "1.0.0".into(),
                api_version: 1,
                min_app_version: "0.2.0".into(),
                kind: PluginKind::Rules,
                files: BTreeMap::from([
                    (
                        "manifest.toml".into(),
                        CommunityPluginFile {
                            url: "https://example.com/manifest.toml".into(),
                            sha256: "0".repeat(64),
                        },
                    ),
                    (
                        "rules.toml".into(),
                        CommunityPluginFile {
                            url: "https://example.com/rules.toml".into(),
                            sha256: "0".repeat(64),
                        },
                    ),
                ]),
            }],
        }
    }
}
