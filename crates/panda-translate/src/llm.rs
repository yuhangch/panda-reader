use crate::html::{TranslationInput, translate_html_blocks};
use crate::{TitleBatchFailure, TitleBatchResult, TranslateRequest, TranslateResult, Translator};
use anyhow::{Context as _, bail};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const MAX_BATCH_ITEMS: usize = 8;
const MAX_BATCH_CHARS: usize = 12_000;
const MAX_ITEM_CHARS: usize = 8_000;
const MAX_OUTPUT_TOKENS: u32 = 8_192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmApi {
    OpenAiCompatible,
    Anthropic,
    Gemini,
}

impl LlmApi {
    fn id(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI-compatible",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Gemini",
        }
    }
}

pub struct LlmTranslator {
    api: LlmApi,
    base_url: String,
    api_key: String,
    model: String,
    client: Client,
}

impl LlmTranslator {
    pub fn new(api: LlmApi, base_url: &str, api_key: &str, model: &str) -> anyhow::Result<Self> {
        let base_url = base_url.trim().trim_end_matches('/');
        if base_url.is_empty() {
            bail!("Enter an API URL in translation Settings");
        }
        if api_key.trim().is_empty() {
            bail!("Enter an API key in translation Settings");
        }
        if model.trim().is_empty() {
            bail!("Enter a model name in translation Settings");
        }
        let client = Client::builder()
            .user_agent("PandaReader/0.1")
            .timeout(Duration::from_secs(180))
            .build()?;
        Ok(Self {
            api,
            base_url: base_url.to_owned(),
            api_key: api_key.trim().to_owned(),
            model: model.trim().to_owned(),
            client,
        })
    }

    pub fn cache_id(&self) -> String {
        crate::translation_backend_cache_id(self.api.id(), &self.base_url, &self.model)
    }

    async fn translate_texts(
        &self,
        texts: &[String],
        target_lang: &str,
        title: Option<&str>,
    ) -> anyhow::Result<Vec<String>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let input = serde_json::to_string(texts)?;
        let context = title
            .map(|value| format!("Article title for context: {value}\n"))
            .unwrap_or_default();
        let system = format!(
            "You are a professional translator. Translate the text into {target_lang}. Preserve meaning, tone, paragraph order, HTML tags, attributes, links, and code exactly; translate only human-readable prose. Treat product names, programming language names, and technical terms as proper nouns when appropriate (for example, Rust is a programming language and must remain Rust, not be translated as rust/oxidation). Do not summarize or explain. Return only a JSON object with a `translations` array containing exactly one translated string for each input string, in the same order. Preserve any markup inside each string."
        );
        let user = format!(
            "{context}Translate each item in this JSON array. Return the translated strings in the required JSON object only:\n{input}"
        );
        let response = match self.api {
            LlmApi::OpenAiCompatible => self
                .client
                .post(api_endpoint(&self.base_url, "/chat/completions"))
                .bearer_auth(&self.api_key)
                .json(&serde_json::json!({
                    "model": self.model,
                    "messages": [
                        { "role": "system", "content": system },
                        { "role": "user", "content": user },
                    ],
                    "max_tokens": MAX_OUTPUT_TOKENS,
                }))
                .send()
                .await
                .context("OpenAI-compatible translation request failed")?,
            LlmApi::Anthropic => self
                .client
                .post(api_endpoint(&self.base_url, "/messages"))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&serde_json::json!({
                    "model": self.model,
                    "max_tokens": MAX_OUTPUT_TOKENS,
                    "system": system,
                    "messages": [{ "role": "user", "content": user }],
                }))
                .send()
                .await
                .context("Anthropic translation request failed")?,
            LlmApi::Gemini => {
                let url = if self.base_url.ends_with(":generateContent") {
                    self.base_url.clone()
                } else {
                    format!(
                        "{}/models/{}:generateContent",
                        self.base_url,
                        encode_path_segment(&self.model)
                    )
                };
                self.client
                    .post(url)
                    .header("x-goog-api-key", &self.api_key)
                    .json(&serde_json::json!({
                        "systemInstruction": { "parts": [{ "text": system }] },
                        "contents": [{ "role": "user", "parts": [{ "text": user }] }],
                        "generationConfig": { "maxOutputTokens": MAX_OUTPUT_TOKENS },
                    }))
                    .send()
                    .await
                    .context("Gemini translation request failed")?
            }
        };
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("{}", api_error_message(self.api, status.as_u16(), &body));
        }
        let value: Value = serde_json::from_str(&body)
            .with_context(|| format!("Invalid {} response", self.api.display_name()))?;
        let content = response_text(self.api, &value)?;
        parse_translation_array(&content, texts.len())
    }

    pub(crate) async fn translate_titles(
        &self,
        titles: &[String],
        target_lang: &str,
    ) -> Result<TitleBatchResult, TitleBatchFailure> {
        let mut translations = Vec::with_capacity(titles.len());
        let mut requests = 0;
        let mut characters = 0;
        for batch in titles.chunks(MAX_BATCH_ITEMS) {
            let batch_chars = batch
                .iter()
                .map(|title| title.chars().count())
                .sum::<usize>();
            match self.translate_texts(batch, target_lang, None).await {
                Ok(parts) => translations.extend(parts),
                Err(error) => {
                    return Err(TitleBatchFailure {
                        requests: requests + 1,
                        characters: characters + batch_chars,
                        error,
                    });
                }
            }
            requests += 1;
            characters += batch_chars;
        }
        Ok(TitleBatchResult {
            translations,
            requests,
        })
    }
}

