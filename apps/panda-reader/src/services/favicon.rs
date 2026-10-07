use super::worker::{WorkerState, job, lock_mutex};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::oneshot;

use std::path::Path;

static FAVICON_LIMIT: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

pub fn sanitize_host(host: &str) -> String {
    host.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub async fn fetch_favicon(
    host: &str,
    site_url: &str,
    icons_dir: &Path,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(icons_dir).map_err(|e| e.to_string())?;
    let safe = sanitize_host(host);
    let path = icons_dir.join(format!("{safe}.png"));
    if path.is_file() {
        return Ok(path);
    }
    let url = format!("https://www.google.com/s2/favicons?domain={host}&sz=64");
    let _ = site_url;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("PandaReader/0.1")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("favicon HTTP client configuration is valid")
    });
    let bytes = client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("empty favicon".into());
    }
    let tmp = icons_dir.join(format!("{safe}.png.part"));
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

pub(super) fn ensure_favicon(
    host: String,
    site_url: String,
    icons_dir: PathBuf,
    reply: oneshot::Sender<Result<PathBuf, String>>,
    state: &WorkerState,
) {
    let inflight = state.favicon_inflight.clone();
    job(reply, move |runtime| {
        let safe = sanitize_host(&host);
        let path = icons_dir.join(format!("{safe}.png"));
        loop {
            let already_running = {
                let mut guard = lock_mutex(&inflight, "favicon in-flight");
                if guard.contains_key(&host) {
                    true
                } else {
                    guard.insert(host.clone(), ());
                    false
                }
            };
            if !already_running {
                break;
            }
            if path.is_file() {
                return Ok(path);
            }
            runtime.block_on(tokio::time::sleep(std::time::Duration::from_millis(40)));
        }
        let semaphore = FAVICON_LIMIT
            .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(6)))
            .clone();
        let request_host = host.clone();
        let request_site_url = site_url.clone();
        let request_icons_dir = icons_dir.clone();
        let result = runtime.block_on(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|error| error.to_string())?;
            fetch_favicon(&request_host, &request_site_url, &request_icons_dir).await
        });
        lock_mutex(&inflight, "favicon in-flight").remove(&host);
        result
    });
}

pub(crate) fn host_from_url(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host = rest.split(['/', '?', '#']).next()?.trim();
    let host = host.split('@').next_back()?.trim();
    (!host.is_empty()).then_some(host)
}

pub(crate) fn local_favicon_path(icons_dir: &Path, host: &str) -> Option<PathBuf> {
    let path = icons_dir.join(format!("{}.png", sanitize_host(host)));
    path.is_file().then_some(path)
}

pub(crate) fn feed_host(site_url: Option<&str>, feed_url: &str) -> Option<String> {
    let candidate = site_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(feed_url);
    host_from_url(candidate).map(str::to_owned)
}
