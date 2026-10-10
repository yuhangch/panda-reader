use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MAX_FAVICON_BYTES: usize = 2 * 1024 * 1024;

static FAVICON_LIMIT: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
static FAVICON_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

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

/// Fetch feed icons outside the shared service worker pool so slow icon hosts cannot delay sync.
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

    let _permit = FAVICON_LIMIT
        .get_or_init(|| tokio::sync::Semaphore::new(6))
        .acquire()
        .await
        .map_err(|error| error.to_string())?;
    // Another feed may have requested the same host while this task waited for a permit.
    if path.is_file() {
        return Ok(path);
    }

    let client = FAVICON_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("PandaReader/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(2))
            .timeout(std::time::Duration::from_secs(4))
            .build()
            .expect("favicon HTTP client configuration is valid")
    });

    let site_icon_url = url::Url::parse(site_url).ok().and_then(|mut url| {
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return None;
        }
        url.set_path("/favicon.ico");
        url.set_query(None);
        url.set_fragment(None);
        Some(url)
    });

    let site_icon_url =
        site_icon_url.ok_or_else(|| "site URL is not a valid HTTP(S) URL".to_owned())?;
    // Keep icon lookup on the feed's own origin; a missing icon falls back to the UI initials.
    let bytes = download_icon(client, site_icon_url.as_str())
        .await
        .and_then(|bytes| normalize_icon(&bytes))?;

    let tmp = icons_dir.join(format!("{safe}.png.part"));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Ok(path),
        // Another request for the same host may have won the race to publish the icon.
        Err(_) if path.is_file() => {
            let _ = std::fs::remove_file(tmp);
            Ok(path)
        }
        Err(error) => {
            let _ = std::fs::remove_file(tmp);
            Err(error.to_string())
        }
    }
}

async fn download_icon(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_FAVICON_BYTES as u64)
    {
        return Err("favicon exceeds the 2 MiB size limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_FAVICON_BYTES {
            return Err("favicon exceeds the 2 MiB size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err("empty favicon".into());
    }
    Ok(bytes)
}

fn normalize_icon(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|error| format!("unsupported favicon image: {error}"))?;
    let mut png = std::io::Cursor::new(Vec::new());
    decoded
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| format!("could not encode favicon: {error}"))?;
    Ok(png.into_inner())
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
