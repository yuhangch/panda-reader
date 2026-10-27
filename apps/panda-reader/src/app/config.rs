//! Paths, persisted settings, and startup migration.

use super::preferences::{LibrarySource, Preferences};
use directories::ProjectDirs;
use panda_providers::ProviderSettingsMap;
use panda_translate::TranslatorConfig;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::fs::File;
use std::path::PathBuf;

pub(super) struct AppConfig {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub preferences_path: PathBuf,
    pub provider_settings_path: PathBuf,
    pub translator_path: PathBuf,
    pub preferences: Preferences,
    pub provider_settings: ProviderSettingsMap,
    pub translator_config: TranslatorConfig,
    pub(super) _instance_lock: Option<InstanceLock>,
}

fn data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("PANDA_READER_DATA_DIR").filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    ProjectDirs::from("com", "PandaReader", "PandaReader")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("data"))
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let data_dir = data_dir();
        std::fs::create_dir_all(&data_dir)?;
        let lock = InstanceLock::acquire(&data_dir)?;
        let mut config = Self::load_from(data_dir);
        config._instance_lock = Some(lock);
        Ok(config)
    }

    pub(super) fn load_from(data_dir: PathBuf) -> Self {
        let database = data_dir.join("panda-reader.sqlite3");
        apply_pending_restore(&data_dir, &database);
        let preferences_path = data_dir.join("preferences.json");
        let provider_settings_path = data_dir.join("providers.json");
        let legacy_credentials_path = data_dir.join("miniflux.json");
        let translator_path = data_dir.join("translator.json");
        let mut preferences = Preferences::load(&preferences_path);
        let has_saved_source = std::fs::read(&preferences_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|value| value.get("library_source").is_some());
        let mut provider_settings = panda_providers::load_settings(&provider_settings_path)
            .unwrap_or_else(|error| {
                eprintln!("could not read provider settings: {error}");
                panda_providers::ProviderSettingsMap::new()
            });
        if let Ok(Some(legacy)) = panda_miniflux::Connection::load(&legacy_credentials_path) {
            provider_settings
                .entry(panda_providers::ProviderKind::Miniflux)
                .or_insert_with(|| panda_providers::ProviderSettings {
                    endpoint: legacy.endpoint,
                    username: String::new(),
                    secret: legacy.token,
                });
            if !has_saved_source {
                preferences.library_source = LibrarySource::Miniflux;
            }
            let settings_saved =
                match panda_providers::save_settings(&provider_settings_path, &provider_settings) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("could not migrate Miniflux settings: {error}");
                        false
                    }
                };
            let preferences_saved = if has_saved_source {
                true
            } else {
                preferences.library_source = LibrarySource::Miniflux;
                match preferences.save(&preferences_path) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("could not save migrated library selection: {error}");
                        false
                    }
                }
            };
            if settings_saved
                && preferences_saved
                && let Err(error) = std::fs::remove_file(&legacy_credentials_path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("could not remove migrated Miniflux settings: {error}");
            }
        }
        let translator_config = panda_translate::TranslatorConfig::load(&translator_path)
            .unwrap_or_else(|error| {
                eprintln!("could not read translator settings: {error}");
                panda_translate::TranslatorConfig::default()
            });
        Self {
            data_dir,
            database,
            preferences_path,
            provider_settings_path,
            translator_path,
            preferences,
            provider_settings,
            translator_config,
            _instance_lock: None,
        }
    }
}

/// Keep one process responsible for a data directory from before startup
/// restore through shutdown. The OS lock is automatically released if the
/// process exits unexpectedly.
pub(super) struct InstanceLock {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(unix)]
    file: File,
}

impl InstanceLock {
    fn acquire(data_dir: &std::path::Path) -> anyhow::Result<Self> {
        let canonical = std::fs::canonicalize(data_dir)?;
        let digest = Sha256::digest(canonical.to_string_lossy().as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::{
                Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, SetLastError},
                System::Threading::CreateMutexW,
            };

            let name: Vec<u16> = std::ffi::OsStr::new(&format!("Local\\PandaReader-{digest}"))
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: `name` is NUL-terminated and remains alive during the call.
            unsafe { SetLastError(0) };
            let handle: HANDLE = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
            anyhow::ensure!(
                !handle.is_null(),
                "could not acquire the Panda Reader data lock"
            );
            // CreateMutexW grants ownership to this process when it creates the
            // mutex. Existing mutexes are created without initial ownership.
            let error = unsafe { GetLastError() };
            if error == ERROR_ALREADY_EXISTS {
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(handle);
                }
                anyhow::bail!("another Panda Reader instance is using this data directory");
            }
            Ok(Self { handle })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let path = data_dir.join(format!(".panda-reader-{digest}.lock"));
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .mode(0o600)
                .open(path)?;
            // SAFETY: `file` owns a valid descriptor for the duration of the
            // lock and remains in `InstanceLock` until application shutdown.
            let result = unsafe {
                libc::flock(
                    std::os::fd::AsRawFd::as_raw_fd(&file),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            };
            anyhow::ensure!(
                result == 0,
                "another Panda Reader instance is using this data directory"
            );
            Ok(Self { file })
        }
    }
}

