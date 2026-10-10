//! Volcengine (火山引擎) machine translation via OpenAPI TranslateText.

use crate::html::{TranslationInput, split_text_for_translate, translate_html_blocks};
use crate::{TranslateRequest, TranslateResult, Translator};
use anyhow::{Context as _, bail};
use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::time::Duration;

type HmacSha256 = Hmac<Sha256>;

const HOST: &str = "open.volcengineapi.com";
const SERVICE: &str = "translate";
const REGION: &str = "cn-north-1";
const ACTION: &str = "TranslateText";
const VERSION: &str = "2020-06-01";
/// Volcengine rejects when total TextList characters exceed 5000.
const MAX_REQUEST_CHARS: usize = 4800;
/// Keep each TextList item comfortably under the per-item ceiling.
const MAX_ITEM_CHARS: usize = 4800;
/// Volcengine limits TextList to 16 items per request.
const MAX_BATCH: usize = 16;

pub struct VolcengineTranslator {
    access_key: String,
    secret_key: String,
    client: Client,
}

impl VolcengineTranslator {
    pub(crate) async fn translate_titles(
        &self,
        titles: &[String],
        target: &str,
    ) -> Result<crate::TitleBatchResult, crate::TitleBatchFailure> {
        let texts: Vec<String> = titles
            .iter()
            .flat_map(|title| split_for_translate(title))
            .collect();
        let batches = batches_within_budget(&texts);
        let mut output = Vec::with_capacity(texts.len());
        let mut requests = 0usize;
        let mut characters = 0usize;
        for batch in &batches {
            let batch_chars: usize = batch.iter().map(|text| text.chars().count()).sum();
            let (parts, _) = self
                .translate_batch(batch, map_target_lang(target))
                .await
                .map_err(|error| crate::TitleBatchFailure {
                    requests: requests + 1,
                    characters: characters + batch_chars,
                    input_tokens: None,
                    output_tokens: None,
                    error,
                })?;
            output.extend(parts);
            requests += 1;
            characters += batch_chars;
        }
        // Keep positional mapping for normal-sized titles; long titles may be split and rejoined.
        let mut mapped = Vec::with_capacity(titles.len());
        let mut cursor = 0;
        for title in titles {
            let count = split_for_translate(title).len();
            mapped.push(output[cursor..cursor + count].concat());
            cursor += count;
        }
        Ok(crate::TitleBatchResult {
            translations: mapped,
            requests,
            input_tokens: None,
            output_tokens: None,
        })
    }

    pub fn new(access_key: &str, secret_key: &str) -> anyhow::Result<Self> {
        if access_key.trim().is_empty() || secret_key.trim().is_empty() {
            bail!("Enter Volcengine Access Key ID and Secret Access Key in Settings");
        }
        let client = Client::builder()
            .user_agent("PandaReader/0.1")
            .timeout(Duration::from_secs(90))
            .build()?;
        Ok(Self {
            access_key: access_key.trim().to_owned(),
            secret_key: secret_key.trim().to_owned(),
            client,
        })
    }
}

impl Translator for VolcengineTranslator {
    fn id(&self) -> &'static str {
        "volcengine"
    }

    fn display_name(&self) -> &'static str {
        "Volcengine"
    }

    async fn translate(&self, req: TranslateRequest) -> anyhow::Result<TranslateResult> {
        let title = req
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned);
        if req.html.trim().is_empty() && title.is_none() {
            bail!("Nothing to translate");
        }
        let target = map_target_lang(&req.target_lang);
        let mut translated_title = None;
        let mut detected = None;
        if let Some(title) = &title {
            translated_title = Some(
                self.translate_sized_text(title, target, &mut detected)
                    .await?,
            );
        }
        let html = if req.html.trim().is_empty() {
            String::new()
        } else {
            let translation = translate_html_blocks(
                &req.html,
                TranslationInput::PlainText,
                MAX_BATCH,
                MAX_REQUEST_CHARS,
                MAX_ITEM_CHARS,
                |texts| {
                    let texts = texts.to_vec();
                    async move { self.translate_batch(&texts, target).await }
                },
            )
            .await?;
            detected = detected.or(translation.detected_source_lang);
            translation.html
        };
        Ok(TranslateResult {
            html,
            title: translated_title,
            detected_source_lang: detected,
            metrics: Default::default(),
        })
    }
}

impl VolcengineTranslator {
    async fn translate_sized_text(
        &self,
        text: &str,
        target: &str,
        detected: &mut Option<String>,
    ) -> anyhow::Result<String> {
        let chunks = split_for_translate(text);
        self.translate_chunks(&chunks, target, detected).await
    }

