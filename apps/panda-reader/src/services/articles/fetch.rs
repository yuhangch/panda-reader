use anyhow::Context as _;
use panda_core::network::{ArticleNetworkMode, is_public_ip, validate_http_destination};
use reqwest::Client;
use std::time::Duration;
use url::Url;

const MAX_ARTICLE_PAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_ARTICLE_REDIRECTS: usize = 10;
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36";

#[derive(Clone, Debug)]
pub(crate) struct FetchedArticlePage {
    pub(crate) final_url: String,
    pub(crate) content_type: String,
    pub(crate) charset: String,
    pub(crate) body_bytes: usize,
    pub(crate) html: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ArticleFetchNetwork {
    pub(crate) mode: ArticleNetworkMode,
    pub(crate) proxy_url: String,
    pub(crate) trusted_fake_ip_ranges: String,
}

impl Default for ArticleFetchNetwork {
    fn default() -> Self {
        Self {
            mode: ArticleNetworkMode::System,
            proxy_url: String::new(),
            trusted_fake_ip_ranges: "198.18.0.0/15\n2001:2::/48".into(),
        }
    }
}

impl ArticleFetchNetwork {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        if self.mode == ArticleNetworkMode::ExplicitProxy {
            validate_proxy_url(&self.proxy_url)?;
        }
        if self.mode == ArticleNetworkMode::TrustedTransparent {
            parse_trusted_fake_ip_ranges(&self.trusted_fake_ip_ranges)?;
        }
        Ok(())
    }
}

/// The route is selected again for every redirect target. In System mode we
/// cannot inspect a PAC decision portably, so destination DNS stays strict;
/// users with synthetic DNS should configure an explicit proxy or opt in to
/// trusted transparent routing.
#[derive(Clone, Debug)]
enum FetchRoute {
    SystemProxy(Vec<ipnet::IpNet>),
    Direct,
    ExplicitProxy(String),
    TrustedTransparent(Vec<ipnet::IpNet>),
}

struct RoutePlanner<'a> {
    network: &'a ArticleFetchNetwork,
}

impl<'a> RoutePlanner<'a> {
    fn new(network: &'a ArticleFetchNetwork) -> Self {
        Self { network }
    }

    fn route_for(&self, url: &Url) -> anyhow::Result<FetchRoute> {
        validate_article_url(url)?;
        Ok(match self.network.mode {
            ArticleNetworkMode::System => FetchRoute::SystemProxy(parse_trusted_fake_ip_ranges(
                &self.network.trusted_fake_ip_ranges,
            )?),
            ArticleNetworkMode::Direct => FetchRoute::Direct,
            ArticleNetworkMode::ExplicitProxy => {
                FetchRoute::ExplicitProxy(self.network.proxy_url.clone())
            }
            ArticleNetworkMode::TrustedTransparent => FetchRoute::TrustedTransparent(
                parse_trusted_fake_ip_ranges(&self.network.trusted_fake_ip_ranges)?,
            ),
        })
    }

    async fn plan(
        &self,
        url: &Url,
    ) -> anyhow::Result<(FetchRoute, Option<(String, Vec<std::net::SocketAddr>)>)> {
        let route = self.route_for(url)?;
        let pin = pin_destination_for_route(url, &route).await?;
        Ok((route, pin))
    }
}

pub(super) async fn fetch_html_with_network(
    url: &str,
    referer: Option<&str>,
    network: &ArticleFetchNetwork,
) -> anyhow::Result<FetchedArticlePage> {
    let response = fetch_public_destination_with_network(url, referer, network).await?;
    let final_url = response.url().as_str().to_owned();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = read_response_limited(response, MAX_ARTICLE_PAGE_BYTES, "article page").await?;
    let charset = content_type
        .split(';')
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("charset")
                .then_some(value.trim())
        })
        .unwrap_or("utf-8")
        .trim_matches([' ', '\'', '"']);
    validate_article_content_type(&content_type, &bytes)?;
    let body_bytes = bytes.len();
    let encoding =
        encoding_rs::Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    Ok(FetchedArticlePage {
        final_url,
        content_type: content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase(),
        charset: encoding.name().to_owned(),
        body_bytes,
        html: encoding.decode(&bytes).0.into_owned(),
    })
}