#[cfg(windows)]
impl Drop for InstanceLock {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(unix)]
impl Drop for InstanceLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn apply_pending_restore(data_dir: &std::path::Path, database: &std::path::Path) {
    let ready = data_dir.join("restore-ready");
    let pending = data_dir.join("panda-reader.sqlite3.restore-pending");
    let prefs_pending = data_dir.join("preferences.json.restore-pending");
    let translator_pending = data_dir.join("translator-public.json.restore-pending");
    if !ready.exists() {
        let _ = std::fs::remove_file(&pending);
        let _ = std::fs::remove_file(&prefs_pending);
        let _ = std::fs::remove_file(&translator_pending);
        return;
    }
    if !pending.exists() {
        if let Err(error) = apply_pending_preferences(data_dir) {
            eprintln!("could not finish restored Panda Reader preferences: {error:#}");
            return;
        }
        if let Err(error) = apply_pending_public_translator(data_dir) {
            eprintln!("could not finish restored Panda Reader translator settings: {error:#}");
            return;
        }
        let _ = std::fs::remove_file(ready);
        return;
    }
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let backup = data_dir.join(format!("panda-reader.sqlite3.pre-restore-{stamp}.bak"));
    let prefs = data_dir.join("preferences.json");
    let prefs_backup = data_dir.join(format!("preferences.json.pre-restore-{stamp}.bak"));

    let result = (|| -> anyhow::Result<()> {
        panda_backup::reset_provider_sync_state(&pending)?;
        let schema = rusqlite::Connection::open_with_flags(
            &pending,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
        anyhow::ensure!(
            schema <= panda_store::CURRENT_SCHEMA_VERSION,
            "restore database schema is newer than this app"
        );
        if database.exists() {
            panda_backup::create_sqlite_snapshot(database, &backup)?;
        }
        if prefs.exists() {
            std::fs::copy(&prefs, &prefs_backup)?;
        }
        for suffix in ["-wal", "-shm"] {
            let sidecar = std::path::PathBuf::from(format!("{}{suffix}", database.display()));
            if sidecar.exists() {
                let sidecar_backup =
                    std::path::PathBuf::from(format!("{}{suffix}", backup.display()));
                std::fs::rename(sidecar, sidecar_backup)?;
            }
        }
        if database.exists() {
            std::fs::remove_file(database)?;
        }
        if let Err(error) = std::fs::rename(&pending, database) {
            if backup.exists() {
                let _ = std::fs::copy(&backup, database);
            }
            return Err(error.into());
        }
        if prefs_pending.exists()
            && let Err(error) = std::fs::rename(&prefs_pending, &prefs)
        {
            let failed = data_dir.join("panda-reader.sqlite3.restore-failed");
            let _ = std::fs::rename(database, failed);
            if backup.exists() {
                let _ = std::fs::copy(&backup, database);
            }
            if prefs_backup.exists() {
                let _ = std::fs::copy(&prefs_backup, &prefs);
            }
            let _ = std::fs::remove_file(&prefs_pending);
            let _ = std::fs::remove_file(&translator_pending);
            let _ = std::fs::remove_file(&ready);
            return Err(error.into());
        }
        apply_pending_public_translator(data_dir)?;
        std::fs::remove_file(&ready)?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("could not apply staged Panda Reader restore: {error:#}");
    }
}

fn apply_pending_preferences(data_dir: &std::path::Path) -> anyhow::Result<()> {
    let pending = data_dir.join("preferences.json.restore-pending");
    if !pending.exists() {
        return Ok(());
    }
    let path = data_dir.join("preferences.json");
    let bytes = std::fs::read(&pending)?;
    let temp = tempfile::NamedTempFile::new_in(data_dir)?;
    std::fs::write(temp.path(), bytes)?;
    temp.persist(&path).map_err(|error| error.error)?;
    std::fs::remove_file(pending)?;
    Ok(())
}

fn apply_pending_public_translator(data_dir: &std::path::Path) -> anyhow::Result<()> {
    let pending = data_dir.join("translator-public.json.restore-pending");
    if !pending.exists() {
        return Ok(());
    }
    let path = data_dir.join("translator.json");
    (|| -> anyhow::Result<()> {
        let mut current = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
            Err(error) => return Err(error.into()),
        };
        let public: serde_json::Value = serde_json::from_slice(&std::fs::read(&pending)?)?;
        let current = current
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("translator settings must be a JSON object"))?;
        if let Some(public) = public.as_object() {
            for key in [
                "provider",
                "azure_region",
                "openai_url",
                "openai_model",
                "anthropic_url",
                "anthropic_model",
                "gemini_url",
                "gemini_model",
            ] {
                if let Some(value) = public.get(key) {
                    current.insert(key.into(), value.clone());
                }
            }
        }
        let temp = tempfile::NamedTempFile::new_in(data_dir)?;
        std::fs::write(temp.path(), serde_json::to_vec_pretty(&current)?)?;
        temp.persist(&path).map_err(|error| error.error)?;
        std::fs::remove_file(pending)?;
        Ok(())
    })()
}

