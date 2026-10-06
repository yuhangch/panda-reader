use super::worker::{WorkerState, job};
use std::path::PathBuf;
use tokio::sync::oneshot;

use std::path::Path;

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
    let client = reqwest::Client::builder()
        .user_agent("PandaReader/0.1")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
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
        {
            let mut guard = inflight.lock().map_err(|e| e.to_string())?;
            if guard.contains_key(&host) {
                let safe = sanitize_host(&host);
                let path = icons_dir.join(format!("{safe}.png"));
                if path.is_file() {
                    return Ok(path);
                }
            }
            guard.insert(host.clone(), ());
        }
        let result = runtime.block_on(fetch_favicon(&host, &site_url, &icons_dir));
        let _ = inflight.lock().map(|mut g| g.remove(&host));
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
