//! Versioned, credential-free Panda Reader backup archives.
//!
//! This crate owns the on-disk protocol and validates archives before callers
//! apply their contents. App-specific configuration is passed as already
//! allowlisted JSON; secrets must never be passed here.

use anyhow::{Context, ensure};
use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, backup::Backup};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

mod config;
mod crypto;
mod storage;
pub use config::{safe_preferences, safe_provider_hints, safe_translator_config};
pub use crypto::{decrypt_file, encrypt_file, is_age_encrypted_file};
pub use storage::{
    BackupItem, BackupTargetConfig, RemoteStorage, S3Target, WebDavTarget, load_targets,
    save_targets,
};

const FORMAT: &str = "panda-reader-backup";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_CONFIG_ITEM_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_ITEMS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupMode {
    Config,
    Full,
}

impl BackupMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: String,
    pub format_version: u32,
    pub mode: BackupMode,
    pub created_at: String,
    pub app_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<i64>,
    pub items: Vec<ManifestItem>,
    pub record_counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestItem {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct BackupPayload {
    pub preferences: Vec<u8>,
    pub translator_public: Vec<u8>,
    pub provider_hints: Vec<u8>,
    pub local_feeds: Option<Vec<u8>>,
    pub database: Option<PathBuf>,
    pub app_version: String,
    pub record_counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug)]
pub struct ValidatedArchive {
    pub manifest: BackupManifest,
    pub files: BTreeMap<String, Vec<u8>>,
    /// Full database is extracted to a private temporary directory to avoid
    /// loading large libraries into memory.
    pub database: Option<PathBuf>,
    _staging: Option<Arc<tempfile::TempDir>>,
}

/// Create a consistent independent SQLite snapshot using SQLite's online backup API.
pub fn create_sqlite_snapshot(source: &Path, destination: &Path) -> anyhow::Result<i64> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let source = Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(std::time::Duration::from_secs(10))?;
    let schema_version = source.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    let mut target = Connection::open(destination)?;
    target.busy_timeout(std::time::Duration::from_secs(10))?;
    {
        let backup = Backup::new(&source, &mut target)?;
        backup.run_to_completion(128, std::time::Duration::from_millis(25), None)?;
    }
    validate_sqlite(destination)?;
    Ok(schema_version)
}

pub fn validate_sqlite(path: &Path) -> anyhow::Result<()> {
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(
        integrity == "ok",
        "backup database failed SQLite integrity check"
    );
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    ensure!(
        rows.next()?.is_none(),
        "backup database has foreign key violations"
    );
    Ok(())
}

/// Keep restored provider caches available offline while forcing a clean
/// authenticated refresh before any queued marks or old cursors are reused.
pub fn reset_provider_sync_state(path: &Path) -> anyhow::Result<()> {
    let mut connection = Connection::open(path)?;
    let transaction = connection.transaction()?;
    let has_pending: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='pending_remote_marks')", [], |row| row.get(0))?;
    if has_pending {
        transaction.execute("DELETE FROM pending_remote_marks", [])?;
    }
    let has_remote: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='remote_state')",
        [],
        |row| row.get(0),
    )?;
    if has_remote {
        for column in ["sync_cursor", "last_sync_at", "last_full_sync_at"] {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('remote_state') WHERE name=?1)",
                [column],
                |row| row.get(0),
            )?;
            if exists {
                transaction.execute(&format!("UPDATE remote_state SET {column}=NULL"), [])?;
            }
        }
    }
    transaction.commit()?;
    validate_sqlite(path)
}