fn validate_article_content_type(content_type: &str, body: &[u8]) -> anyhow::Result<()> {
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if media_type.is_empty() {
        if looks_like_html(body) || looks_like_json(body) {
            return Ok(());
        }
        anyhow::bail!("article URL returned content with no recognizable document type")
    }
    if matches!(
        media_type.as_str(),
        "text/html"
            | "application/xhtml+xml"
            | "text/plain"
            | "application/json"
            | "text/xml"
            | "application/xml"
    ) {
        return Ok(());
    }
    if media_type.starts_with("image/")
        || media_type.starts_with("audio/")
        || media_type.starts_with("video/")
        || matches!(
            media_type.as_str(),
            "application/pdf" | "application/octet-stream" | "application/zip"
        )
    {
        anyhow::bail!("article URL returned non-HTML content ({media_type})");
    }
    // Some publishers serve HTML with an incorrect MIME type. Keep those pages
    // usable when the response body clearly starts with recognizable markup.
    if looks_like_html(body) {
        return Ok(());
    }
    anyhow::bail!("article URL returned an unsupported content type ({media_type})")
}

fn looks_like_html(body: &[u8]) -> bool {
    let sample = String::from_utf8_lossy(&body[..body.len().min(4096)]).to_ascii_lowercase();
    let sample = sample.trim_start_matches('\u{feff}').trim_start();
    [
        "<!doctype html",
        "<html",
        "<head",
        "<body",
        "<article",
        "<main",
    ]
    .iter()
    .any(|prefix| sample.starts_with(prefix) || sample.contains(prefix))
}

fn looks_like_json(body: &[u8]) -> bool {
    let sample = String::from_utf8_lossy(&body[..body.len().min(4096)]);
    matches!(sample.trim_start().chars().next(), Some('{' | '['))
}

#[cfg(test)]
async fn fetch_public_destination(
    url: &str,
    referer: Option<&str>,
) -> anyhow::Result<reqwest::Response> {
    fetch_public_destination_with_network(url, referer, &ArticleFetchNetwork::default()).await
}

