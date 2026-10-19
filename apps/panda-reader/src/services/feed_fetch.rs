//! HTTP retrieval for local RSS feeds. The store receives parsed feed data only.

use panda_store::FetchedFeed;
use reqwest::Client;
use std::sync::OnceLock;

const MAX_FEED_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

fn shared_http_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent("PandaReader/0.1 (+https://github.com)")
            .timeout(std::time::Duration::from_secs(25))
            .build()
            .expect("feed fetch HTTP client configuration is valid")
    })
}

pub(super) async fn fetch_feed_data(
    url: &str,
    etag: Option<&str>,
    modified: Option<&str>,
) -> anyhow::Result<FetchedFeed> {
    fetch_feed_data_with_limit(url, etag, modified, MAX_FEED_RESPONSE_BYTES).await
}

async fn fetch_feed_data_with_limit(
    url: &str,
    etag: Option<&str>,
    modified: Option<&str>,
    max_bytes: usize,
) -> anyhow::Result<FetchedFeed> {
    let client = shared_http_client();
    let mut request = client.get(url);
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    if let Some(modified) = modified {
        request = request.header(reqwest::header::IF_MODIFIED_SINCE, modified);
    }
    let response = request.send().await?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(FetchedFeed::not_modified(
            etag.map(str::to_owned),
            modified.map(str::to_owned),
        ));
    }
    let response = response.error_for_status()?;
    let next_etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let next_modified = response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        anyhow::bail!("RSS feed response exceeds the {max_bytes}-byte limit");
    }
    let mut response = response;
    let mut body =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(max_bytes as u64) as usize);
    while let Some(chunk) = response.chunk().await? {
        append_bounded(&mut body, &chunk, max_bytes)?;
    }
    let parsed = feed_rs::parser::parse(body.as_slice())?;
    Ok(FetchedFeed::from_parsed(parsed, next_etag, next_modified))
}

fn append_bounded(body: &mut Vec<u8>, chunk: &[u8], limit: usize) -> anyhow::Result<()> {
    if body.len().saturating_add(chunk.len()) > limit {
        anyhow::bail!("RSS feed response exceeds the {limit}-byte limit");
    }
    body.extend_from_slice(chunk);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write as _, net::TcpListener};

    #[test]
    fn response_limit_applies_to_streamed_chunks() {
        let mut body = vec![0; 3];
        assert!(append_bounded(&mut body, &[1, 2], 4).is_err());
        assert_eq!(body.len(), 3);
        append_bounded(&mut body, &[1], 4).unwrap();
        assert_eq!(body.len(), 4);
    }

    #[test]
    fn fetched_feed_parses_body_and_keeps_validators() {
        let body = br#"<rss version="2.0"><channel><title>Example</title><item><title>Story</title><guid>one</guid></item></channel></rss>"#;
        let parsed = feed_rs::parser::parse(&body[..]).unwrap();
        let feed = FetchedFeed::from_parsed(parsed, Some("tag".into()), Some("date".into()));
        assert!(!feed.is_not_modified());
        let unchanged = FetchedFeed::not_modified(Some("tag".into()), Some("date".into()));
        assert!(unchanged.is_not_modified());
    }

    #[tokio::test]
    async fn feed_download_rejects_oversized_declared_and_streamed_bodies() {
        for response in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n4\r\nefgh\r\n0\r\n\r\n".as_slice(),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/feed", listener.local_addr().unwrap());
            let response = response.to_vec();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let _ = std::io::Read::read(&mut stream, &mut request);
                stream.write_all(&response).unwrap();
            });

            let error = match fetch_feed_data_with_limit(&url, None, None, 5).await {
                Ok(_) => panic!("oversized feed unexpectedly succeeded"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("5-byte limit"));
            server.join().unwrap();
        }
    }
}