/// Build a ZIP archive using only the version 1 file allowlist.
pub fn create_archive(
    destination: &Path,
    payload: BackupPayload,
) -> anyhow::Result<BackupManifest> {
    let mode = if payload.database.is_some() {
        BackupMode::Full
    } else {
        BackupMode::Config
    };
    ensure!(
        (mode == BackupMode::Config) == payload.local_feeds.is_some(),
        "config archive must include feeds and full archive must not"
    );
    let mut files = BTreeMap::new();
    files.insert("preferences.json".to_string(), payload.preferences);
    files.insert(
        "translator-public.json".to_string(),
        payload.translator_public,
    );
    files.insert("provider-hints.json".to_string(), payload.provider_hints);
    if let Some(feeds) = payload.local_feeds {
        files.insert("local-feeds.json".to_string(), feeds);
    }
    let mut total_bytes = 0_u64;
    let mut items: Vec<ManifestItem> = files
        .iter()
        .map(|(name, bytes)| {
            total_bytes += bytes.len() as u64;
            ManifestItem {
                name: name.clone(),
                bytes: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
            }
        })
        .collect();
    let schema_version = if let Some(path) = &payload.database {
        validate_sqlite(path)?;
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let (size, digest) = hash_file(path)?;
        total_bytes = total_bytes.saturating_add(size);
        ensure!(
            total_bytes <= MAX_ARCHIVE_BYTES,
            "backup exceeds size limit"
        );
        items.push(ManifestItem {
            name: "library.sqlite3".into(),
            bytes: size,
            sha256: digest,
        });
        Some(version)
    } else {
        None
    };
    ensure!(
        total_bytes <= MAX_ARCHIVE_BYTES,
        "backup exceeds size limit"
    );
    let manifest = BackupManifest {
        format: FORMAT.into(),
        format_version: 1,
        mode,
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        app_version: payload.app_version,
        schema_version,
        items,
        record_counts: payload.record_counts,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    let file = temp.as_file().try_clone()?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o600);
    zip.start_file("manifest.json", options)?;
    zip.write_all(&manifest_bytes)?;
    for (name, bytes) in files {
        zip.start_file(name, options)?;
        zip.write_all(&bytes)?;
    }
    if let Some(database) = payload.database {
        zip.start_file("library.sqlite3", options)?;
        std::io::copy(&mut File::open(database)?, &mut zip)?;
    }
    zip.finish()?.sync_all()?;
    temp.as_file().sync_all()?;
    temp.persist(destination).map_err(|error| error.error)?;
    Ok(manifest)
}

