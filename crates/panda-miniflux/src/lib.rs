use anyhow::{Context as _, bail};
use reqwest::{Client, Method, RequestBuilder};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{path::Path, time::Duration};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Connection {
    pub endpoint: String,
    pub token: String,
}

impl Connection {
    pub fn new(endpoint: &str, token: &str) -> anyhow::Result<Self> {
        let mut url = Url::parse(endpoint.trim()).context("Invalid Miniflux URL")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            bail!("Miniflux URL must use http or https");
        }
        if token.trim().is_empty() {
            bail!("Enter a Miniflux API token");
        }
        url.set_query(None);
        url.set_fragment(None);
        // Accept both the Miniflux root and a `/v1` API base. Requests always
        // append `v1/...`, so a pasted `/v1` path would otherwise become `/v1/v1`.
        let mut path = url.path().trim_end_matches('/').to_owned();
        if path.to_ascii_lowercase().ends_with("/v1") {
            path.truncate(path.len() - 3);
        }
        if path.is_empty() {
            path = "/".into();
        }
        if !path.ends_with('/') {
            path.push('/');
        }
        url.set_path(&path);
        Ok(Self {
            endpoint: url.to_string(),
            token: token.trim().to_owned(),
        })
    }

    pub fn load(path: &Path) -> anyhow::Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(self)?;
        #[cfg(unix)]
        {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(&bytes)?;
        }
        #[cfg(not(unix))]
        std::fs::write(path, bytes)?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct Miniflux {
    connection: Connection,
    client: Client,
}

