use crate::html::{TranslationInput, translate_html_blocks};
use crate::{TranslateRequest, TranslateResult, Translator};
use anyhow::{Context as _, bail};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

/// Azure allows many segments per request; keep batches modest.
const MAX_BATCH: usize = 40;
const MAX_BATCH_CHARS: usize = 12_000;

pub struct AzureTranslator {
    key: String,
    region: String,
    client: Client,
}

impl AzureTranslator {
    pub fn new(key: &str, region: &str) -> anyhow::Result<Self> {
        if key.is_empty() {
            bail!("Enter an Azure Translator API key in Settings");
        }
        let region = if region.is_empty() {
            "eastasia".to_owned()
        } else {
            region.to_owned()
        };
        let client = Client::builder()
            .user_agent("PandaReader/0.1")
            .timeout(Duration::from_secs(90))
            .build()?;
        Ok(Self {
            key: key.to_owned(),
            region,
            client,
        })
    }

    async fn translate_texts(
        &self,
        texts: &[String],
        target_lang: &str,
    ) -> anyhow::Result<(Vec<String>, Option<String>)> {
        if texts.is_empty() {
            return Ok((Vec::new(), None));
        }
        let url = format!(
            "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&to={}&textType=html",
            urlencoding_lite(target_lang)
        );
        let payload: Vec<_> = texts
            .iter()
            .map(|text| serde_json::json!({ "text": text }))
            .collect();
        let response = self
            .client
            .post(&url)
            .header("Ocp-Apim-Subscription-Key", &self.key)
            .header("Ocp-Apim-Subscription-Region", &self.region)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await
            .context("Azure Translator request failed")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(azure_error_message(status.as_u16(), &body));
        }
        let parsed: Vec<AzureTranslateResponse> =
            serde_json::from_str(&body).context("Invalid Azure Translator response")?;
        if parsed.len() != texts.len() {
            bail!(
                "Azure Translator returned {} results for {} inputs",
                parsed.len(),
                texts.len()
            );
        }
        let detected = parsed
            .first()
            .and_then(|item| item.detected_language.as_ref())
            .map(|item| item.language.clone());
        let parts = parsed
            .into_iter()
            .map(|item| {
                item.translations
                    .into_iter()
                    .next()
                    .map(|translation| translation.text)
                    .unwrap_or_default()
            })
            .collect();
        Ok((parts, detected))
    }

    pub(crate) async fn translate_titles(
        &self,
        titles: &[String],
        target: &str,
    ) -> Result<crate::TitleBatchResult, crate::TitleBatchFailure> {
        let mut output = Vec::with_capacity(titles.len());
        let mut requests = 0;
        let mut start = 0;
        while start < titles.len() {
            let mut end = start;
            let mut chars = 0;
            while end < titles.len() && end - start < MAX_BATCH {
                let n = titles[end].chars().count();
                if end > start && chars + n > MAX_BATCH_CHARS {
                    break;
                }
                chars += n;
                end += 1;
            }
            if end == start {
                end += 1;
            }
            let (parts, _) = self
                .translate_texts(&titles[start..end], target)
                .await
                .map_err(|error| crate::TitleBatchFailure {
                    requests: requests + 1,
                    characters: titles[..end]
                        .iter()
                        .map(|title| title.chars().count())
                        .sum(),
                    error,
                })?;
            output.extend(parts);
            requests += 1;
            start = end;
        }
        Ok(crate::TitleBatchResult {
            translations: output,
            requests,
        })
    }
}

impl Translator for AzureTranslator {
    fn id(&self) -> &'static str {
        "azure"
    }

    fn display_name(&self) -> &'static str {
        "Azure Translator"
    }

    async fn translate(&self, req: TranslateRequest) -> anyhow::Result<TranslateResult> {
        if req.html.trim().is_empty() && req.title.as_ref().is_none_or(|t| t.trim().is_empty()) {
            bail!("Nothing to translate");
        }
        let mut detected = None;
        let translated_title = if let Some(title) = req
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
        {
            let (parts, source) = self
                .translate_texts(std::slice::from_ref(&title.to_owned()), &req.target_lang)
                .await?;
            detected = source;
            parts.into_iter().next()
        } else {
            None
        };

        let html = if req.html.trim().is_empty() {
            String::new()
        } else {
            let target_lang = req.target_lang.clone();
            let translation = translate_html_blocks(
                &req.html,
                TranslationInput::Html,
                MAX_BATCH,
                MAX_BATCH_CHARS,
                MAX_BATCH_CHARS,
                |texts| {
                    let texts = texts.to_vec();
                    let target_lang = target_lang.clone();
                    async move { self.translate_texts(&texts, &target_lang).await }
                },
            )
            .await?;
            if translation.html.trim().is_empty() {
                bail!("Azure Translator returned an empty translation");
            }
            detected = detected.or(translation.detected_source_lang);
            translation.html
        };

        Ok(TranslateResult {
            html,
            title: translated_title,
            detected_source_lang: detected,
        })
    }
}

#[derive(Debug, Deserialize)]
struct AzureTranslateResponse {
    #[serde(rename = "detectedLanguage")]
    detected_language: Option<AzureDetectedLanguage>,
    translations: Vec<AzureTranslation>,
}

#[derive(Debug, Deserialize)]
struct AzureDetectedLanguage {
    language: String,
}

#[derive(Debug, Deserialize)]
struct AzureTranslation {
    text: String,
}

fn azure_error_message(status: u16, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(message) = value
            .pointer("/error/message")
            .and_then(|item| item.as_str())
    {
        return format!("Azure Translator error ({status}): {message}");
    }
    if body.trim().is_empty() {
        format!("Azure Translator error ({status})")
    } else {
        format!(
            "Azure Translator error ({status}): {}",
            body.chars().take(180).collect::<String>()
        )
    }
}

fn urlencoding_lite(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_key() {
        assert!(AzureTranslator::new("", "eastasia").is_err());
    }
}