#[cfg(test)]
mod backup_restore_tests {
    use super::*;

    #[test]
    fn data_directory_lock_rejects_a_second_instance() {
        let dir = tempfile::tempdir().unwrap();
        let first = InstanceLock::acquire(dir.path()).unwrap();
        assert!(InstanceLock::acquire(dir.path()).is_err());
        drop(first);
        assert!(InstanceLock::acquire(dir.path()).is_ok());
    }

    #[test]
    fn staged_full_restore_replaces_database_and_preserves_device_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("panda-reader.sqlite3");
        let current = rusqlite::Connection::open(&database).unwrap();
        current
            .execute_batch(
                "CREATE TABLE old_data(value TEXT); INSERT INTO old_data VALUES('keep-backup');",
            )
            .unwrap();
        drop(current);
        let staged =
            rusqlite::Connection::open(dir.path().join("panda-reader.sqlite3.restore-pending"))
                .unwrap();
        staged.execute_batch("CREATE TABLE restored_data(value TEXT); INSERT INTO restored_data VALUES('restored'); CREATE TABLE remote_state(workspace TEXT PRIMARY KEY,account TEXT,sync_cursor TEXT,last_sync_at TEXT,last_full_sync_at TEXT); INSERT INTO remote_state VALUES('provider:miniflux','old','cursor','now','then'); CREATE TABLE pending_remote_marks(workspace TEXT,remote_id INTEGER); INSERT INTO pending_remote_marks VALUES('provider:miniflux',1); PRAGMA user_version=1;").unwrap();
        drop(staged);
        std::fs::write(dir.path().join("preferences.json"), b"{\"theme\":\"old\"}").unwrap();
        std::fs::write(
            dir.path().join("preferences.json.restore-pending"),
            b"{\"theme\":\"new\",\"library_source\":\"local\"}",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("translator.json"),
            b"{\"provider\":\"azure\",\"azure_key\":\"device-secret\",\"azure_region\":\"old\"}",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("translator-public.json.restore-pending"),
            b"{\"provider\":\"openai_compatible\",\"openai_model\":\"model-x\"}",
        )
        .unwrap();
        std::fs::write(dir.path().join("restore-ready"), b"v1").unwrap();

        apply_pending_restore(dir.path(), &database);

        let restored = rusqlite::Connection::open(&database).unwrap();
        let value: String = restored
            .query_row("SELECT value FROM restored_data", [], |row| row.get(0))
            .unwrap();
        let marks: i64 = restored
            .query_row("SELECT count(*) FROM pending_remote_marks", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(value, "restored");
        assert_eq!(marks, 0);
        let prefs: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("preferences.json")).unwrap())
                .unwrap();
        assert_eq!(prefs["theme"], "new");
        let translator: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("translator.json")).unwrap())
                .unwrap();
        assert_eq!(translator["azure_key"], "device-secret");
        assert_eq!(translator["provider"], "openai_compatible");
        assert_eq!(translator["openai_model"], "model-x");
        assert!(!dir.path().join("restore-ready").exists());
        assert!(std::fs::read_dir(dir.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("pre-restore-")
        }));
    }

    #[test]
    fn resumed_restore_atomically_replaces_existing_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("panda-reader.sqlite3");
        std::fs::write(dir.path().join("restore-ready"), b"v1").unwrap();
        std::fs::write(dir.path().join("preferences.json"), b"{\"theme\":\"old\"}").unwrap();
        std::fs::write(
            dir.path().join("preferences.json.restore-pending"),
            b"{\"theme\":\"new\"}",
        )
        .unwrap();

        apply_pending_restore(dir.path(), &database);

        let preferences: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("preferences.json")).unwrap())
                .unwrap();
        assert_eq!(preferences["theme"], "new");
        assert!(!dir.path().join("preferences.json.restore-pending").exists());
        assert!(!dir.path().join("restore-ready").exists());
    }
}