async fn fetch_public_destination_with_network(
    url: &str,
    referer: Option<&str>,
    network: &ArticleFetchNetwork,
) -> anyhow::Result<reqwest::Response> {
    network.validate()?;
    let mut current_url = Url::parse(url)?;
    let planner = RoutePlanner::new(network);
    for redirect_count in 0..=MAX_ARTICLE_REDIRECTS {
        let (route, pinned_host) = planner.plan(&current_url).await?;
        let mut client = Client::builder()
            .timeout(Duration::from_secs(25))
            .redirect(reqwest::redirect::Policy::none());
        match route {
            FetchRoute::SystemProxy(_) => {}
            FetchRoute::Direct | FetchRoute::TrustedTransparent(_) => {
                client = client.no_proxy();
            }
            FetchRoute::ExplicitProxy(proxy_url) => {
                client = client.proxy(reqwest::Proxy::all(proxy_url)?);
            }
        }
        if let Some((host, addresses)) = pinned_host {
            client = client.resolve_to_addrs(&host, &addresses);
        }
        let client = client.build()?;
        let mut request = client
            .get(current_url.clone())
            .header(reqwest::header::USER_AGENT, BROWSER_UA)
            .header(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header(
                reqwest::header::ACCEPT_LANGUAGE,
                "zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7",
            );
        if let Some(referer) = referer {
            request = request.header(reqwest::header::REFERER, referer);
        }
        let response = request.send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            anyhow::bail!(
                "Publisher blocked automated article downloads ({status}). Open the original page instead."
            );
        }
        if status.is_redirection() {
            if redirect_count == MAX_ARTICLE_REDIRECTS {
                anyhow::bail!("article page exceeded the redirect limit");
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .context("article redirect has no valid Location header")?;
            current_url = article_redirect_url(&current_url, location)?;
            continue;
        }
        return Ok(response.error_for_status()?);
    }
    anyhow::bail!("article page exceeded the redirect limit")
}

pub(super) fn article_redirect_url(current: &Url, location: &str) -> anyhow::Result<Url> {
    let next = current.join(location)?;
    if next.scheme() != "http" && next.scheme() != "https" {
        anyhow::bail!("article redirect used an unsupported URL scheme");
    }
    if current.scheme() == "https" && next.scheme() == "http" {
        anyhow::bail!("article redirect attempted to downgrade HTTPS to HTTP");
    }
    Ok(next)
}

#[cfg(test)]
async fn pin_public_destination(
    url: &Url,
) -> anyhow::Result<Option<(String, Vec<std::net::SocketAddr>)>> {
    pin_destination(url, &ArticleFetchNetwork::default()).await
}

#[cfg(test)]
async fn pin_destination(
    url: &Url,
    network: &ArticleFetchNetwork,
) -> anyhow::Result<Option<(String, Vec<std::net::SocketAddr>)>> {
    RoutePlanner::new(network)
        .plan(url)
        .await
        .map(|(_, pin)| pin)
}

async fn pin_destination_for_route(
    url: &Url,
    route: &FetchRoute,
) -> anyhow::Result<Option<(String, Vec<std::net::SocketAddr>)>> {
    use std::net::IpAddr;

    let host = url.host().context("article URL has no host")?;
    if matches!(route, FetchRoute::ExplicitProxy(_)) {
        if let Some(address) = host_ip(host) {
            if !is_public_ip(address) {
                anyhow::bail!("article URL resolves to a non-public network address");
            }
        }
        return Ok(None);
    }
    let port = url
        .port_or_known_default()
        .context("article URL has no port")?;
    match host {
        url::Host::Domain(domain) => {
            let domain = domain.to_owned();
            let lookup_domain = domain.clone();
            let addresses: Vec<_> = tokio::task::spawn_blocking(move || {
                use std::net::ToSocketAddrs as _;
                (lookup_domain.as_str(), port)
                    .to_socket_addrs()
                    .map(|addresses| addresses.collect())
            })
            .await??;
            let fake_ranges = match route {
                FetchRoute::SystemProxy(ranges) | FetchRoute::TrustedTransparent(ranges) => {
                    ranges.as_slice()
                }
                _ => &[],
            };
            if addresses.is_empty()
                || addresses.iter().any(|address| {
                    !is_public_ip(address.ip())
                        && (is_local_or_private_ip(address.ip())
                            || !fake_ranges
                                .iter()
                                .any(|range| range.contains(&address.ip())))
                })
            {
                if addresses
                    .iter()
                    .any(|address| is_known_synthetic_route_ip(address.ip()))
                    && !fake_ranges.iter().any(|range| {
                        addresses
                            .iter()
                            .any(|address| range.contains(&address.ip()))
                    })
                {
                    anyhow::bail!(
                        "article URL resolved to a reserved or synthetic routing address outside the trusted CIDRs; add its range in Developer settings or use an explicit proxy"
                    );
                }
                anyhow::bail!("article URL resolves to a non-public network address");
            }
            Ok(Some((domain.to_owned(), addresses)))
        }
        url::Host::Ipv4(address) => {
            if !is_public_ip(IpAddr::V4(address)) {
                anyhow::bail!("article URL resolves to a non-public network address");
            }
            Ok(None)
        }
        url::Host::Ipv6(address) => {
            if !is_public_ip(IpAddr::V6(address)) {
                anyhow::bail!("article URL resolves to a non-public network address");
            }
            Ok(None)
        }
    }
}

fn validate_article_url(url: &Url) -> anyhow::Result<()> {
    validate_http_destination(url).map_err(anyhow::Error::msg)
}

fn host_ip(host: url::Host<&str>) -> Option<std::net::IpAddr> {
    use std::net::IpAddr;
    match host {
        url::Host::Ipv4(address) => Some(IpAddr::V4(address)),
        url::Host::Ipv6(address) => Some(IpAddr::V6(address)),
        url::Host::Domain(_) => None,
    }
}

fn is_local_or_private_ip(address: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            let first_two = u16::from(octets[0]) << 8 | u16::from(octets[1]);
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_broadcast()
                || address.is_unspecified()
                || address.is_multicast()
                || octets[0] == 0
                || octets[0] >= 240
                || (0x6440..=0x647f).contains(&first_two)
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || (segments[0] & 0xfe00 == 0xfc00)
                || (segments[0] & 0xffc0 == 0xfe80)
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_local_or_private_ip(v4.into()))
        }
    }
}

fn is_known_synthetic_route_ip(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(address) => {
            let octets = address.octets();
            octets[0] == 198 && (18..=19).contains(&octets[1])
        }
        std::net::IpAddr::V6(address) => {
            let segments = address.segments();
            segments[0] == 0x2001 && segments[1] == 0x0002 && segments[2] == 0
        }
    }
}

fn parse_trusted_fake_ip_ranges(value: &str) -> anyhow::Result<Vec<ipnet::IpNet>> {
    let ranges: anyhow::Result<Vec<_>> = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.parse()
                .with_context(|| format!("invalid fake-IP CIDR: {line}"))
        })
        .collect();
    let ranges = ranges?;
    if ranges.is_empty() {
        anyhow::bail!("trusted transparent mode needs at least one fake-IP CIDR");
    }
    let local_ranges = [
        "0.0.0.0/8",
        "10.0.0.0/8",
        "100.64.0.0/10",
        "127.0.0.0/8",
        "169.254.0.0/16",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "224.0.0.0/4",
        "240.0.0.0/4",
        "::/128",
        "::1/128",
        "::ffff:0:0/96",
        "fc00::/7",
        "fe80::/10",
        "ff00::/8",
    ]
    .into_iter()
    .map(|range| range.parse::<ipnet::IpNet>().expect("valid built-in CIDR"))
    .collect::<Vec<_>>();
    if ranges
        .iter()
        .any(|range| local_ranges.iter().any(|local| cidrs_overlap(range, local)))
    {
        anyhow::bail!(
            "trusted fake-IP CIDRs must not overlap private, local, or multicast networks"
        );
    }
    Ok(ranges)
}