/// Read and validate every member, size and digest before returning any file.
pub fn read_archive(path: &Path) -> anyhow::Result<ValidatedArchive> {
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() <= MAX_ARCHIVE_BYTES,
        "backup archive exceeds size limit"
    );
    let mut zip = ZipArchive::new(file).context("invalid backup ZIP")?;
    ensure!(
        !zip.is_empty() && zip.len() <= MAX_ITEMS,
        "backup contains an invalid number of files"
    );
    let mut files = BTreeMap::new();
    let mut digests = BTreeMap::new();
    let mut database = None;
    let staging = Arc::new(tempfile::tempdir()?);
    let mut expanded_bytes = 0_u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_string();
        ensure!(
            valid_member_name(&name),
            "backup contains an unsafe file path"
        );
        ensure!(entry.is_file(), "backup contains a directory entry");
        if let Some(mode) = entry.unix_mode() {
            ensure!(
                mode & 0o170000 != 0o120000,
                "backup contains a symbolic link"
            );
        }
        let per_item_limit = if name == "library.sqlite3" {
            MAX_ARCHIVE_BYTES
        } else {
            MAX_CONFIG_ITEM_BYTES
        };
        ensure!(
            entry.size() <= per_item_limit,
            "backup member exceeds size limit"
        );
        expanded_bytes = expanded_bytes.saturating_add(entry.size());
        ensure!(
            expanded_bytes <= MAX_ARCHIVE_BYTES,
            "expanded backup exceeds size limit"
        );
        ensure!(
            !files.contains_key(&name),
            "backup contains duplicate file names"
        );
        let expected_size = entry.size();
        if name == "library.sqlite3" {
            let path = staging.path().join("library.sqlite3");
            let mut output = File::create(&path)?;
            let (size, digest) = copy_hash(&mut entry, &mut output)?;
            output.sync_all()?;
            ensure!(size == expected_size, "backup member length mismatch");
            digests.insert(name, (size, digest));
            database = Some(path);
        } else {
            let mut bytes = Vec::with_capacity(expected_size as usize);
            (&mut entry)
                .take(MAX_CONFIG_ITEM_BYTES + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 == expected_size,
                "backup member length mismatch"
            );
            digests.insert(
                name.clone(),
                (expected_size, hex::encode(Sha256::digest(&bytes))),
            );
            files.insert(name, bytes);
        }
    }
    let manifest_bytes = files
        .remove("manifest.json")
        .context("backup manifest is missing")?;
    ensure!(
        manifest_bytes.len() as u64 <= MAX_MANIFEST_BYTES,
        "backup manifest is too large"
    );
    let manifest: BackupManifest =
        serde_json::from_slice(&manifest_bytes).context("invalid backup manifest")?;
    ensure!(
        manifest.format == FORMAT && manifest.format_version == 1,
        "unsupported backup format version"
    );
    let expected: Vec<&str> = match manifest.mode {
        BackupMode::Config => vec![
            "local-feeds.json",
            "preferences.json",
            "translator-public.json",
            "provider-hints.json",
        ],
        BackupMode::Full => vec![
            "library.sqlite3",
            "preferences.json",
            "translator-public.json",
            "provider-hints.json",
        ],
    };
    let mut actual: Vec<&str> = files.keys().map(String::as_str).collect();
    if database.is_some() {
        actual.push("library.sqlite3");
    }
    actual.sort();
    let mut expected_sorted = expected.clone();
    expected_sorted.sort();
    ensure!(
        actual == expected_sorted,
        "backup files do not match the selected mode"
    );
    ensure!(
        manifest.items.len() == expected.len(),
        "manifest file list does not match archive"
    );
    let manifest_names: std::collections::BTreeSet<&str> = manifest
        .items
        .iter()
        .map(|item| item.name.as_str())
        .collect();
    ensure!(
        manifest_names.len() == manifest.items.len()
            && manifest_names
                .iter()
                .copied()
                .eq(expected_sorted.iter().copied()),
        "manifest file list does not match archive"
    );
    for item in &manifest.items {
        let (size, digest) = digests
            .get(&item.name)
            .context("manifest references missing file")?;
        ensure!(
            *size == item.bytes,
            "backup member length does not match manifest"
        );
        ensure!(*digest == item.sha256, "backup member checksum mismatch");
    }
    if manifest.mode == BackupMode::Full {
        // Validated callers can write this file to an isolated staging directory and
        // invoke validate_sqlite there before applying startup replacement.
        let database = database
            .as_deref()
            .context("Full backup database is missing")?;
        let declared = manifest
            .schema_version
            .context("full backup lacks schema version")?;
        let connection =
            Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let actual: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        ensure!(
            declared == actual,
            "backup manifest schema version does not match database"
        );
        validate_sqlite(database)?;
    } else {
        ensure!(
            manifest.schema_version.is_none(),
            "Config Only backup must not declare a database schema"
        );
    }
    let staging = database.as_ref().map(|_| staging);
    Ok(ValidatedArchive {
        manifest,
        files,
        database,
        _staging: staging,
    })
}

fn copy_hash(reader: &mut impl Read, writer: &mut impl Write) -> anyhow::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .context("backup length overflow")?;
        ensure!(
            total <= MAX_ARCHIVE_BYTES,
            "expanded backup exceeds size limit"
        );
        writer.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
    }
    Ok((total, hex::encode(hasher.finalize())))
}

fn valid_member_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && Path::new(name)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
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

