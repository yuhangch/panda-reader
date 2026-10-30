//! Application backup and restore operations.

use anyhow::Context as _;
use panda_backup::{BackupMode, BackupPayload};
use panda_store::Store;
use std::collections::BTreeMap;

pub(crate) fn stage_full_restore(
    data_dir: &std::path::Path,
    archive: panda_backup::ValidatedArchive,
) -> anyhow::Result<()> {
    static STAGING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _staging = STAGING
        .try_lock()
        .map_err(|_| anyhow::anyhow!("another restore is being staged"))?;
    anyhow::ensure!(
        !data_dir.join("restore-ready").exists(),
        "a Full restore is already staged; restart Panda Reader first"
    );
    let database = archive
        .database
        .as_deref()
        .context("Full backup database is missing")?;
    let version = archive
        .manifest
        .schema_version
        .context("Full backup schema version is missing")?;
    anyhow::ensure!(
        version <= panda_store::CURRENT_SCHEMA_VERSION,
        "backup database was created by a newer Panda Reader version"
    );
    let db_temp = tempfile::NamedTempFile::new_in(data_dir)?;
    std::fs::copy(database, db_temp.path())?;
    panda_backup::reset_provider_sync_state(db_temp.path())?;
    let preferences = archive
        .files
        .get("preferences.json")
        .context("Full backup preferences are missing")?;
    let preferences_value: serde_json::Value = serde_json::from_slice(preferences)?;
    let mut preferences: crate::app::preferences::Preferences =
        serde_json::from_value(panda_backup::safe_preferences(&preferences_value))?;
    preferences.library_source = crate::app::preferences::LibrarySource::Local;
    preferences.last_refresh_at = None;
    crate::services::ArticleFetchNetwork {
        mode: preferences.article_network_mode,
        proxy_url: preferences.article_proxy_url.clone(),
        trusted_fake_ip_ranges: preferences.trusted_proxy_fake_ip_ranges.clone(),
    }
    .validate()?;
    let pref_temp = tempfile::NamedTempFile::new_in(data_dir)?;
    std::fs::write(pref_temp.path(), serde_json::to_vec_pretty(&preferences)?)?;
    let translator = archive
        .files
        .get("translator-public.json")
        .context("Full backup public translator settings are missing")?;
    let translator: serde_json::Value = serde_json::from_slice(translator)?;
    let translator = panda_backup::safe_translator_config(&translator);
    let translator_temp = tempfile::NamedTempFile::new_in(data_dir)?;
    std::fs::write(
        translator_temp.path(),
        serde_json::to_vec_pretty(&translator)?,
    )?;
    db_temp
        .persist(data_dir.join("panda-reader.sqlite3.restore-pending"))
        .map_err(|error| error.error)?;
    pref_temp
        .persist(data_dir.join("preferences.json.restore-pending"))
        .map_err(|error| error.error)?;
    translator_temp
        .persist(data_dir.join("translator-public.json.restore-pending"))
        .map_err(|error| error.error)?;
    let ready_temp = tempfile::NamedTempFile::new_in(data_dir)?;
    std::fs::write(ready_temp.path(), b"v1")?;
    ready_temp
        .persist(data_dir.join("restore-ready"))
        .map_err(|error| error.error)?;
    Ok(())
}

pub(crate) fn write_backup_archive(
    data_dir: &std::path::Path,
    mode: BackupMode,
    destination: &std::path::Path,
    preferences: Vec<u8>,
    translator: Vec<u8>,
    provider_hints: Vec<u8>,
    scratch: &std::path::Path,
) -> anyhow::Result<panda_backup::BackupManifest> {
    let db = data_dir.join("panda-reader.sqlite3");
    let (local_feeds, database, record_counts) = match mode {
        BackupMode::Config => {
            let store = Store::open_read_workspace(&db, "local")?;
            let feeds = store.export_local_backup_feeds()?;
            let count = feeds.len() as u64;
            (
                Some(serde_json::to_vec_pretty(&feeds)?),
                None,
                BTreeMap::from([("local_feeds".into(), count)]),
            )
        }
        BackupMode::Full => {
            let snapshot = scratch.join("library.sqlite3");
            panda_backup::create_sqlite_snapshot(&db, &snapshot)?;
            let connection = rusqlite::Connection::open_with_flags(
                &snapshot,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let mut counts = BTreeMap::new();
            for table in [
                "feeds",
                "articles",
                "article_reading_positions",
                "article_translation_segments",
            ] {
                let exists: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                    [table],
                    |row| row.get(0),
                )?;
                if exists {
                    let count: i64 = connection.query_row(
                        &format!("SELECT count(*) FROM {table}"),
                        [],
                        |row| row.get(0),
                    )?;
                    counts.insert(table.to_owned(), count as u64);
                }
            }
            (None, Some(snapshot), counts)
        }
    };
    Ok(panda_backup::create_archive(
        destination,
        BackupPayload {
            preferences,
            translator_public: translator,
            provider_hints,
            local_feeds,
            database,
            app_version: env!("CARGO_PKG_VERSION").into(),
            record_counts,
        },
    )?)
}