fn cidrs_overlap(left: &ipnet::IpNet, right: &ipnet::IpNet) -> bool {
    use ipnet::IpNet;
    match (left, right) {
        (IpNet::V4(left), IpNet::V4(right)) => {
            let prefix = left.prefix_len().min(right.prefix_len());
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(left.network()) & mask == u32::from(right.network()) & mask
        }
        (IpNet::V6(left), IpNet::V6(right)) => {
            let prefix = left.prefix_len().min(right.prefix_len());
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            u128::from(left.network()) & mask == u128::from(right.network()) & mask
        }
        _ => false,
    }
}

fn validate_proxy_url(value: &str) -> anyhow::Result<()> {
    let proxy = Url::parse(value).context("proxy URL must be an absolute URL")?;
    if !matches!(proxy.scheme(), "http" | "https" | "socks5h")
        || proxy.host().is_none()
        || !proxy.username().is_empty()
        || proxy.password().is_some()
        || proxy.query().is_some()
        || proxy.fragment().is_some()
    {
        anyhow::bail!(
            "proxy URL must use http, https, or socks5h and must not contain credentials"
        );
    }
    Ok(())
}

pub(super) async fn read_response_limited(
    mut response: reqwest::Response,
    limit: usize,
    label: &str,
) -> anyhow::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        anyhow::bail!("{label} response exceeds the {limit}-byte limit");
    }
    let mut bytes =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(limit as u64) as usize);
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            anyhow::bail!("{label} response exceeds the {limit}-byte limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    #[test]
    fn article_content_type_validation_accepts_html_and_rejects_binary_media() {
        assert!(
            validate_article_content_type("text/html; charset=utf-8", b"<html>ok</html>").is_ok()
        );
        assert!(validate_article_content_type("application/json", br#"{"body":"text"}"#).is_ok());
        assert!(validate_article_content_type("application/pdf", b"%PDF-1.7").is_err());
        assert!(validate_article_content_type("image/png", b"\x89PNG").is_err());
        assert!(validate_article_content_type("application/x-custom", b"<html>ok</html>").is_ok());
        assert!(validate_article_content_type("application/x-custom", b"opaque binary").is_err());
        assert!(validate_article_content_type("", b"<html>ok</html>").is_ok());
        assert!(validate_article_content_type("", br#"{"content":"ok"}"#).is_ok());
        assert!(validate_article_content_type("", b"opaque binary").is_err());
    }

    #[test]
    fn article_fetch_only_allows_public_ip_destinations() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "192.0.2.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        for address in ["1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public_ip(address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn common_synthetic_ranges_are_distinguished_from_private_destinations() {
        for address in ["198.18.0.21", "198.19.255.254", "2001:2::14"] {
            let address = address.parse().unwrap();
            assert!(!is_public_ip(address));
            assert!(is_known_synthetic_route_ip(address));
        }
        for address in ["10.0.0.1", "192.168.1.1", "2001:db8::1"] {
            let address = address.parse().unwrap();
            assert!(!is_public_ip(address));
            assert!(!is_known_synthetic_route_ip(address));
        }
    }

    #[test]
    fn system_proxy_accepts_configured_synthetic_routes_but_rejects_private_addresses() {
        let network = ArticleFetchNetwork::default();
        let route = RoutePlanner::new(&network)
            .route_for(&Url::parse("https://example.com/story").unwrap())
            .unwrap();
        let ranges = match &route {
            FetchRoute::SystemProxy(ranges) => ranges,
            _ => panic!("default route should use the system proxy"),
        };
        let accepts = |address: &str| {
            let address: std::net::IpAddr = address.parse().unwrap();
            is_public_ip(address)
                || (!is_local_or_private_ip(address)
                    && ranges.iter().any(|range| range.contains(&address)))
        };

        assert!(accepts("198.18.0.21"));
        assert!(accepts("2001:2::14"));
        assert!(!accepts("127.0.0.1"));
        assert!(!accepts("192.168.1.10"));
        assert!(!accepts("169.254.1.1"));
        assert!(!accepts("192.0.2.1"));
    }

    #[tokio::test]
    async fn article_fetch_rejects_loopback_and_pins_public_dns_results() {
        assert!(
            pin_public_destination(&Url::parse("http://127.0.0.1/").unwrap())
                .await
                .is_err()
        );
        assert!(
            pin_public_destination(&Url::parse("http://[::1]/").unwrap())
                .await
                .is_err()
        );
        assert!(
            pin_public_destination(&Url::parse("http://localhost/").unwrap())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn article_redirect_destinations_use_the_same_public_address_check() {
        let current = Url::parse("http://example.com/story").unwrap();
        let redirected = article_redirect_url(&current, "http://127.0.0.1/admin").unwrap();
        assert!(pin_public_destination(&redirected).await.is_err());
        assert!(
            article_redirect_url(
                &Url::parse("https://example.com").unwrap(),
                "http://example.com"
            )
            .is_err()
        );
        assert!(article_redirect_url(&current, "file:///etc/passwd").is_err());
    }

    #[tokio::test]
    async fn explicit_http_proxy_receives_original_host_without_local_dns_lookup() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 11\r\nConnection: close\r\n\r\nproxy works",
                )
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let network = ArticleFetchNetwork {
            mode: ArticleNetworkMode::ExplicitProxy,
            proxy_url,
            ..ArticleFetchNetwork::default()
        };

        let response = fetch_html_with_network("http://reader-test.invalid/story", None, &network)
            .await
            .unwrap();

        assert_eq!(response.html, "proxy works");
        assert_eq!(response.final_url, "http://reader-test.invalid/story");
        assert_eq!(response.content_type, "text/html");
        assert_eq!(response.charset, "UTF-8");
        let request = server.join().unwrap();
        assert!(request.starts_with("GET http://reader-test.invalid/story HTTP/1.1"));
    }

    #[tokio::test]
    async fn fetched_page_reports_final_redirect_url_and_decodes_declared_charset() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in [
                b"HTTP/1.1 302 Found\r\nLocation: http://reader-test.invalid/final/story?key=private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=windows-1252\r\nContent-Length: 4\r\nConnection: close\r\n\r\n\x63\x61\x66\xe9".as_slice(),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                stream.write_all(response).unwrap();
            }
            requests
        });
        let network = ArticleFetchNetwork {
            mode: ArticleNetworkMode::ExplicitProxy,
            proxy_url,
            ..ArticleFetchNetwork::default()
        };

        let page = fetch_html_with_network("http://reader-test.invalid/start", None, &network)
            .await
            .unwrap();

        assert_eq!(
            page.final_url,
            "http://reader-test.invalid/final/story?key=private"
        );
        assert_eq!(page.content_type, "text/html");
        assert_eq!(page.charset, "windows-1252");
        assert_eq!(page.html, "café");
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET http://reader-test.invalid/start HTTP/1.1"));
        assert!(
            requests[1]
                .starts_with("GET http://reader-test.invalid/final/story?key=private HTTP/1.1")
        );
    }

    #[tokio::test]
    async fn socks5h_proxy_receives_the_hostname_for_remote_resolution() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_url = format!("socks5h://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut greeting = [0; 2];
            stream.read_exact(&mut greeting).unwrap();
            let mut methods = vec![0; usize::from(greeting[1])];
            stream.read_exact(&mut methods).unwrap();
            stream.write_all(&[5, 0]).unwrap();

            let mut request_header = [0; 4];
            stream.read_exact(&mut request_header).unwrap();
            assert_eq!(request_header, [5, 1, 0, 3]);
            let mut host_length = [0; 1];
            stream.read_exact(&mut host_length).unwrap();
            let mut host = vec![0; usize::from(host_length[0])];
            stream.read_exact(&mut host).unwrap();
            let mut port = [0; 2];
            stream.read_exact(&mut port).unwrap();
            stream
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .unwrap();

            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 11\r\nConnection: close\r\n\r\nproxy works",
                )
                .unwrap();
            (String::from_utf8(host).unwrap(), u16::from_be_bytes(port))
        });
        let network = ArticleFetchNetwork {
            mode: ArticleNetworkMode::ExplicitProxy,
            proxy_url,
            ..ArticleFetchNetwork::default()
        };

        let response = fetch_html_with_network("http://reader-test.invalid/story", None, &network)
            .await
            .unwrap();

        assert_eq!(response.html, "proxy works");
        assert_eq!(response.final_url, "http://reader-test.invalid/story");
        assert_eq!(server.join().unwrap(), ("reader-test.invalid".into(), 80));
    }
}
