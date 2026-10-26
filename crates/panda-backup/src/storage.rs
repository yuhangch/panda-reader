use anyhow::{Context as _, ensure};
use opendal::{Operator, services};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::File,
    io::{Read, Write},
    path::{Component, Path},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackupTargetConfig {
    WebDav(WebDavTarget),
    S3(S3Target),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WebDavTarget {
    pub endpoint: String,
    pub username: String,
    pub password: String,
    pub directory: String,
    #[serde(default)]
    pub allow_insecure_http: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct S3Target {
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
    pub path_style: bool,
    #[serde(default)]
    pub allow_insecure_http: bool,
}

impl fmt::Debug for BackupTargetConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WebDav(target) => f
                .debug_struct("WebDavTarget")
                .field("endpoint", &target.endpoint)
                .field("username", &target.username)
                .field("password", &"[REDACTED]")
                .field("directory", &target.directory)
                .finish(),
            Self::S3(target) => f
                .debug_struct("S3Target")
                .field("endpoint", &target.endpoint)
                .field("region", &target.region)
                .field("bucket", &target.bucket)
                .field("prefix", &target.prefix)
                .field("access_key", &"[REDACTED]")
                .field("secret_key", &"[REDACTED]")
                .field("session_token", &"[REDACTED]")
                .field("path_style", &target.path_style)
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupItem {
    pub archive_name: String,
    pub created_at: String,
    pub mode: String,
    pub encrypted_bytes: u64,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
struct CompleteMarker {
    archive_name: String,
    encrypted_bytes: u64,
    sha256: String,
    mode: String,
    created_at: String,
}

pub struct RemoteStorage {
    operator: Operator,
}

impl RemoteStorage {
    pub fn new(config: &BackupTargetConfig) -> anyhow::Result<Self> {
        match config {
            BackupTargetConfig::WebDav(target) => {
                validate_endpoint(&target.endpoint, target.allow_insecure_http)?;
                let root = object_prefix(&format!("{}/PandaReader/backups/v1", target.directory))?;
                let builder = services::Webdav::default()
                    .endpoint(&target.endpoint)
                    .username(&target.username)
                    .password(&target.password)
                    .root(&root);
                let operator = Operator::new(builder)?;
                Ok(Self { operator })
            }
            BackupTargetConfig::S3(target) => {
                ensure!(!target.bucket.trim().is_empty(), "S3 bucket is required");
                ensure!(!target.region.trim().is_empty(), "S3 region is required");
                ensure!(
                    !target.access_key.trim().is_empty() && !target.secret_key.trim().is_empty(),
                    "S3 access key and secret key are required"
                );
                if let Some(endpoint) = &target.endpoint {
                    validate_endpoint(endpoint, target.allow_insecure_http)?;
                }
                let root = object_prefix(&format!("{}/PandaReader/backups/v1", target.prefix))?;
                let mut builder = services::S3::default()
                    .bucket(&target.bucket)
                    .region(&target.region)
                    .access_key_id(&target.access_key)
                    .secret_access_key(&target.secret_key)
                    .disable_config_load()
                    .disable_ec2_metadata()
                    .root(&root);
                if let Some(endpoint) = target
                    .endpoint
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    builder = builder.endpoint(endpoint);
                }
                if let Some(token) = target
                    .session_token
                    .as_deref()
                    .filter(|value| !value.is_empty())
                {
                    builder = builder.session_token(token);
                }
                if !target.path_style {
                    builder = builder.enable_virtual_host_style();
                }
                let operator = Operator::new(builder)?;
                Ok(Self { operator })
            }
        }
    }

    pub async fn test_connection(&self) -> anyhow::Result<()> {
        self.operator.list("").await.map_err(|_| anyhow::anyhow!("Cloud storage connection failed; check endpoint, credentials, permissions and network"))?;
        Ok(())
    }

    /// Upload an encrypted archive, then publish its completion marker last.
    pub async fn upload(
        &self,
        encrypted_file: &Path,
        archive_name: &str,
        mode: &str,
        created_at: &str,
    ) -> anyhow::Result<BackupItem> {
        validate_object_name(archive_name)?;
        ensure!(
            archive_name.ends_with(".prbackup.age"),
            "cloud backups must use age encryption"
        );
        let (size, sha256) = hash_file(encrypted_file)?;
        let archive_path = archive_name.to_owned();
        let marker_path = format!("{archive_path}.complete.json");
        let exists = self.operator.exists(&archive_path).await.map_err(|_| {
            anyhow::anyhow!("Cloud storage could not verify that the backup name is unused")
        })?;
        ensure!(!exists, "backup object already exists");
        let mut writer = self
            .operator
            .writer(&archive_path)
            .await
            .map_err(|_| anyhow::anyhow!("Cloud upload could not be started"))?;
        let mut input = File::open(encrypted_file)?;
        let mut buffer = [0; 256 * 1024];
        let upload_result = async {
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                writer.write(buffer[..count].to_vec()).await?;
            }
            writer.close().await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(error) = upload_result {
            let _ = self.operator.delete(&archive_path).await;
            return Err(error).context("cloud backup upload failed");
        }
        let marker = CompleteMarker {
            archive_name: archive_name.into(),
            encrypted_bytes: size,
            sha256: sha256.clone(),
            mode: mode.into(),
            created_at: created_at.into(),
        };
        if self
            .operator
            .write(&marker_path, serde_json::to_vec(&marker)?)
            .await
            .is_err()
        {
            let _ = self.operator.delete(&archive_path).await;
            return Err(anyhow::anyhow!(
                "Cloud backup marker could not be published"
            ));
        }
        Ok(BackupItem {
            archive_name: archive_name.into(),
            created_at: created_at.into(),
            mode: mode.into(),
            encrypted_bytes: size,
            sha256,
        })
    }

    pub async fn list(&self) -> anyhow::Result<Vec<BackupItem>> {
        let entries = self
            .operator
            .list("")
            .await
            .map_err(|_| anyhow::anyhow!("Cloud backup history could not be loaded"))?;
        let mut result = Vec::new();
        for entry in entries {
            let marker_name = entry.name();
            if !marker_name.ends_with(".prbackup.age.complete.json") {
                continue;
            }
            let Ok(bytes) = self.operator.read(marker_name).await else {
                continue;
            };
            let Ok(marker) = serde_json::from_slice::<CompleteMarker>(&bytes.to_bytes()) else {
                continue;
            };
            if validate_object_name(&marker.archive_name).is_err()
                || format!("{}.complete.json", marker.archive_name) != marker_name
            {
                continue;
            }
            let Ok(metadata) = self.operator.stat(&marker.archive_name).await else {
                continue;
            };
            if metadata.content_length() != marker.encrypted_bytes {
                continue;
            }
            result.push(BackupItem {
                archive_name: marker.archive_name,
                created_at: marker.created_at,
                mode: marker.mode,
                encrypted_bytes: marker.encrypted_bytes,
                sha256: marker.sha256,
            });
        }
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn download(&self, item: &BackupItem, destination: &Path) -> anyhow::Result<()> {
        validate_object_name(&item.archive_name)?;
        let temp = tempfile::NamedTempFile::new_in(
            destination.parent().unwrap_or_else(|| Path::new(".")),
        )?;
        let reader = self
            .operator
            .reader(&item.archive_name)
            .await
            .map_err(|_| anyhow::anyhow!("Cloud backup download could not be started"))?;
        let output = temp.as_file().try_clone()?;
        let mut output = std::io::BufWriter::new(output);
        let mut offset = 0_u64;
        loop {
            let chunk = reader
                .read(offset..offset.saturating_add(256 * 1024))
                .await
                .map_err(|_| anyhow::anyhow!("Cloud backup download failed"))?;
            if chunk.is_empty() {
                break;
            }
            let chunk = chunk.to_bytes();
            offset = offset.saturating_add(chunk.len() as u64);
            output.write_all(&chunk)?;
        }
        output.flush()?;
        output.get_ref().sync_all()?;
        let (size, hash) = hash_file(temp.path())?;
        ensure!(
            size == item.encrypted_bytes && hash == item.sha256,
            "downloaded backup checksum mismatch"
        );
        temp.persist(destination).map_err(|error| error.error)?;
        Ok(())
    }

    pub async fn delete(&self, item: &BackupItem) -> anyhow::Result<()> {
        validate_object_name(&item.archive_name)?;
        self.operator
            .delete(&format!("{}.complete.json", item.archive_name))
            .await
            .map_err(|_| anyhow::anyhow!("Backup completion marker could not be deleted"))?;
        self.operator
            .delete(&item.archive_name)
            .await
            .map_err(|_| anyhow::anyhow!("Backup archive could not be deleted"))?;
        Ok(())
    }
}

pub fn save_targets(path: &Path, targets: &[BackupTargetConfig]) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    std::fs::write(temp.path(), serde_json::to_vec_pretty(targets)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub fn load_targets(path: &Path) -> anyhow::Result<Vec<BackupTargetConfig>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn object_prefix(value: &str) -> anyhow::Result<String> {
    let normalized = value.trim_matches('/');
    ensure!(
        Path::new(normalized)
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "remote directory contains an unsafe path"
    );
    if normalized.is_empty() {
        Ok("/".into())
    } else {
        Ok(format!("/{normalized}/"))
    }
}

fn validate_endpoint(value: &str, allow_insecure_http: bool) -> anyhow::Result<()> {
    let url = url::Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "backup endpoint must use HTTP or HTTPS"
    );
    if url.scheme() == "http" {
        let localhost = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        ensure!(
            localhost || allow_insecure_http,
            "HTTP backup endpoints require explicit risk confirmation"
        );
    }
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "credentials must be entered in their separate fields"
    );
    Ok(())
}

fn validate_object_name(name: &str) -> anyhow::Result<()> {
    ensure!(
        !name.is_empty()
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')),
        "backup object name is invalid"
    );
    ensure!(
        !name.starts_with('.') && !name.contains(".."),
        "backup object name is invalid"
    );
    Ok(())
}

fn hash_file(path: &Path) -> anyhow::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        hasher.update(&buffer[..count]);
    }
    Ok((total, hex::encode(hasher.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_debug_never_includes_credentials() {
        let target = BackupTargetConfig::S3(S3Target {
            endpoint: None,
            region: "us-east-1".into(),
            bucket: "reader".into(),
            prefix: String::new(),
            access_key: "ACCESS-SECRET".into(),
            secret_key: "SECRET-SECRET".into(),
            session_token: Some("SESSION-SECRET".into()),
            path_style: true,
            allow_insecure_http: false,
        });
        let debug = format!("{target:?}");
        assert!(!debug.contains("ACCESS-SECRET"));
        assert!(!debug.contains("SECRET-SECRET"));
        assert!(!debug.contains("SESSION-SECRET"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn target_credentials_are_saved_with_private_unix_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup-targets.json");
        let targets = vec![BackupTargetConfig::WebDav(WebDavTarget {
            endpoint: "https://dav.example".into(),
            username: "reader".into(),
            password: "secret".into(),
            directory: "PandaReader".into(),
            allow_insecure_http: false,
        })];
        save_targets(&path, &targets).unwrap();
        assert_eq!(load_targets(&path).unwrap().len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn remote_target_rejects_unconfirmed_http_and_embedded_credentials() {
        assert!(validate_endpoint("http://cloud.example", false).is_err());
        assert!(validate_endpoint("https://user:password@cloud.example", false).is_err());
        assert!(validate_endpoint("http://127.0.0.1:8080", false).is_ok());
    }
}
