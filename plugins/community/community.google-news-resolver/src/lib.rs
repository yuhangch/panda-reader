#[cfg(target_arch = "wasm32")]
use panda_plugin_sdk::Stage;
use serde_json::Value;
#[cfg(target_arch = "wasm32")]
use serde_json::json;

#[cfg(target_arch = "wasm32")]
const CONSENT_COOKIE: &str = "SOCS=CAESEwgDEgk0ODE3Nzk3MjQaAmVuIAEaBgiA_LyaBg";
#[cfg(target_arch = "wasm32")]
const BATCH_ENDPOINT: &str = "https://news.google.com/_/DotsSplashUi/data/batchexecute";

#[cfg(target_arch = "wasm32")]
fn resolve(stage: Stage) -> Result<bool, String> {
    if stage != Stage::ResolveUrl {
        return Ok(false);
    }
    let context = panda_plugin_sdk::context()?;
    let source_url = context
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "resolver context did not contain a URL".to_owned())?;
    let Some(article_id) = article_id(source_url) else {
        return Ok(false);
    };

    let page = panda_plugin_sdk::http_request(
        "GET",
        source_url,
        &[
            ("accept", "text/html,application/xhtml+xml"),
            ("accept-language", "en-US,en;q=0.9"),
            ("cookie", CONSENT_COOKIE),
            ("referer", "https://news.google.com/"),
            ("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36"),
        ],
        "",
    )?;
    if !(200..300).contains(&page.status) {
        return Err(format!("Google News article page returned HTTP {}", page.status));
    }
    let signature = html_attribute(&page.body, "data-n-a-sg")
        .ok_or_else(|| "Google News page is missing a decode signature".to_owned())?;
    let timestamp = html_attribute(&page.body, "data-n-a-ts")
        .ok_or_else(|| "Google News page is missing a decode timestamp".to_owned())?
        .parse::<i64>()
        .map_err(|error| format!("invalid Google News decode timestamp: {error}"))?;

    let shell = json!([
        ["X", "X", ["X", "X"], null, null, 1, 1, "US:en", null, 1,
         null, null, null, null, null, 0, 1],
        "X", "X", 1, [1, 1, 1], 1, 1, null, 0, 0, null, 0
    ]);
    let inner = json!(["garturlreq", shell, article_id, timestamp, signature]);
    let request = json!([[ ["Fbv4je", inner.to_string(), null, "generic"] ]]);
    let form = format!("f.req={}", form_encode(&request.to_string()));
    let decoded = panda_plugin_sdk::http_request(
        "POST",
        BATCH_ENDPOINT,
        &[
            ("content-type", "application/x-www-form-urlencoded;charset=UTF-8"),
            ("cookie", CONSENT_COOKIE),
            ("referer", "https://news.google.com/"),
            ("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36"),
        ],
        &form,
    )?;
    if !(200..300).contains(&decoded.status) {
        return Err(format!("Google News decode endpoint returned HTTP {}", decoded.status));
    }
    let destination = batch_response_url(&decoded.body)
        .ok_or_else(|| "Google News response did not contain a publisher URL".to_owned())?;
    panda_plugin_sdk::set_resolved_url(&destination)?;
    Ok(true)
}

fn article_id(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://news.google.com/")?;
    let mut segments = rest.split(['/', '?', '#']);
    while let Some(segment) = segments.next() {
        if segment == "articles" {
            let id = segments.next()?;
            return (!id.is_empty()).then(|| id.to_owned());
        }
    }
    None
}

fn html_attribute(html: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = html.find(&needle)? + needle.len();
    let end = html[start..].find('"')? + start;
    Some(html[start..end].to_owned())
}

fn batch_response_url(body: &str) -> Option<String> {
    let body = body
        .trim_start()
        .strip_prefix(")]}'")
        .map(str::trim_start)
        .unwrap_or_else(|| body.trim_start());
    let start = body.find('[')?;
    let envelopes: Vec<Value> = serde_json::from_str(&body[start..]).ok()?;
    for envelope in envelopes {
        let Some(row) = envelope.as_array() else { continue };
        if row.first()?.as_str()? != "wrb.fr" || row.get(1)?.as_str()? != "Fbv4je" {
            continue;
        }
        let nested: Value = serde_json::from_str(row.get(2)?.as_str()?).ok()?;
        let nested = nested.as_array()?;
        if nested.first()?.as_str()? == "garturlres" {
            let url = nested.get(1)?.as_str()?;
            if url.starts_with("https://") || url.starts_with("http://") {
                return Some(url.to_owned());
            }
        }
    }
    None
}

fn form_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                output.push(char::from(byte));
            }
            b' ' => output.push('+'),
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

#[cfg(target_arch = "wasm32")]
panda_plugin_sdk::export_plugin!(resolve);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_google_news_rss_wrappers_only() {
        assert_eq!(
            article_id("https://news.google.com/rss/articles/CBMiEXAMPLE?oc=5").as_deref(),
            Some("CBMiEXAMPLE")
        );
        assert_eq!(article_id("https://example.com/articles/123"), None);
    }

    #[test]
    fn parses_google_batch_response_and_attributes() {
        let body = r#")]}'

[["wrb.fr","Fbv4je","[\"garturlres\",\"https://www.example.com/story\",1]",null,null,null,"generic"]]
"#;
        assert_eq!(
            batch_response_url(body).as_deref(),
            Some("https://www.example.com/story")
        );
        assert_eq!(
            html_attribute(r#"<div data-n-a-sg="sig123" data-n-a-ts="42"></div>"#, "data-n-a-sg")
                .as_deref(),
            Some("sig123")
        );
    }

    #[test]
    fn form_encoding_escapes_google_payload() {
        assert_eq!(form_encode("a b&c"), "a+b%26c");
    }
}
