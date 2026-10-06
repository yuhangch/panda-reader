use crate::{ProviderIdentity, numeric_id};
use anyhow::{Context as _, bail};
use panda_miniflux::{Category, Entry, Feed};
use reqwest::{Client, Method, RequestBuilder};
use serde_json::Value;
use std::time::Duration;
use url::Url;

#[derive(Clone, Debug)]
pub struct FreshRssConnection {
    pub endpoint: String,
    pub username: String,
    pub password: String,
}

impl FreshRssConnection {
    pub fn new(endpoint: &str, username: &str, password: &str) -> anyhow::Result<Self> {
        let mut url = Url::parse(endpoint.trim()).context("Invalid FreshRSS API URL")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            bail!("FreshRSS URL must use http or https");
        }
        if username.trim().is_empty() || password.is_empty() {
            bail!("Enter the FreshRSS username and API password");
        }
        url.set_query(None);
        url.set_fragment(None);
        let path = url.path().trim_end_matches('/');
        let path = if path.to_ascii_lowercase().ends_with("/api/greader.php") {
            format!("{path}/")
        } else {
            format!("{path}/api/greader.php/")
        };
        url.set_path(&path);
        Ok(Self {
            endpoint: url.to_string(),
            username: username.trim().to_owned(),
            password: password.to_owned(),
        })
    }
}

#[derive(Clone)]
pub struct FreshRss {
    connection: FreshRssConnection,
    client: Client,
}

#[derive(Clone, Debug)]
struct Session {
    auth: String,
    token: String,
}