pub fn suggested_filename(mode: BackupMode) -> String {
    format!(
        "panda-reader-{}-{}-{}.pdbk",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        Uuid::new_v4(),
        mode.label()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn allowlists_remove_credentials_and_portable_source_state() {
        let prefs = safe_preferences(
            &json!({"theme":"bamboo", "library_source":"freshrss", "last_refresh_at":"yesterday", "surprise_secret":"x"}),
        );
        assert_eq!(prefs["library_source"], "local");
        assert!(prefs["last_refresh_at"].is_null());
        assert!(prefs.get("surprise_secret").is_none());

        let translator = safe_translator_config(
            &json!({"provider":"openai_compatible", "openai_url":"https://user:pass@api.example/v1?key=secret#x", "openai_key":"secret", "azure_key":"secret"}),
        );
        assert!(translator.get("openai_key").is_none());
        assert!(translator.get("azure_key").is_none());
        assert_eq!(translator["openai_url"], "https://api.example/v1");
        let hints = safe_provider_hints(
            &json!({"miniflux":{"endpoint":"https://user:pass@feed.example?token=abc","username":"reader","secret":"token"}}),
        );
        assert!(hints["miniflux"].get("secret").is_none());
        assert_eq!(hints["miniflux"]["endpoint"], "https://feed.example");
    }

    #[test]
    fn config_archive_round_trips_and_rejects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.prbackup");
        create_archive(
            &path,
            BackupPayload {
                preferences: br#"{"theme":"bamboo"}"#.to_vec(),
                translator_public: b"{}".to_vec(),
                provider_hints: b"{}".to_vec(),
                local_feeds: Some(b"[]".to_vec()),
                database: None,
                app_version: "0.2.11".into(),
                record_counts: BTreeMap::new(),
            },
        )
        .unwrap();
        let archive = read_archive(&path).unwrap();
        assert_eq!(archive.manifest.mode, BackupMode::Config);
        assert_eq!(archive.files["local-feeds.json"], b"[]");

        let mut manifest = archive.manifest;
        manifest.items[0].sha256 = "0".repeat(64);
        let file = File::create(&path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        for (name, bytes) in archive.files {
            zip.start_file(name, options).unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
        assert!(read_archive(&path).is_err());
    }

    #[test]
    fn full_snapshot_archive_streams_and_verifies_database() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.sqlite3");
        let snapshot = dir.path().join("snapshot.sqlite3");
        let connection = Connection::open(&source).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection.execute_batch("CREATE TABLE articles(id INTEGER PRIMARY KEY, body TEXT NOT NULL); INSERT INTO articles(body) VALUES('snapshot'); PRAGMA user_version=1;").unwrap();
        create_sqlite_snapshot(&source, &snapshot).unwrap();
        let db = Connection::open_with_flags(&snapshot, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
        let body: String = db
            .query_row("SELECT body FROM articles", [], |row| row.get(0))
            .unwrap();
        assert_eq!(body, "snapshot");

        let archive_path = dir.path().join("full.prbackup");
        create_archive(
            &archive_path,
            BackupPayload {
                preferences: b"{}".to_vec(),
                translator_public: b"{}".to_vec(),
                provider_hints: b"{}".to_vec(),
                local_feeds: None,
                database: Some(snapshot),
                app_version: "0.2.11".into(),
                record_counts: BTreeMap::new(),
            },
        )
        .unwrap();
        let restored = read_archive(&archive_path).unwrap();
        assert_eq!(restored.manifest.mode, BackupMode::Full);
        assert!(restored.database.as_ref().is_some_and(|path| path.exists()));
    }

    #[test]
    fn full_restore_invalidates_provider_cursor_without_dropping_cache_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provider.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE remote_state(workspace TEXT PRIMARY KEY, account TEXT, sync_cursor TEXT, last_sync_at TEXT, last_full_sync_at TEXT); INSERT INTO remote_state VALUES('provider:miniflux','old-account','cursor','now','then'); CREATE TABLE pending_remote_marks(workspace TEXT, remote_id INTEGER); INSERT INTO pending_remote_marks VALUES('provider:miniflux',1); CREATE TABLE cached_articles(id INTEGER PRIMARY KEY); INSERT INTO cached_articles VALUES(1);").unwrap();
        drop(connection);
        reset_provider_sync_state(&path).unwrap();
        let connection = Connection::open(&path).unwrap();
        let cursor: Option<String> = connection
            .query_row("SELECT sync_cursor FROM remote_state", [], |row| row.get(0))
            .unwrap();
        let marks: i64 = connection
            .query_row("SELECT count(*) FROM pending_remote_marks", [], |row| {
                row.get(0)
            })
            .unwrap();
        let cache: i64 = connection
            .query_row("SELECT count(*) FROM cached_articles", [], |row| row.get(0))
            .unwrap();
        assert!(cursor.is_none());
        assert_eq!(marks, 0);
        assert_eq!(cache, 1);
    }

    #[test]
    fn archive_rejects_path_traversal_before_extracting_member() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traversal.prbackup");
        let file = File::create(&path).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("../outside.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"unsafe").unwrap();
        zip.finish().unwrap();
        assert!(read_archive(&path).is_err());
        assert!(!dir.path().join("outside.json").exists());
    }
}