    async fn translate_chunks(
        &self,
        chunks: &[String],
        target: &str,
        detected: &mut Option<String>,
    ) -> anyhow::Result<String> {
        let mut parts = Vec::with_capacity(chunks.len());
        for batch in batches_within_budget(chunks) {
            let (batch_parts, source) = self.translate_batch(batch, target).await?;
            if detected.is_none() {
                *detected = source;
            }
            parts.extend(batch_parts);
        }
        Ok(parts.concat())
    }

    async fn translate_batch(
        &self,
        texts: &[String],
        target: &str,
    ) -> anyhow::Result<(Vec<String>, Option<String>)> {
        let body = serde_json::json!({
            "TargetLanguage": target,
            "TextList": texts,
        });
        let body_str = serde_json::to_string(&body)?;
        let body_hash = sha256_hex(body_str.as_bytes());
        let x_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let short_date = &x_date[..8];
        let credential_scope = format!("{short_date}/{REGION}/{SERVICE}/request");
        let signed_headers = "content-type;host;x-content-sha256;x-date";
        let canonical_headers = format!(
            "content-type:application/json\nhost:{HOST}\nx-content-sha256:{body_hash}\nx-date:{x_date}\n"
        );
        let canonical_query = format!("Action={ACTION}&Version={VERSION}");
        let canonical_request = format!(
            "POST\n/\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{body_hash}"
        );
        let hashed_canonical = sha256_hex(canonical_request.as_bytes());
        let string_to_sign =
            format!("HMAC-SHA256\n{x_date}\n{credential_scope}\n{hashed_canonical}");
        let signing_key = signing_key(&self.secret_key, short_date);
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));
        let authorization = format!(
            "HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key
        );
        let url = format!("https://{HOST}/?{canonical_query}");
        let response = self
            .client
            .post(url)
            .header("Authorization", authorization)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("Host", HOST)
            .header("X-Content-Sha256", &body_hash)
            .header("X-Date", &x_date)
            .body(body_str)
            .send()
            .await
            .context("Volcengine Translator request failed")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(volc_error_message(status.as_u16(), &body));
        }
        parse_translate_response(&body, texts.len())
            .map_err(|error| anyhow::anyhow!("{error}: {}", preview_body(&body)))
    }
}

fn parse_translate_response(
    body: &str,
    expected: usize,
) -> anyhow::Result<(Vec<String>, Option<String>)> {
    let value: serde_json::Value =
        serde_json::from_str(body).context("Invalid Volcengine Translator response")?;
    if let Some(message) = error_from_value(&value) {
        bail!("Volcengine Translator error: {message}");
    }
    let list = value
        .get("TranslationList")
        .and_then(|item| item.as_array())
        .context("Volcengine Translator returned no translations")?;
    if list.len() != expected {
        bail!(
            "Volcengine Translator returned {} results for {} inputs",
            list.len(),
            expected
        );
    }
    let detected = list
        .first()
        .and_then(|item| item.get("DetectedSourceLanguage"))
        .and_then(|item| item.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let parts = list
        .iter()
        .map(|item| {
            item.get("Translation")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_owned()
        })
        .collect();
    Ok((parts, detected))
}

fn error_from_value(value: &serde_json::Value) -> Option<String> {
    let metadata = value
        .get("ResponseMetadata")
        .or_else(|| value.get("ResponseMetaData"))?;
    let error = metadata.get("Error")?;
    if error.is_null() {
        return None;
    }
    if let Some(message) = error.as_str().filter(|value| !value.is_empty()) {
        return Some(message.to_owned());
    }
    let code = error
        .get("Code")
        .and_then(|item| item.as_str())
        .unwrap_or("Error");
    let message = error
        .get("Message")
        .and_then(|item| item.as_str())
        .unwrap_or("unknown");
    if code == "Error" && message == "unknown" && error.as_object().is_some_and(|o| o.is_empty()) {
        return None;
    }
    Some(format!("{code}: {message}"))
}

fn preview_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "(empty body)".into()
    } else {
        trimmed.chars().take(240).collect()
    }
}

fn map_target_lang(lang: &str) -> &str {
    match lang {
        "zh-Hans" | "zh-CN" | "zh_cn" | "zh" => "zh",
        "zh-Hant" | "zh-TW" | "zh_tw" => "zh-Hant",
        other => other,
    }
}

fn split_for_translate(html: &str) -> Vec<String> {
    split_text_for_translate(html, MAX_ITEM_CHARS)
}

