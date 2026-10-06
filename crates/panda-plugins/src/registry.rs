use crate::{
    CommunityPlugin, PluginCapability, PluginKind, PluginManifest, PluginStage,
    catalog::install_verified,
    rules::{RuleSet, apply_rules_isolated},
    wasm,
};
use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Cursor, Read},
    path::Path,
    sync::Mutex,
};
use url::Url;
use zip::ZipArchive;

const API_VERSION: u32 = 1;
const MAX_PLUGIN_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PLUGIN_ARCHIVE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PLUGIN_ARCHIVE_ENTRIES: usize = 64;

#[derive(Clone, Debug)]
pub struct PluginSummary {
    pub manifest: PluginManifest,
    pub enabled: bool,
    pub bundled: bool,
    pub content_hash: String,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PluginDiagnostic {
    pub plugin_id: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PluginSettings {
    #[serde(default)]
    disabled: HashSet<String>,
}

impl PluginSettings {
    pub fn set_enabled(&mut self, plugin_id: &str, enabled: bool) {
        if enabled {
            self.disabled.remove(plugin_id);
        } else {
            self.disabled.insert(plugin_id.to_owned());
        }
    }

    pub fn remove(&mut self, plugin_id: &str) {
        self.disabled.remove(plugin_id);
    }
}

#[derive(Clone, Debug, Default)]
pub struct PluginResult {
    pub html: String,
    pub body_selected: bool,
    pub matched_plugins: Vec<(String, String)>,
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// Immutable registry snapshot. A job keeps one generation for its full run.
pub struct PluginRegistry {
    generation: u64,
    plugins: Vec<PluginEntry>,
    diagnostics: Vec<PluginDiagnostic>,
    runtime_errors: Mutex<HashMap<String, String>>,
}

struct PluginEntry {
    summary: PluginSummary,
    rules: Option<RuleSet>,
    wasm: Option<Vec<u8>>,
}

impl PluginRegistry {
    pub fn empty() -> Self {
        Self {
            generation: 0,
            plugins: Vec::new(),
            diagnostics: Vec::new(),
            runtime_errors: Mutex::new(HashMap::new()),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Stable cache key covering the ordered manifests, plugin bytes, and enabled state.
    pub fn cache_key(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.generation.to_le_bytes());
        for plugin in &self.plugins {
            hasher.update(plugin.summary.manifest.id.as_bytes());
            hasher.update([0]);
            hasher.update(toml::to_string(&plugin.summary.manifest).unwrap_or_default());
            hasher.update([u8::from(plugin.summary.enabled)]);
            hasher.update(plugin.summary.content_hash.as_bytes());
            hasher.update([0xff]);
        }
        hex::encode(hasher.finalize())
    }
    pub fn diagnostics(&self) -> &[PluginDiagnostic] {
        &self.diagnostics
    }
    pub fn plugins(&self) -> Vec<PluginSummary> {
        let errors = self.runtime_errors.lock().ok();
        self.plugins
            .iter()
            .map(|entry| {
                let mut summary = entry.summary.clone();
                if let Some(error) = errors
                    .as_ref()
                    .and_then(|errors| errors.get(&summary.manifest.id))
                {
                    summary.last_error = Some(error.clone());
                }
                summary
            })
            .collect()
    }

    pub fn load(root: &Path, settings: &PluginSettings, generation: u64) -> Self {
        let mut registry = Self {
            generation,
            plugins: Vec::new(),
            diagnostics: Vec::new(),
            runtime_errors: Mutex::new(HashMap::new()),
        };
        if root.join("manifest.toml").is_file() {
            match load_plugin(root, true) {
                Ok(mut plugin) => {
                    plugin.summary.enabled =
                        !settings.disabled.contains(&plugin.summary.manifest.id);
                    registry.plugins.push(plugin);
                }
                Err(error) => registry.diagnostics.push(PluginDiagnostic {
                    plugin_id: root
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    message: error.to_string(),
                }),
            }
            return registry;
        }
        let Ok(items) = fs::read_dir(root) else {
            return registry;
        };
        for item in items.flatten() {
            if !item.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let directory = item.path();
            let folder_name = item.file_name().to_string_lossy().to_string();
            match load_plugin(&directory, true) {
                Ok(plugin) => {
                    let mut plugin = plugin;
                    plugin.summary.enabled =
                        !settings.disabled.contains(&plugin.summary.manifest.id);
                    if registry
                        .plugins
                        .iter()
                        .any(|loaded| loaded.summary.manifest.id == plugin.summary.manifest.id)
                    {
                        registry.diagnostics.push(PluginDiagnostic {
                            plugin_id: plugin.summary.manifest.id,
                            message: "plugin ID conflicts with an existing plugin".into(),
                        });
                    } else {
                        registry.plugins.push(plugin);
                    }
                }
                Err(error) => registry.diagnostics.push(PluginDiagnostic {
                    plugin_id: folder_name,
                    message: error.to_string(),
                }),
            }
        }
        registry.plugins.sort_by(|a, b| {
            b.summary
                .manifest
                .priority
                .cmp(&a.summary.manifest.priority)
                .then_with(|| a.summary.manifest.id.cmp(&b.summary.manifest.id))
        });
        registry
    }

    pub fn process(
        &self,
        source: &str,
        url: &str,
        title: &str,
        stage: PluginStage,
    ) -> PluginResult {
        let mut output = PluginResult {
            html: source.to_owned(),
            ..PluginResult::default()
        };
        let Some(parsed_url) = Url::parse(url).ok() else {
            return output;
        };
        let Some(host) = parsed_url.host_str() else {
            return output;
        };
        for plugin in &self.plugins {
            let manifest = &plugin.summary.manifest;
            if !plugin.summary.enabled
                || manifest.stage != stage
                || !manifest_matches(manifest, host, &parsed_url)
            {
                continue;
            }
            let applied = match manifest.kind {
                PluginKind::Rules => plugin
                    .rules
                    .as_ref()
                    .context("rules file was not loaded")
                    .and_then(|rules| {
                        let selected_body = rules.selects_body();
                        apply_rules_isolated(&output.html, rules)
                            .map(|result| result.map(|html| (html, selected_body)))
                    }),
                PluginKind::Wasm => plugin
                    .wasm
                    .as_deref()
                    .context("Wasm module was not loaded")
                    .and_then(|bytes| {
                        wasm::process(
                            bytes,
                            &output.html,
                            title,
                            url,
                            stage,
                            &manifest.capabilities,
                        )
                    }),
            };
            match applied {
                Ok(result) => {
                    if let Ok(mut errors) = self.runtime_errors.lock() {
                        errors.remove(&manifest.id);
                    }
                    if let Some((html, selected_body)) = result {
                        if html != output.html || selected_body {
                            output.html = html;
                            output.body_selected |= selected_body;
                            output
                                .matched_plugins
                                .push((manifest.id.clone(), plugin.summary.content_hash.clone()));
                        }
                    }
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    if let Ok(mut errors) = self.runtime_errors.lock() {
                        errors.insert(manifest.id.clone(), message.clone());
                    }
                    output.diagnostics.push(PluginDiagnostic {
                        plugin_id: manifest.id.clone(),
                        message,
                    });
                }
            }
        }
        output
    }

    pub fn set_enabled(&mut self, plugin_id: &str, enabled: bool) -> anyhow::Result<()> {
        let plugin = self
            .plugins
            .iter_mut()
            .find(|item| item.summary.manifest.id == plugin_id)
            .with_context(|| format!("unknown plugin ID: {plugin_id}"))?;
        plugin.summary.enabled = enabled;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    pub fn install_directory(source: &Path, root: &Path) -> anyhow::Result<String> {
        let plugin = load_plugin(source, true)?;
        if plugin.summary.bundled || plugin.summary.manifest.id.starts_with("builtin.") {
            bail!("built-in plugin IDs cannot be imported");
        }
        fs::create_dir_all(root)?;
        let id = plugin.summary.manifest.id;
        let destination = root.join(&id);
        let staging = root.join(format!(".{id}.staging"));
        let backup = root.join(format!(".{id}.backup"));
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        if backup.exists() {
            fs::remove_dir_all(&backup)?;
        }
        fs::create_dir(&staging)?;
        for file in ["manifest.toml", "rules.toml", "plugin.wasm"] {
            let source_file = source.join(file);
            if source_file.is_file() {
                fs::copy(source_file, staging.join(file))?;
            }
        }
        load_plugin(&staging, true)?;
        if destination.exists() {
            fs::rename(&destination, &backup)?;
        }
        if let Err(error) = fs::rename(&staging, &destination) {
            if backup.exists() {
                let _ = fs::rename(&backup, &destination);
            }
            return Err(error.into());
        }
        if backup.exists() {
            fs::remove_dir_all(backup)?;
        }
        Ok(id)
    }

    pub fn install_community_plugin(
        plugin: &CommunityPlugin,
        manifest_bytes: &[u8],
        payload_bytes: &[u8],
        root: &Path,
    ) -> anyhow::Result<String> {
        let staging = tempfile::tempdir().context("create community plugin staging directory")?;
        install_verified(staging.path(), manifest_bytes, payload_bytes, plugin)?;
        Self::install_directory(staging.path(), root)
    }

    pub fn install_archive(bytes: &[u8], root: &Path) -> anyhow::Result<String> {
        if bytes.len() > MAX_PLUGIN_ARCHIVE_BYTES {
            bail!("plugin archive exceeds the 8 MiB download limit");
        }
        let mut archive = ZipArchive::new(Cursor::new(bytes)).context("read plugin ZIP archive")?;
        if archive.len() > MAX_PLUGIN_ARCHIVE_ENTRIES {
            bail!("plugin archive contains more than 64 entries");
        }

        let mut manifest_path = None;
        let mut total_size = 0u64;
        for index in 0..archive.len() {
            let file = archive
                .by_index(index)
                .context("read plugin archive entry")?;
            let Some(path) = file.enclosed_name() else {
                bail!("plugin archive contains an unsafe path");
            };
            total_size = total_size.saturating_add(file.size());
            if total_size > MAX_PLUGIN_ARCHIVE_BYTES as u64 {
                bail!("plugin archive expands beyond the 8 MiB processing limit");
            }
            if !file.is_dir() && path.file_name().is_some_and(|name| name == "manifest.toml") {
                if manifest_path.replace(path).is_some() {
                    bail!("plugin archive contains more than one manifest.toml");
                }
            }
        }
        let manifest_path = manifest_path.context("plugin archive has no manifest.toml")?;
        let directory = manifest_path.parent().unwrap_or_else(|| Path::new(""));
        let staging = tempfile::tempdir().context("create temporary plugin import directory")?;

        for name in ["manifest.toml", "rules.toml", "plugin.wasm"] {
            let expected_path = directory.join(name);
            let mut matched_index = None;
            for index in 0..archive.len() {
                let file = archive
                    .by_index(index)
                    .context("read plugin archive entry")?;
                if file.enclosed_name().as_deref() == Some(expected_path.as_path()) {
                    if matched_index.replace(index).is_some() {
                        bail!("plugin archive contains duplicate {name} files");
                    }
                }
            }
            let Some(index) = matched_index else {
                continue;
            };
            let mut file = archive.by_index(index).context("read plugin payload")?;
            let max_size = if name == "manifest.toml" {
                64 * 1024
            } else {
                MAX_PLUGIN_FILE_BYTES as usize
            };
            if file.size() > max_size as u64 {
                bail!("plugin archive entry {name} exceeds its size limit");
            }
            let mut content = Vec::with_capacity(file.size() as usize);
            file.by_ref()
                .take(max_size as u64 + 1)
                .read_to_end(&mut content)
                .context("decompress plugin archive entry")?;
            if content.len() > max_size {
                bail!("plugin archive entry {name} exceeds its size limit");
            }
            fs::write(staging.path().join(name), content)
                .with_context(|| format!("stage plugin archive entry {name}"))?;
        }
        Self::install_directory(staging.path(), root)
    }

    pub fn remove_user_plugin(root: &Path, plugin_id: &str) -> anyhow::Result<()> {
        validate_plugin_id(plugin_id)?;
        let directory = root.join(plugin_id);
        if directory.exists() {
            fs::remove_dir_all(directory)?;
        }
        Ok(())
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

fn load_plugin(root: &Path, enabled: bool) -> anyhow::Result<PluginEntry> {
    let manifest_bytes = fs::read(root.join("manifest.toml")).context("read manifest.toml")?;
    if manifest_bytes.len() > 64 * 1024 {
        bail!("manifest exceeds 64 KiB");
    }
    let manifest_text =
        std::str::from_utf8(&manifest_bytes).context("manifest.toml is not UTF-8")?;
    let manifest: PluginManifest = toml::from_str(manifest_text).context("parse manifest.toml")?;
    validate_manifest(&manifest)?;
    let payload_path = match manifest.kind {
        PluginKind::Rules => root.join("rules.toml"),
        PluginKind::Wasm => root.join("plugin.wasm"),
    };
    if fs::metadata(&payload_path)
        .context("read plugin payload")?
        .len()
        > MAX_PLUGIN_FILE_BYTES
    {
        bail!("plugin payload exceeds the 4 MiB limit");
    }
    let payload = fs::read(payload_path)?;
    let mut plugin_hash = Sha256::new();
    plugin_hash.update(&manifest_bytes);
    plugin_hash.update(&payload);
    let content_hash = hex::encode(plugin_hash.finalize());
    let rules = if manifest.kind == PluginKind::Rules {
        let rules_text = std::str::from_utf8(&payload).context("rules.toml is not UTF-8")?;
        let rules: RuleSet = toml::from_str(rules_text).context("parse rules.toml")?;
        rules.validate()?;
        let needs_read = !rules.rules.is_empty();
        let needs_write = !rules.rules.is_empty();
        if needs_read
            && !manifest
                .capabilities
                .contains(&PluginCapability::DocumentRead)
        {
            bail!("rules which query the document must declare document.read");
        }
        if needs_write
            && !manifest
                .capabilities
                .contains(&PluginCapability::DocumentWrite)
        {
            bail!("rules which edit the document must declare document.write");
        }
        Some(rules)
    } else {
        None
    };
    let wasm = (manifest.kind == PluginKind::Wasm).then_some(payload);
    Ok(PluginEntry {
        summary: PluginSummary {
            manifest,
            enabled,
            bundled: false,
            content_hash,
            last_error: None,
        },
        rules,
        wasm,
    })
}

fn validate_manifest(manifest: &PluginManifest) -> anyhow::Result<()> {
    validate_plugin_id(&manifest.id)?;
    if manifest.id.starts_with("builtin.") {
        bail!("the `builtin.` ID namespace is reserved");
    }
    if manifest.api_version != API_VERSION {
        bail!("unsupported plugin API version");
    }
    if manifest.name.trim().is_empty()
        || manifest.version.trim().is_empty()
        || manifest.domains.is_empty()
    {
        bail!("manifest requires name, version, and at least one domain");
    }
    for domain in &manifest.domains {
        let bare = domain.strip_prefix("*.").unwrap_or(domain);
        if bare.contains(['/', ':', '@']) || Url::parse(&format!("https://{bare}/")).is_err() {
            bail!("invalid plugin domain pattern: {domain}");
        }
    }
    if manifest
        .path_prefixes
        .iter()
        .any(|path| !path.starts_with('/'))
    {
        bail!("path prefixes must start with `/`");
    }
    Ok(())
}

fn validate_plugin_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        bail!("plugin ID may contain only letters, digits, `.`, `-`, and `_`");
    }
    Ok(())
}

fn manifest_matches(manifest: &PluginManifest, host: &str, url: &Url) -> bool {
    let host_matches = manifest
        .domains
        .iter()
        .any(|pattern| match pattern.strip_prefix("*.") {
            Some(domain) => {
                host.eq_ignore_ascii_case(domain)
                    || host
                        .strip_suffix(domain)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            }
            None => host.eq_ignore_ascii_case(pattern),
        });
    host_matches
        && (manifest.path_prefixes.is_empty()
            || manifest
                .path_prefixes
                .iter()
                .any(|prefix| url.path().starts_with(prefix)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_rules_matches_root_and_subdomains_and_respects_disable_state() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = temp.path().join("example.cleanup");
        fs::create_dir(&plugin).unwrap();
        fs::write(
            plugin.join("manifest.toml"),
            r#"
id = "example.cleanup"
name = "Example cleanup"
version = "1.0.0"
api_version = 1
kind = "rules"
domains = ["*.example.com"]
stage = "cleanup"
priority = 10
capabilities = ["document_read", "document_write"]
"#,
        )
        .unwrap();
        fs::write(
            plugin.join("rules.toml"),
            "[[rules]]\nop = \"remove\"\nselector = \".ad\"\n",
        )
        .unwrap();

        let registry = PluginRegistry::load(temp.path(), &PluginSettings::default(), 1);
        assert!(
            registry
                .process(
                    "<p>Keep</p><aside class='ad'>Remove</aside>",
                    "https://example.com/story",
                    "Title",
                    PluginStage::Cleanup,
                )
                .html
                .contains("Keep")
        );
        let result = registry.process(
            "<p>Keep</p><aside class='ad'>Remove</aside>",
            "https://news.example.com/story",
            "Title",
            PluginStage::Cleanup,
        );
        assert!(!result.html.contains("Remove"));
        assert!(result.diagnostics.is_empty());

        let mut settings = PluginSettings::default();
        settings.set_enabled("example.cleanup", false);
        let disabled = PluginRegistry::load(temp.path(), &settings, 2);
        assert!(
            disabled
                .process(
                    "<aside class='ad'>Keep</aside>",
                    "https://news.example.com/story",
                    "Title",
                    PluginStage::Cleanup,
                )
                .html
                .contains("Keep")
        );
    }

    #[test]
    fn installs_a_community_plugin_from_its_catalog_entry() {
        let catalog =
            crate::CommunityCatalog::parse(include_bytes!("../../../plugins/community/index.json"))
                .unwrap();
        let plugin = catalog
            .plugins
            .iter()
            .find(|plugin| plugin.id == "community.wechat")
            .unwrap();
        let manifest = include_bytes!("../../../plugins/community/community.wechat/manifest.toml");
        let payload = include_bytes!("../../../plugins/community/community.wechat/rules.toml");
        let temp = tempfile::tempdir().unwrap();
        PluginRegistry::install_community_plugin(plugin, manifest, payload, temp.path()).unwrap();

        let registry = PluginRegistry::load(temp.path(), &PluginSettings::default(), 1);
        let result = registry.process(
            "<article><p>Keep</p><img id='wx_img' src='/share.png'></article>",
            "https://mp.weixin.qq.com/s/example",
            "Title",
            PluginStage::Cleanup,
        );
        assert!(result.html.contains("Keep"));
        assert!(!result.html.contains("wx_img"));
    }
}