impl FreshRss {
    pub(super) fn new(connection: FreshRssConnection) -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("PandaReader/0.1")
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { connection, client })
    }

    fn url(&self, path: &str) -> anyhow::Result<Url> {
        let base = Url::parse(&self.connection.endpoint)?;
        Ok(base.join(path)?)
    }

    async fn session(&self) -> anyhow::Result<Session> {
        let login = self
            .client
            .post(self.url("accounts/ClientLogin")?)
            .form(&[
                ("Email", self.connection.username.as_str()),
                ("Passwd", self.connection.password.as_str()),
            ])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let sid = login
            .lines()
            .find_map(|line| line.strip_prefix("SID="))
            .context("FreshRSS did not return a session ID")?;
        let auth = login
            .lines()
            .find_map(|line| line.strip_prefix("Auth="))
            .unwrap_or(sid)
            .to_owned();
        let session = Session {
            auth,
            token: String::new(),
        };
        let token = self
            .auth_request(&session, Method::GET, "reader/api/0/token")?
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(Session {
            auth: session.auth,
            token: token.trim().to_owned(),
        })
    }

    fn auth_request(
        &self,
        session: &Session,
        method: Method,
        path: &str,
    ) -> anyhow::Result<RequestBuilder> {
        Ok(self.client.request(method, self.url(path)?).header(
            "Authorization",
            format!("GoogleLogin auth={}", session.auth),
        ))
    }

    async fn json(
        &self,
        session: &Session,
        path: &str,
        query: &[(&str, String)],
    ) -> anyhow::Result<Value> {
        Ok(self
            .auth_request(session, Method::GET, path)?
            .query(query)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn mutate(
        &self,
        session: &Session,
        path: &str,
        values: &[(&str, String)],
    ) -> anyhow::Result<()> {
        let mut form = values.to_vec();
        form.push(("T", session.token.clone()));
        self.auth_request(session, Method::POST, path)?
            .form(&form)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub(super) async fn identity(&self) -> anyhow::Result<ProviderIdentity> {
        self.session().await?;
        Ok(ProviderIdentity {
            account: format!("{}:{}", self.connection.endpoint, self.connection.username),
            name: self.connection.username.clone(),
        })
    }

    pub(super) async fn feeds(&self) -> anyhow::Result<Vec<Feed>> {
        let session = self.session().await?;
        let value = self
            .json(
                &session,
                "reader/api/0/subscription/list",
                &[("output", "json".into())],
            )
            .await?;
        let rows = value
            .get("subscriptions")
            .and_then(Value::as_array)
            .context("FreshRSS response has no subscriptions list")?;
        rows.iter()
            .map(|row| {
                let id_text = row
                    .get("id")
                    .and_then(Value::as_str)
                    .context("FreshRSS feed is missing its ID")?;
                let id = numeric_id(id_text)?;
                let title = row
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Untitled feed")
                    .to_owned();
                let feed_url = row
                    .get("url")
                    .and_then(Value::as_str)
                    .or_else(|| row.get("feedUrl").and_then(Value::as_str))
                    .context("FreshRSS feed is missing its URL")?
                    .to_owned();
                let site_url = row
                    .get("htmlUrl")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let category = row
                    .get("categories")
                    .and_then(Value::as_array)
                    .and_then(|items| items.first())
                    .and_then(|item| {
                        let label = item
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        Some(Category {
                            id: 0,
                            title: label,
                        })
                    });
                Ok(Feed {
                    id,
                    title,
                    feed_url,
                    site_url: site_url.unwrap_or_default(),
                    language: None,
                    category,
                })
            })
            .collect()
    }

    pub(super) async fn all_entries(&self) -> anyhow::Result<Vec<Entry>> {
        let session = self.session().await?;
        let mut cursor = String::new();
        let mut output = Vec::new();
        loop {
            let mut query = vec![("output", "json".to_owned()), ("n", "500".to_owned())];
            if !cursor.is_empty() {
                query.push(("c", cursor.clone()));
            }
            let value = self
                .json(
                    &session,
                    "reader/api/0/stream/contents/reading-list",
                    &query,
                )
                .await?;
            let rows = value
                .get("items")
                .and_then(Value::as_array)
                .context("FreshRSS response has no items list")?;
            for row in rows {
                if let Some(entry) = parse_entry(row)? {
                    output.push(entry);
                }
            }
            let next = value
                .get("continuation")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if rows.is_empty() || next.is_empty() || next == cursor {
                break;
            }
            cursor = next;
        }
        Ok(output)
    }

    pub(super) async fn add_feed(&self, url: &str) -> anyhow::Result<()> {
        let session = self.session().await?;
        self.mutate(
            &session,
            "reader/api/0/subscription/quickadd",
            &[("quickadd", url.to_owned())],
        )
        .await?;
        Ok(())
    }

    pub(super) async fn remove_feed(&self, id: i64) -> anyhow::Result<()> {
        let session = self.session().await?;
        self.mutate(
            &session,
            "reader/api/0/subscription/edit",
            &[("ac", "unsubscribe".into()), ("s", format!("feed/{id}"))],
        )
        .await
    }

    pub(super) async fn mark_entries_status(
        &self,
        ids: &[i64],
        state: &str,
        add: bool,
    ) -> anyhow::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let session = self.session().await?;
        let action = if add { "a" } else { "r" };
        let state = match state {
            "read" => "user/-/state/com.google/read",
            "starred" => "user/-/state/com.google/starred",
            _ => bail!("Unsupported FreshRSS state"),
        };
        for chunk in ids.chunks(100) {
            let mut form = Vec::new();
            form.push(("T", session.token.clone()));
            for id in chunk {
                form.push(("i", id.to_string()));
            }
            form.push((action, state.to_owned()));
            self.auth_request(&session, Method::POST, "reader/api/0/edit-tag")?
                .form(&form)
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }

    pub(super) async fn set_starred(&self, id: i64, starred: bool) -> anyhow::Result<()> {
        self.mark_entries_status(&[id], "starred", starred).await
    }

    pub(super) async fn import_opml(&self, xml: &str) -> anyhow::Result<()> {
        let document = opml::OPML::from_str(xml)?;
        let session = self.session().await?;
        let mut urls = Vec::new();
        for outline in &document.body.outlines {
            collect_urls(outline, &mut urls);
        }
        for url in urls {
            self.mutate(
                &session,
                "reader/api/0/subscription/quickadd",
                &[("quickadd", url)],
            )
            .await?;
        }
        Ok(())
    }

    pub(super) async fn export_opml(&self) -> anyhow::Result<String> {
        let feeds = self.feeds().await?;
        let outlines = feeds
            .into_iter()
            .map(|feed| opml::Outline {
                text: feed.title.clone(),
                xml_url: Some(feed.feed_url),
                r#type: Some("rss".into()),
                ..opml::Outline::default()
            })
            .collect();
        Ok(opml::OPML {
            head: Some(opml::Head {
                title: Some("Panda Reader subscriptions".into()),
                ..opml::Head::default()
            }),
            body: opml::Body { outlines },
            ..opml::OPML::default()
        }
        .to_string()?)
    }
}

fn parse_entry(row: &Value) -> anyhow::Result<Option<Entry>> {
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .context("FreshRSS item is missing its ID")?;
    let feed_id = row
        .get("origin")
        .and_then(|v| v.get("streamId"))
        .and_then(Value::as_str)
        .context("FreshRSS item is missing its feed ID")?;
    let categories = row.get("categories").and_then(Value::as_array);
    let states = categories
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let content = row
        .get("content")
        .and_then(|v| v.get("content"))
        .and_then(Value::as_str)
        .or_else(|| {
            row.get("summary")
                .and_then(|v| v.get("content"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let url = row
        .get("canonical")
        .and_then(Value::as_array)
        .and_then(|links| links.first())
        .and_then(|link| link.get("href"))
        .and_then(Value::as_str)
        .or_else(|| {
            row.get("alternate")
                .and_then(Value::as_array)
                .and_then(|links| links.first())
                .and_then(|link| link.get("href"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned);
    let published_at = row
        .get("published")
        .and_then(Value::as_i64)
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|time| time.to_rfc3339());
    Ok(Some(Entry {
        id: numeric_id(id)?,
        feed_id: numeric_id(feed_id)?,
        title: row
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled")
            .to_owned(),
        url,
        author: row.get("author").and_then(Value::as_str).map(str::to_owned),
        published_at,
        content: content.to_owned(),
        status: if states.iter().any(|s| s.ends_with("/read")) {
            "read"
        } else {
            "unread"
        }
        .into(),
        starred: states.iter().any(|s| s.ends_with("/starred")),
    }))
}

fn collect_urls(outline: &opml::Outline, output: &mut Vec<String>) {
    if let Some(url) = outline.xml_url.as_ref() {
        output.push(url.clone());
    }
    for child in &outline.outlines {
        collect_urls(child, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    fn mock_server(
        request_count: usize,
        responder: impl Fn(&str, &str) -> (u16, &'static str) + Send + 'static,
    ) -> (String, std::thread::JoinHandle<Vec<(String, String)>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let join = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..request_count {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    if stream.read_exact(&mut byte).is_err() {
                        break;
                    }
                    request.push(byte[0]);
                }
                let headers = String::from_utf8_lossy(&request).to_string();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; content_length];
                let _ = stream.read_exact(&mut body);
                let body = String::from_utf8_lossy(&body).to_string();
                let first_line = headers.lines().next().unwrap_or_default().to_owned();
                let status = responder(&first_line, &body);
                let reason = if status.0 == 200 {
                    "OK"
                } else {
                    "Unauthorized"
                };
                let response = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status.0,
                    reason,
                    status.1.len(),
                    status.1
                );
                stream.write_all(response.as_bytes()).unwrap();
                seen.push((first_line, body));
            }
            seen
        });
        (format!("http://{address}/"), join)
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn normalizes_freshrss_endpoint_and_requires_api_credentials() {
        let config =
            FreshRssConnection::new("https://rss.example.net/base/", " reader ", "secret").unwrap();
        assert_eq!(
            config.endpoint,
            "https://rss.example.net/base/api/greader.php/"
        );
        assert_eq!(config.username, "reader");
        assert!(FreshRssConnection::new("file:///tmp", "reader", "secret").is_err());
    }

    #[test]
    fn maps_reader_api_items_to_common_entry_shape() {
        let item = serde_json::json!({
            "id":"tag:google.com,2005:reader/item/99", "title":"Story", "published": 1700000000,
            "origin":{"streamId":"feed/4"}, "canonical":[{"href":"https://example.org/story"}],
            "content":{"content":"<p>Hello</p>"}, "categories":["user/-/state/com.google/read", "user/-/state/com.google/starred"]
        });
        let entry = parse_entry(&item).unwrap().unwrap();
        assert_eq!(entry.id, 99);
        assert_eq!(entry.feed_id, 4);
        assert_eq!(entry.status, "read");
        assert!(entry.starred);
    }

    #[test]
    fn mock_server_covers_auth_pagination_feed_changes_and_state_sync() {
        let (endpoint, server) = mock_server(18, |line, _body| {
            let path = line.split_whitespace().nth(1).unwrap_or_default();
            if path.ends_with("/accounts/ClientLogin") {
                (200, "SID=session\nAuth=auth-token")
            } else if path.ends_with("/reader/api/0/token") {
                (200, "csrf")
            } else if path.contains("/subscription/list") {
                (
                    200,
                    r#"{"subscriptions":[{"id":"feed/7","title":"Feed","url":"https://example.org/rss","htmlUrl":"https://example.org","categories":[{"label":"News"}]}]}"#,
                )
            } else if path.contains("/stream/contents/reading-list") && path.contains("c=next") {
                (
                    200,
                    r#"{"items":[{"id":"tag:google.com,2005:reader/item/2","title":"Second","origin":{"streamId":"feed/7"},"content":{"content":"two"}}]}"#,
                )
            } else if path.contains("/stream/contents/reading-list") {
                (
                    200,
                    r#"{"items":[{"id":"tag:google.com,2005:reader/item/1","title":"First","origin":{"streamId":"feed/7"},"content":{"content":"one"}}],"continuation":"next"}"#,
                )
            } else {
                (200, "OK")
            }
        });
        let connection = FreshRssConnection::new(&endpoint, "reader", "api-password").unwrap();
        let client = FreshRss::new(connection).unwrap();
        let runtime = runtime();
        assert_eq!(runtime.block_on(client.identity()).unwrap().name, "reader");
        let feeds = runtime.block_on(client.feeds()).unwrap();
        assert_eq!(feeds[0].id, 7);
        assert_eq!(feeds[0].category.as_ref().unwrap().title, "News");
        let entries = runtime.block_on(client.all_entries()).unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        runtime
            .block_on(client.add_feed("https://new.example/rss"))
            .unwrap();
        runtime.block_on(client.remove_feed(7)).unwrap();
        runtime
            .block_on(client.mark_entries_status(&[1, 2], "read", true))
            .unwrap();

        let requests = server.join().unwrap();
        assert!(
            requests
                .iter()
                .all(|(line, _)| line.contains("/api/greader.php/"))
        );
        assert!(
            requests
                .iter()
                .filter(|(line, _)| line.ends_with("/reader/api/0/token HTTP/1.1"))
                .count()
                == 6
        );
        assert!(
            requests
                .iter()
                .any(|(line, body)| line.contains("subscription/quickadd")
                    && body.contains("quickadd=https%3A%2F%2Fnew.example%2Frss")
                    && body.contains("T=csrf"))
        );
        assert!(
            requests
                .iter()
                .any(|(line, body)| line.contains("subscription/edit")
                    && body.contains("ac=unsubscribe")
                    && body.contains("s=feed%2F7"))
        );
        assert!(requests.iter().any(|(line, body)| line.contains("edit-tag")
            && body.contains("a=user%2F-%2Fstate%2Fcom.google%2Fread")
            && body.contains("i=1")
            && body.contains("i=2")));
        assert!(
            requests
                .iter()
                .any(|(line, _)| line.contains("stream/contents/reading-list")
                    && line.contains("c=next"))
        );
    }

    #[test]
    fn mock_server_surfaces_authentication_errors() {
        let (endpoint, server) = mock_server(1, |_, _| (401, "invalid credentials"));
        let client =
            FreshRss::new(FreshRssConnection::new(&endpoint, "reader", "bad").unwrap()).unwrap();
        let error = runtime().block_on(client.identity()).unwrap_err();
        assert!(error.to_string().contains("401"));
        assert_eq!(server.join().unwrap().len(), 1);
    }
}