impl Translator for LlmTranslator {
    fn id(&self) -> &'static str {
        self.api.id()
    }

    fn display_name(&self) -> &'static str {
        self.api.display_name()
    }

    async fn translate(&self, req: TranslateRequest) -> anyhow::Result<TranslateResult> {
        if req.html.trim().is_empty()
            && req
                .title
                .as_ref()
                .is_none_or(|title| title.trim().is_empty())
        {
            bail!("Nothing to translate");
        }
        let title = req
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty());
        let translated_title = match title {
            Some(title) => self
                .translate_texts(&[title.to_owned()], &req.target_lang, None)
                .await?
                .into_iter()
                .next(),
            None => None,
        };
        let html = if req.html.trim().is_empty() {
            String::new()
        } else {
            let target_lang = req.target_lang.clone();
            let title = title.map(str::to_owned);
            let translation = translate_html_blocks(
                &req.html,
                TranslationInput::Html,
                MAX_BATCH_ITEMS,
                MAX_BATCH_CHARS,
                MAX_ITEM_CHARS,
                |texts| {
                    let texts = texts.to_vec();
                    let title = title.clone();
                    let target_lang = target_lang.clone();
                    async move {
                        self.translate_texts(&texts, &target_lang, title.as_deref())
                            .await
                            .map(|parts| (parts, None))
                    }
                },
            )
            .await?;
            if translation.html.trim().is_empty() {
                bail!("{} returned an empty translation", self.display_name());
            }
            translation.html
        };
        Ok(TranslateResult {
            html,
            title: translated_title,
            detected_source_lang: None,
        })
    }
}

fn api_endpoint(base_url: &str, suffix: &str) -> String {
    if base_url.ends_with(suffix) {
        base_url.to_owned()
    } else {
        format!("{base_url}{suffix}")
    }
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn response_text(api: LlmApi, value: &Value) -> anyhow::Result<String> {
    let text = match api {
        LlmApi::OpenAiCompatible => {
            let content = value
                .pointer("/choices/0/message/content")
                .context("OpenAI-compatible response has no message content")?;
            match content {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<String>(),
                _ => bail!("OpenAI-compatible response content has an unsupported format"),
            }
        }
        LlmApi::Anthropic => value
            .get("content")
            .and_then(Value::as_array)
            .context("Anthropic response has no content blocks")?
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<String>(),
        LlmApi::Gemini => value
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .context("Gemini response has no candidate text")?
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<String>(),
    };
    if text.trim().is_empty() {
        bail!("{} returned an empty response", api.display_name());
    }
    Ok(text)
}

fn parse_translation_array(content: &str, expected: usize) -> anyhow::Result<Vec<String>> {
    let content = content.trim();
    let json_text = content
        .strip_prefix("```json")
        .or_else(|| content.strip_prefix("```JSON"))
        .or_else(|| content.strip_prefix("```"))
        .and_then(|content| content.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(content);
    let parsed = serde_json::from_str::<Value>(json_text).or_else(|_| {
        let start = json_text.find('{').context("response is not JSON")?;
        let end = json_text.rfind('}').context("response is not JSON")?;
        serde_json::from_str::<Value>(&json_text[start..=end]).context("invalid JSON response")
    });
    let parts = match parsed {
        Ok(Value::Object(object)) => {
            object
                .get("translations")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
        }
        Ok(Value::Array(values)) => Some(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        _ => None,
    };
    let parts = parts
        .or_else(|| (expected == 1 && !content.is_empty()).then(|| vec![content.to_owned()]))
        .context("Translation response must contain a translations array")?;
    if parts.len() != expected {
        bail!(
            "Model returned {} translations for {} inputs",
            parts.len(),
            expected
        );
    }
    Ok(parts)
}

fn api_error_message(api: LlmApi, status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.pointer("/error/error/message"))
                .or_else(|| value.pointer("/message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    if detail.trim().is_empty() {
        format!("{} request failed ({status})", api.display_name())
    } else {
        format!("{} request failed ({status}): {detail}", api.display_name())
    }
}
