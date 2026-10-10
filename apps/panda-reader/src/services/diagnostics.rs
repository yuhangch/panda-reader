use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Mutex, OnceLock};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const LOG_RETENTION_OPTIONS: [u16; 3] = [7, 30, 90];
static LOG_RETENTION_DAYS: AtomicU16 = AtomicU16::new(30);
static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Copy)]
pub(super) enum LogDetail {
    Summary,
    Detailed,
}

pub(crate) fn log_path(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("sync.log")
}

pub(crate) fn set_log_retention_days(days: u16, database_path: &Path) {
    let days = if LOG_RETENTION_OPTIONS.contains(&days) {
        days
    } else {
        30
    };
    LOG_RETENTION_DAYS.store(days, Ordering::Relaxed);
    let _guard = LOG_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cleanup_expired_logs(database_path, days);
}

pub(super) fn write_sync_log(
    database_path: &Path,
    detailed_enabled: &AtomicBool,
    detail: LogDetail,
    level: &str,
    message: impl AsRef<str>,
) {
    if matches!(detail, LogDetail::Detailed) && !detailed_enabled.load(Ordering::Relaxed) {
        return;
    }

    let line = format!(
        "{} [{level}] {}\n",
        chrono::Utc::now().to_rfc3339(),
        redact_urls(message.as_ref())
    );
    let path = log_path(database_path);
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(parent) {
        eprintln!("could not create diagnostics log directory: {error}");
        return;
    }

    let _guard = LOG_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cleanup_expired_logs(database_path, LOG_RETENTION_DAYS.load(Ordering::Relaxed));
    if std::fs::metadata(&path)
        .is_ok_and(|metadata| metadata.len().saturating_add(line.len() as u64) > MAX_LOG_BYTES)
    {
        let previous = path.with_extension("log.1");
        let _ = std::fs::remove_file(&previous);
        if let Err(error) = std::fs::rename(&path, &previous) {
            eprintln!("could not rotate diagnostics log: {error}");
        }
    }
    if let Err(error) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))
    {
        eprintln!("could not write diagnostics log: {error}");
    }
}

fn cleanup_expired_logs(database_path: &Path, retention_days: u16) {
    cleanup_expired_logs_at(database_path, retention_days, std::time::SystemTime::now());
}

fn cleanup_expired_logs_at(database_path: &Path, retention_days: u16, now: std::time::SystemTime) {
    let current = log_path(database_path);
    let previous = current.with_extension("log.1");
    let cutoff = now.checked_sub(std::time::Duration::from_secs(
        u64::from(retention_days) * 24 * 60 * 60,
    ));
    let Some(cutoff) = cutoff else {
        return;
    };
    for path in [&current, &previous] {
        let expired = std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified < cutoff);
        if expired {
            if let Err(error) = std::fs::remove_file(path) {
                eprintln!("could not remove expired diagnostics log: {error}");
            }
        }
    }
}

fn redact_urls(message: &str) -> String {
    let mut output = String::with_capacity(message.len());
    let mut rest = message;
    while let Some((start, prefix_len)) = find_url_start(rest) {
        output.push_str(&rest[..start]);
        let url_start = start + prefix_len;
        let end = rest[url_start..]
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '\'' | '"' | '<' | '>'))
            .map_or(rest.len(), |offset| url_start + offset);
        let token = &rest[start..end];
        let trimmed_token = token.trim_end_matches(['.', ',', ';', ')', ']', '}']);
        let (candidate, suffix) = token.split_at(trimmed_token.len());
        if let Ok(mut url) = url::Url::parse(candidate) {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            output.push_str(url.as_str());
        } else {
            output.push_str(candidate);
        }
        output.push_str(suffix);
        rest = &rest[end..];
    }
    output.push_str(rest);
    output
}

fn find_url_start(value: &str) -> Option<(usize, usize)> {
    let http = value.find("http://").map(|index| (index, 7));
    let https = value.find("https://").map(|index| (index, 8));
    match (http, https) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn detailed_messages_respect_the_live_switch() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("reader.sqlite3");
        let enabled = AtomicBool::new(false);
        write_sync_log(
            &database,
            &enabled,
            LogDetail::Detailed,
            "INFO",
            "page fetch_ms=12",
        );
        assert!(!log_path(&database).exists());

        enabled.store(true, Ordering::Relaxed);
        write_sync_log(
            &database,
            &enabled,
            LogDetail::Detailed,
            "INFO",
            "page fetch_ms=12",
        );
        assert!(
            std::fs::read_to_string(log_path(&database))
                .unwrap()
                .contains("page fetch_ms=12")
        );
    }

    #[test]
    fn summary_messages_are_written_when_detailed_logging_is_off() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("reader.sqlite3");
        write_sync_log(
            &database,
            &AtomicBool::new(false),
            LogDetail::Summary,
            "ERROR",
            "sync failed",
        );
        assert!(
            std::fs::read_to_string(log_path(&database))
                .unwrap()
                .contains("sync failed")
        );
    }

    #[test]
    fn log_redacts_url_credentials_query_and_fragment() {
        let redacted = redact_urls(
            "request failed https://reader:secret@example.com/api?token=abc#frag, try later",
        );
        assert_eq!(
            redacted,
            "request failed https://example.com/api, try later"
        );
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("token"));
    }

    #[test]
    fn log_rotation_keeps_one_previous_file() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("reader.sqlite3");
        let path = log_path(&database);
        std::fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
        write_sync_log(
            &database,
            &AtomicBool::new(false),
            LogDetail::Summary,
            "INFO",
            "new log",
        );
        assert!(path.with_extension("log.1").is_file());
        assert!(std::fs::read_to_string(path).unwrap().contains("new log"));
    }

    #[test]
    fn retention_cleanup_only_removes_sync_logs_older_than_the_limit() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("reader.sqlite3");
        let current = log_path(&database);
        let previous = current.with_extension("log.1");
        std::fs::write(&current, "current").unwrap();
        std::fs::write(&previous, "rotated").unwrap();
        let now = std::time::SystemTime::now() + std::time::Duration::from_secs(31 * 24 * 60 * 60);

        cleanup_expired_logs_at(&database, 30, now);

        assert!(!current.exists());
        assert!(!previous.exists());
    }
}