#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Category {
    pub id: i64,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Feed {
    pub id: i64,
    pub title: String,
    pub feed_url: String,
    #[serde(default)]
    pub site_url: String,
    pub category: Option<Category>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Entry {
    pub id: i64,
    pub feed_id: i64,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    #[serde(default)]
    pub content: String,
    pub status: String,
    #[serde(default)]
    pub starred: bool,
}

#[derive(Deserialize)]
pub struct EntryPage {
    pub total: usize,
    pub entries: Vec<Entry>,
}

impl Miniflux {
    pub fn new(connection: Connection) -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("PandaReader/0.1")
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { connection, client })
    }

    pub fn endpoint(&self) -> &str {
        &self.connection.endpoint
    }

    fn request(&self, method: Method, path: &str) -> anyhow::Result<RequestBuilder> {
        let base = Url::parse(&self.connection.endpoint)?;
        let url = base.join(path)?;
        Ok(self
            .client
            .request(method, url)
            .header("X-Auth-Token", &self.connection.token))
    }

    async fn json<T: DeserializeOwned>(&self, request: RequestBuilder) -> anyhow::Result<T> {
        let response = request.send().await?.error_for_status()?;
        Ok(response.json().await?)
    }

    async fn empty(&self, request: RequestBuilder) -> anyhow::Result<()> {
        request.send().await?.error_for_status()?;
        Ok(())
    }

    pub async fn me(&self) -> anyhow::Result<User> {
        self.json(self.request(Method::GET, "v1/me")?).await
    }

    pub async fn feeds(&self) -> anyhow::Result<Vec<Feed>> {
        self.json(self.request(Method::GET, "v1/feeds")?).await
    }

    pub async fn entries(&self, offset: usize, limit: usize) -> anyhow::Result<EntryPage> {
        self.json(self.request(Method::GET, "v1/entries")?.query(&[
            ("offset", offset.to_string()),
            ("limit", limit.to_string()),
            ("order", "id".into()),
            ("direction", "desc".into()),
        ]))
        .await
    }

    pub async fn categories(&self) -> anyhow::Result<Vec<Category>> {
        self.json(self.request(Method::GET, "v1/categories")?).await
    }

    pub async fn add_feed(&self, url: &str) -> anyhow::Result<()> {
        let categories = self.categories().await?;
        let category_id = categories
            .first()
            .context("Miniflux has no available categories")?
            .id;
        self.empty(
            self.request(Method::POST, "v1/feeds")?
                .json(&serde_json::json!({"feed_url": url, "category_id": category_id})),
        )
        .await
    }

    pub async fn remove_feed(&self, id: i64) -> anyhow::Result<()> {
        self.empty(self.request(Method::DELETE, &format!("v1/feeds/{id}"))?)
            .await
    }

    pub async fn refresh_feeds(&self) -> anyhow::Result<()> {
        self.empty(self.request(Method::PUT, "v1/feeds/refresh")?)
            .await
    }

    pub async fn mark_read(&self, id: i64, read: bool) -> anyhow::Result<()> {
        self.mark_entries_status(&[id], read).await
    }

    pub async fn mark_entries_status(&self, ids: &[i64], read: bool) -> anyhow::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        for chunk in ids.chunks(100) {
            self.empty(
                self.request(Method::PUT, "v1/entries")?
                    .json(&serde_json::json!({
                        "entry_ids": chunk,
                        "status": if read { "read" } else { "unread" }
                    })),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn mark_feed_as_read(&self, feed_id: i64) -> anyhow::Result<()> {
        self.empty(self.request(Method::PUT, &format!("v1/feeds/{feed_id}/mark-all-as-read"))?)
            .await
    }

    pub async fn set_starred(&self, id: i64, starred: bool) -> anyhow::Result<()> {
        let entry: Entry = self
            .json(self.request(Method::GET, &format!("v1/entries/{id}"))?)
            .await?;
        if entry.starred != starred {
            self.empty(self.request(Method::PUT, &format!("v1/entries/{id}/bookmark"))?)
                .await?;
        }
        Ok(())
    }

    pub async fn import_opml(&self, xml: &str) -> anyhow::Result<()> {
        self.empty(
            self.request(Method::POST, "v1/import")?
                .header(reqwest::header::CONTENT_TYPE, "text/xml")
                .body(xml.to_owned()),
        )
        .await
    }

    pub async fn export_opml(&self) -> anyhow::Result<String> {
        Ok(self
            .request(Method::GET, "v1/export")?
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_reverse_proxy_base_and_rejects_invalid_token() {
        let config = Connection::new("https://example.org/miniflux", " token ").unwrap();
        assert_eq!(config.endpoint, "https://example.org/miniflux/");
        assert_eq!(config.token, "token");
        assert!(Connection::new("file:///tmp/miniflux", "token").is_err());
        assert!(Connection::new("https://example.org", " ").is_err());
    }

    #[test]
    fn strips_api_v1_suffix_from_pasted_base_urls() {
        for endpoint in [
            "https://lab.example.org/flux/v1/",
            "https://lab.example.org/flux/v1",
            "https://lab.example.org/flux/",
            "https://lab.example.org/flux",
        ] {
            let config = Connection::new(endpoint, "token").unwrap();
            assert_eq!(config.endpoint, "https://lab.example.org/flux/");
        }
        assert_eq!(
            Connection::new("https://example.org/v1/", "token")
                .unwrap()
                .endpoint,
            "https://example.org/"
        );
    }

    #[test]
    fn parses_entry_page_with_minimal_api_fields() {
        let page: EntryPage = serde_json::from_str(
            r#"{"total":1,"entries":[{"id":8,"feed_id":2,"title":"Test","status":"unread"}]}"#,
        )
        .unwrap();
        assert_eq!(page.entries[0].id, 8);
        assert!(!page.entries[0].starred);
    }

    #[test]
    fn remove_feed_sends_delete_to_miniflux_api() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 1024];
            while !bytes.ends_with(b"\r\n\r\n") {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(
                request.starts_with("DELETE /v1/feeds/42 "),
                "unexpected request: {request}"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("x-auth-token: secret")
            );
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .unwrap();
        });
        let config = Connection::new(&format!("http://{address}"), "secret").unwrap();
        let remote = Miniflux::new(config).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(remote.remove_feed(42)).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn authenticated_requests_work_behind_a_reverse_proxy() {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let replies = [
                r#"{"id":7,"username":"reader"}"#,
                r#"[{"id":4,"title":"Feed","feed_url":"https://example.com/rss","site_url":"https://example.com","category":{"id":2,"title":"News"}}]"#,
                r#"{"total":1,"entries":[{"id":8,"feed_id":4,"title":"Article","status":"unread"}]}"#,
            ];
            for (index, body) in replies.into_iter().enumerate() {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 1024];
                while !bytes.ends_with(b"\r\n\r\n") {
                    let count = stream.read(&mut chunk).unwrap();
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("x-auth-token: secret")
                );
                let expected = match index {
                    0 => "GET /proxy/v1/me ",
                    1 => "GET /proxy/v1/feeds ",
                    _ => "GET /proxy/v1/entries?",
                };
                assert!(request.starts_with(expected), "{request}");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let config = Connection::new(&format!("http://{address}/proxy"), "secret").unwrap();
        let client = Miniflux::new(config).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert_eq!(client.me().await.unwrap().username, "reader");
            assert_eq!(
                client.feeds().await.unwrap()[0]
                    .category
                    .as_ref()
                    .unwrap()
                    .title,
                "News"
            );
            assert_eq!(client.entries(0, 100).await.unwrap().entries[0].id, 8);
        });
        server.join().unwrap();
    }
}