/// Group chunks so each request stays under the total-character budget.
fn batches_within_budget(chunks: &[String]) -> Vec<&[String]> {
    let mut batches = Vec::new();
    let mut start = 0usize;
    while start < chunks.len() {
        let mut end = start;
        let mut total = 0usize;
        while end < chunks.len() && end - start < MAX_BATCH {
            let next = chunks[end].chars().count();
            if end > start && total + next > MAX_REQUEST_CHARS {
                break;
            }
            // A single oversized item still goes alone (already split to MAX_ITEM_CHARS).
            if end == start && next > MAX_REQUEST_CHARS {
                end += 1;
                break;
            }
            total += next;
            end += 1;
        }
        if end == start {
            end = start + 1;
        }
        batches.push(&chunks[start..end]);
        start = end;
    }
    batches
}

fn signing_key(secret: &str, short_date: &str) -> Vec<u8> {
    let k_date = hmac_sha256(secret.as_bytes(), short_date.as_bytes());
    let k_region = hmac_sha256(&k_date, REGION.as_bytes());
    let k_service = hmac_sha256(&k_region, SERVICE.as_bytes());
    hmac_sha256(&k_service, b"request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn volc_error_message(status: u16, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(message) = error_from_value(&value)
    {
        return format!("Volcengine Translator error ({status}): {message}");
    }
    if body.trim().is_empty() {
        format!("Volcengine Translator error ({status})")
    } else {
        format!(
            "Volcengine Translator error ({status}): {}",
            preview_body(body)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_credentials() {
        assert!(VolcengineTranslator::new("", "secret").is_err());
        assert!(VolcengineTranslator::new("ak", "").is_err());
    }

    #[test]
    fn maps_azure_style_chinese_codes() {
        assert_eq!(map_target_lang("zh-Hans"), "zh");
        assert_eq!(map_target_lang("zh-Hant"), "zh-Hant");
        assert_eq!(map_target_lang("ja"), "ja");
    }

    #[test]
    fn splits_long_html_near_tag_boundaries() {
        let html = format!("<p>{}</p>", "字".repeat(MAX_ITEM_CHARS + 20));
        let chunks = split_for_translate(&html);
        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= MAX_ITEM_CHARS)
        );
    }

    #[test]
    fn batches_respect_total_char_budget() {
        let chunks = vec!["a".repeat(3000), "b".repeat(3000), "c".repeat(1000)];
        let batches = batches_within_budget(&chunks);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[1].len(), 2);
        for batch in batches {
            let total: usize = batch.iter().map(|c| c.chars().count()).sum();
            assert!(total <= MAX_REQUEST_CHARS || batch.len() == 1);
        }
    }

    #[test]
    fn batches_7170_style_payload() {
        let html = "x".repeat(7170);
        let chunks = split_for_translate(&html);
        let batches = batches_within_budget(&chunks);
        assert!(chunks.len() > 1);
        for batch in batches {
            let total: usize = batch.iter().map(|c| c.chars().count()).sum();
            assert!(total <= MAX_REQUEST_CHARS);
            assert!(batch.len() <= MAX_BATCH);
        }
    }

    #[test]
    fn signing_key_is_deterministic() {
        let a = signing_key("secret", "20260101");
        let b = signing_key("secret", "20260101");
        assert_eq!(a, b);
        assert_ne!(
            signing_key("secret", "20260101"),
            signing_key("other", "20260101")
        );
    }

    #[test]
    fn parses_response_with_extra_object_and_metadata_alias() {
        let body = r#"{
          "TranslationList": [
            {
              "Translation": "你好世界",
              "DetectedSourceLanguage": "en",
              "Extra": { "input_characters": "11", "source_language": "en" }
            }
          ],
          "ResponseMetaData": {
            "RequestId": "req",
            "Action": "TranslateText",
            "Version": "2020-06-01",
            "Service": "translate",
            "Region": "cn-north-1"
          }
        }"#;
        let (parts, detected) = parse_translate_response(body, 1).unwrap();
        assert_eq!(parts, vec!["你好世界".to_owned()]);
        assert_eq!(detected.as_deref(), Some("en"));
    }

    #[test]
    fn parses_response_with_null_extra_and_both_metadata_keys() {
        let body = r#"{
          "TranslationList":[{"Translation":"世界你好","DetectedSourceLanguage":"","Extra":null}],
          "ResponseMetadata":{"RequestId":"a","Action":"TranslateText","Version":"2020-06-01","Service":"translate","Region":"cn-north-1"},
          "ResponseMetaData":{"RequestId":"a","Action":"TranslateText","Version":"2020-06-01","Service":"translate","Region":"cn-north-1"}
        }"#;
        let (parts, detected) = parse_translate_response(body, 1).unwrap();
        assert_eq!(parts, vec!["世界你好".to_owned()]);
        assert!(detected.is_none());
    }
}
