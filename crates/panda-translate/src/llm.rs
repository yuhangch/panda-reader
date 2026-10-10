use crate::html::{assemble_html_translation, prepare_html_translation_segments};
use crate::{TitleBatchFailure, TitleBatchResult, TranslateRequest, TranslateResult, Translator};
use anyhow::{Context as _, bail};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;

const MAX_BATCH_ITEMS: usize = 8;
const MAX_BATCH_CHARS: usize = 12_000;
const MAX_CONCURRENT_BATCHES: usize = 2;
const MAX_ITEM_INPUT_CHARS: usize = 8_000;
const MAX_OUTPUT_TOKENS: u32 = 8_192;
const PROMPT_REVISION: &str = "bilingual-segments-v2";
const MAX_RATE_LIMIT_RETRIES: usize = 2;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);
const MAX_BATCH_WAIT: Duration = Duration::from_secs(60);
const MAX_BATCH_FALLBACK_DEPTH: usize = 4;

#[derive(Clone, Debug)]
struct IdentifiedInput {
    id: String,
    text: String,
}

#[derive(Debug)]
struct LlmResponse {
    text: String,
}

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
    usage: Mutex<crate::TranslationMetrics>,
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
            .timeout(MAX_BATCH_WAIT)
            .build()?;
        Ok(Self {
            api,
            base_url: base_url.to_owned(),
            api_key: api_key.trim().to_owned(),
            model: model.trim().to_owned(),
            client,
            usage: Mutex::new(crate::TranslationMetrics::default()),
        })
    }

    pub fn cache_id(&self) -> String {
        crate::translation_backend_cache_id(self.api.id(), &self.base_url, &self.model)
    }

    pub fn prompt_revision() -> &'static str {
        PROMPT_REVISION
    }

    fn record_attempt(&self) {
        self.usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .requests += 1;
    }

    fn record_response_tokens(&self, input: Option<u64>, output: Option<u64>) {
        let mut usage = self
            .usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(input) = input {
            *usage.input_tokens.get_or_insert(0) += input;
        }
        if let Some(output) = output {
            *usage.output_tokens.get_or_insert(0) += output;
        }
    }

    fn take_usage(&self) -> crate::TranslationMetrics {
        std::mem::take(
            &mut *self
                .usage
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// Translate an article with resumable, validated block results. The
    /// callback runs only after the block's markup placeholders are restored.
    pub async fn translate_with_cached_segments<F>(
        &self,
        req: TranslateRequest,
        cached: HashMap<String, String>,
        save_segment: F,
    ) -> anyhow::Result<TranslateResult>
    where
        F: FnMut(&str, &str, &str) -> anyhow::Result<()>,
    {
        self.translate_with_cached_segments_and_progress(req, cached, save_segment, |_| Ok(()))
            .await
    }

    pub async fn translate_with_cached_segments_and_progress<F, P>(
        &self,
        req: TranslateRequest,
        cached: HashMap<String, String>,
        mut save_segment: F,
        mut on_batch: P,
    ) -> anyhow::Result<TranslateResult>
    where
        F: FnMut(&str, &str, &str) -> anyhow::Result<()>,
        P: FnMut(&[(String, String)]) -> anyhow::Result<()>,
    {
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
        let translated_title = if let Some(title) = title {
            let title_hash = crate::title_source_hash(title);
            let title_id = format!("title-{title_hash}");
            if let Some(cached_title) = cached.get(&title_id) {
                Some(cached_title.clone())
            } else {
                let translated = self
                    .translate_texts(&[title.to_owned()], &req.target_lang, None)
                    .await?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Translation response did not contain a title")
                    })?;
                save_segment(&title_id, &title_hash, &translated)?;
                Some(translated)
            }
        } else {
            None
        };

        let segments = prepare_html_translation_segments(&req.html);
        let mut translations = HashMap::new();
        let mut segment_parts = HashMap::<String, Vec<String>>::new();
        let mut part_hashes = HashMap::<String, String>::new();
        let mut pending = Vec::new();
        let mut part_results = HashMap::<String, String>::new();
        let mut cached_progress = Vec::new();
        for segment in &segments {
            if let Some(translation) = cached.get(&segment.id) {
                translations.insert(segment.id.clone(), translation.clone());
                cached_progress.push((segment.id.clone(), translation.clone()));
                continue;
            }
            let parts = split_identified_input(&segment.input, MAX_ITEM_INPUT_CHARS);
            let multipart = parts.len() > 1;
            for (index, text) in parts.into_iter().enumerate() {
                let part_id = if multipart {
                    format!("{}:part-{index}", segment.id)
                } else {
                    segment.id.clone()
                };
                let part_hash = if multipart {
                    let mut hash = Sha256::new();
                    hash.update(segment.source_hash.as_bytes());
                    hash.update([0]);
                    hash.update((index as u64).to_le_bytes());
                    hash.update([0]);
                    hash.update(text.as_bytes());
                    hex::encode(hash.finalize())
                } else {
                    segment.source_hash.clone()
                };
                segment_parts
                    .entry(segment.id.clone())
                    .or_default()
                    .push(part_id.clone());
                part_hashes.insert(part_id.clone(), part_hash);
                if let Some(translation) = cached.get(&part_id) {
                    part_results.insert(part_id, translation.clone());
                } else {
                    pending.push(IdentifiedInput { id: part_id, text });
                }
            }
        }

        if !cached_progress.is_empty() {
            on_batch(&cached_progress)?;
        }
        let mut batches = Vec::new();
        let mut cursor = 0;
        while cursor < pending.len() {
            let mut end = cursor;
            let mut chars = 0;
            while end < pending.len() && end - cursor < MAX_BATCH_ITEMS {
                let next = pending[end].text.chars().count();
                if end > cursor && chars + next > MAX_BATCH_CHARS {
                    break;
                }
                chars += next;
                end += 1;
            }
            if end == cursor {
                end += 1;
            }
            batches.push(pending[cursor..end].to_vec());
            cursor = end;
        }
        let target_lang = req.target_lang.clone();
        let mut completed_batches = stream::iter(batches.into_iter().map(|batch| {
            let target_lang = target_lang.clone();
            async move { self.translate_identified(&batch, &target_lang, title).await }
        }))
        .buffer_unordered(MAX_CONCURRENT_BATCHES);
        while let Some((output, batch_error)) = completed_batches.next().await {
            for item in output {
                let source_hash = part_hashes
                    .get(&item.id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown translation segment {}", item.id))?;
                let is_multipart = item.id.contains(":part-");
                let saved_value = if is_multipart {
                    item.text.clone()
                } else {
                    let segment = segments
                        .iter()
                        .find(|segment| segment.id == item.id)
                        .ok_or_else(|| {
                            anyhow::anyhow!("Unknown translation segment {}", item.id)
                        })?;
                    segment.restore(&item.text)?
                };
                save_segment(&item.id, source_hash, &saved_value)?;
                part_results.insert(item.id, item.text);
            }
            let mut batch_progress = Vec::new();
            for segment in &segments {
                if translations.contains_key(&segment.id) {
                    continue;
                }
                let Some(part_ids) = segment_parts.get(&segment.id) else {
                    continue;
                };
                if !part_ids
                    .iter()
                    .all(|part_id| part_results.contains_key(part_id))
                {
                    continue;
                }
                let translated = part_ids
                    .iter()
                    .map(|part_id| {
                        part_results
                            .get(part_id)
                            .map(String::as_str)
                            .ok_or_else(|| {
                                anyhow::anyhow!("Translation is missing segment {part_id}")
                            })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?
                    .concat();
                let restored = segment.restore(&translated)?;
                if part_ids.len() > 1 {
                    save_segment(&segment.id, &segment.source_hash, &restored)?;
                }
                translations.insert(segment.id.clone(), restored.clone());
                batch_progress.push((segment.id.clone(), restored));
            }
            if !batch_progress.is_empty() {
                on_batch(&batch_progress)?;
            }
            if let Some(error) = batch_error {
                return Err(error);
            }
        }

        let mut remaining_progress = Vec::new();
        for segment in &segments {
            if translations.contains_key(&segment.id) {
                continue;
            }
            let part_ids = segment_parts
                .get(&segment.id)
                .ok_or_else(|| anyhow::anyhow!("Translation is missing segment {}", segment.id))?;
            let translated_parts = part_ids
                .iter()
                .map(|part_id| {
                    part_results
                        .get(part_id)
                        .ok_or_else(|| anyhow::anyhow!("Translation is missing segment {part_id}"))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let translated = translated_parts
                .into_iter()
                .map(String::as_str)
                .collect::<String>();
            let restored = segment.restore(&translated)?;
            if part_ids.len() > 1 {
                save_segment(&segment.id, &segment.source_hash, &restored)?;
            }
            translations.insert(segment.id.clone(), restored);
            if let Some(restored) = translations.get(&segment.id) {
                remaining_progress.push((segment.id.clone(), restored.clone()));
            }
        }
        if !remaining_progress.is_empty() {
            on_batch(&remaining_progress)?;
        }

        let html = assemble_html_translation(&req.html, &segments, &translations)?;
        if !req.html.trim().is_empty() && html.trim().is_empty() {
            bail!("{} returned an empty translation", self.display_name());
        }
        Ok(TranslateResult {
            html,
            title: translated_title,
            detected_source_lang: None,
            metrics: self.take_usage(),
        })
    }

    async fn translate_texts(
        &self,
        texts: &[String],
        target_lang: &str,
        title: Option<&str>,
    ) -> anyhow::Result<Vec<String>> {
        match self.translate_texts_once(texts, target_lang, title).await {
            Ok(translations) => Ok(translations),
            Err(error) if texts.len() > 1 && is_batch_output_error(&error) => {
                // Some OpenAI/Anthropic-compatible gateways return HTTP 200 with no
                // usable text or an invalid batch shape for a multi-item translation.
                // Retry sequentially in smaller batches so one malformed response
                // does not fail the whole article.
                let midpoint = texts.len() / 2;
                let mut translations =
                    Box::pin(self.translate_texts(&texts[..midpoint], target_lang, title)).await?;
                translations.extend(
                    Box::pin(self.translate_texts(&texts[midpoint..], target_lang, title)).await?,
                );
                Ok(translations)
            }
            Err(error) => Err(error),
        }
    }

    async fn translate_texts_once(
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
        let content = self.complete(&system, &user).await?.text;
        parse_translation_array(&content, texts.len())
    }

    async fn translate_identified(
        &self,
        inputs: &[IdentifiedInput],
        target_lang: &str,
        title: Option<&str>,
    ) -> (Vec<IdentifiedInput>, Option<anyhow::Error>) {
        match tokio::time::timeout(
            MAX_BATCH_WAIT,
            self.translate_identified_inner(inputs, target_lang, title, 0),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => (
                Vec::new(),
                Some(anyhow::anyhow!(
                    "Translation batch timed out after 60 seconds. Completed paragraphs are saved; retry to continue."
                )),
            ),
        }
    }

    async fn translate_identified_inner(
        &self,
        inputs: &[IdentifiedInput],
        target_lang: &str,
        title: Option<&str>,
        depth: usize,
    ) -> (Vec<IdentifiedInput>, Option<anyhow::Error>) {
        match self
            .translate_identified_once(inputs, target_lang, title)
            .await
        {
            Ok(output) => (output, None),
            Err(error)
                if inputs.len() > 1
                    && depth < MAX_BATCH_FALLBACK_DEPTH
                    && is_batch_output_error(&error) =>
            {
                let midpoint = inputs.len() / 2;
                let (mut output, first_error) = Box::pin(self.translate_identified_inner(
                    &inputs[..midpoint],
                    target_lang,
                    title,
                    depth + 1,
                ))
                .await;
                let (second_output, second_error) = Box::pin(self.translate_identified_inner(
                    &inputs[midpoint..],
                    target_lang,
                    title,
                    depth + 1,
                ))
                .await;
                output.extend(second_output);
                (output, first_error.or(second_error))
            }
            Err(error)
                if inputs.len() == 1
                    && depth < MAX_BATCH_FALLBACK_DEPTH
                    && is_size_output_error(&error)
                    && inputs[0].text.chars().count() > 1_000 =>
            {
                let input = &inputs[0];
                let chunks = split_identified_input(&input.text, input.text.chars().count() / 2);
                let mut text = String::new();
                for (index, chunk) in chunks.into_iter().enumerate() {
                    let fragment = [IdentifiedInput {
                        id: format!("{}:fallback-{depth}-{index}", input.id),
                        text: chunk,
                    }];
                    let (output, error) = Box::pin(self.translate_identified_inner(
                        &fragment,
                        target_lang,
                        title,
                        depth + 1,
                    ))
                    .await;
                    if let Some(error) = error {
                        return (Vec::new(), Some(error));
                    }
                    for item in output {
                        text.push_str(&item.text);
                    }
                }
                (
                    vec![IdentifiedInput {
                        id: input.id.clone(),
                        text,
                    }],
                    None,
                )
            }
            Err(error) => (Vec::new(), Some(error)),
        }
    }

    async fn translate_identified_once(
        &self,
        inputs: &[IdentifiedInput],
        target_lang: &str,
        title: Option<&str>,
    ) -> anyhow::Result<Vec<IdentifiedInput>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let input = serde_json::to_string(
            &inputs
                .iter()
                .map(|item| serde_json::json!({"id":item.id,"text":item.text}))
                .collect::<Vec<_>>(),
        )?;
        let context = title
            .map(|value| format!("Article title for context: {value}\n"))
            .unwrap_or_default();
        let system = format!(
            "You are a professional translator. Translate the text into {target_lang}. Keep every ⟪PANDA_HTML_nnnn⟫ placeholder exactly once and in the same order; they protect HTML tags, links, attributes, images, and code. Translate only human-readable prose. Never add raw HTML; encode literal angle brackets as HTML entities. Treat technical names as proper nouns (Rust is a programming language and stays Rust). Do not summarize or explain. Return only a JSON object with a `translations` array of objects, each containing the original `id` and translated `text`. Return exactly one item for every input ID."
        );
        let user = format!(
            "{context}Translate each item in this JSON array. Do not change IDs or placeholders.\n{input}"
        );
        let content = self.complete(&system, &user).await?.text;
        let output = parse_identified_translations(&content, inputs)?;
        for item in &output {
            let source = inputs
                .iter()
                .find(|input| input.id == item.id)
                .expect("parser validates response IDs");
            if item.text.contains('<') || item.text.contains('>') {
                bail!("Translation response introduced unprotected HTML markup");
            }
            if html_placeholders(&source.text) != html_placeholders(&item.text) {
                bail!(
                    "Translation response has missing, duplicated, or reordered HTML placeholders"
                );
            }
        }
        Ok(output)
    }

    async fn complete(&self, system: &str, user: &str) -> anyhow::Result<LlmResponse> {
        let gate = request_gate(&self.cache_id());
        for attempt in 0..=MAX_RATE_LIMIT_RETRIES {
            let request = match self.api {
                LlmApi::OpenAiCompatible => self
                    .client
                    .post(api_endpoint(&self.base_url, "/chat/completions"))
                    .bearer_auth(&self.api_key)
                    .json(&self.translation_payload(serde_json::json!({
                        "model": self.model,
                        "messages": [
                            { "role": "system", "content": system },
                            { "role": "user", "content": user },
                        ],
                        "max_tokens": MAX_OUTPUT_TOKENS,
                    }))),
                LlmApi::Anthropic => self
                    .client
                    .post(api_endpoint(&self.base_url, "/messages"))
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", "2023-06-01")
                    .json(&self.translation_payload(serde_json::json!({
                        "model": self.model,
                        "max_tokens": MAX_OUTPUT_TOKENS,
                        "system": system,
                        "messages": [{ "role": "user", "content": user }],
                    }))),
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
                }
            };
            let queued_at = std::time::Instant::now();
            let permit = gate.acquire().await.map_err(|_| {
                anyhow::anyhow!("Translation provider request queue is unavailable")
            })?;
            let queue_ms = queued_at.elapsed().as_millis();
            let requested_at = std::time::Instant::now();
            self.record_attempt();
            let response = request
                .send()
                .await
                .context("Translation provider request failed")?;
            let status = response.status();
            let request_id_header = response
                .headers()
                .get("x-request-id")
                .or_else(|| response.headers().get("request-id"))
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs);
            let body = response.text().await.unwrap_or_default();
            drop(permit);
            if status.as_u16() == 429 {
                eprintln!(
                    "translation rate limited: provider={} model={} request_id={} retry_after_seconds={}",
                    self.api.id(),
                    self.model,
                    request_id_header.as_deref().unwrap_or("unknown"),
                    retry_after
                        .map_or_else(|| "default".to_owned(), |delay| delay.as_secs().to_string())
                );
                if attempt == MAX_RATE_LIMIT_RETRIES {
                    bail!("{}", api_error_message(self.api, 429, &body));
                }
                if retry_after.is_some_and(|delay| delay > MAX_RETRY_AFTER) {
                    bail!(
                        "Translation provider rate limit reached; retry after more than 30 seconds. Try again later."
                    );
                }
                tokio::time::sleep(
                    retry_after
                        .unwrap_or_else(|| Duration::from_secs(if attempt == 0 { 1 } else { 3 })),
                )
                .await;
                continue;
            }
            if !status.is_success() {
                let error = api_error_message(self.api, status.as_u16(), &body);
                eprintln!(
                    "translation request failed: provider={} model={} status={} request_id={}",
                    self.api.id(),
                    self.model,
                    status.as_u16(),
                    request_id_header.as_deref().unwrap_or("unknown")
                );
                bail!("{error}");
            }
            let value: Value = serde_json::from_str(&body)
                .with_context(|| format!("Invalid {} response", self.api.display_name()))?;
            let request_id = request_id_header
                .or_else(|| value.get("id").and_then(Value::as_str).map(str::to_owned));
            let finish_reason = response_finish_reason(self.api, &value);
            let (input_tokens, output_tokens) = response_token_usage(self.api, &value);
            self.record_response_tokens(input_tokens, output_tokens);
            if finish_reason
                .as_deref()
                .is_some_and(is_truncated_finish_reason)
            {
                eprintln!(
                    "translation response truncated: provider={} model={} request_id={} finish_reason={} input_tokens={} output_tokens={}",
                    self.api.id(),
                    self.model,
                    request_id.as_deref().unwrap_or("unknown"),
                    finish_reason.as_deref().unwrap_or("unknown"),
                    input_tokens.map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                    output_tokens.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
                );
                bail!(
                    "Translation response truncated at the output-token limit (finish_reason={})",
                    finish_reason.as_deref().unwrap_or("unknown")
                );
            }
            let text = match response_text(self.api, &value) {
                Ok(text) => text,
                Err(error) => {
                    eprintln!(
                        "translation response unusable: provider={} model={} request_id={} finish_reason={} input_tokens={} output_tokens={}",
                        self.api.id(),
                        self.model,
                        request_id.as_deref().unwrap_or("unknown"),
                        finish_reason.as_deref().unwrap_or("unknown"),
                        input_tokens
                            .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                        output_tokens
                            .map_or_else(|| "unknown".to_owned(), |value| value.to_string())
                    );
                    return Err(error);
                }
            };
            eprintln!(
                "translation request complete: provider={} model={} request_id={} finish_reason={} input_tokens={} output_tokens={} queue_ms={} request_ms={}",
                self.api.id(),
                self.model,
                request_id.as_deref().unwrap_or("unknown"),
                finish_reason.as_deref().unwrap_or("unknown"),
                input_tokens.map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                output_tokens.map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
                queue_ms,
                requested_at.elapsed().as_millis()
            );
            return Ok(LlmResponse { text });
        }
        unreachable!("rate limit retry loop always returns")
    }

    fn translation_payload(&self, mut payload: Value) -> Value {
        // DeepSeek defaults to high-effort thinking on both compatible APIs.
        // Translation needs the text output budget, without hidden reasoning.
        if reqwest::Url::parse(&self.base_url)
            .ok()
            .is_some_and(|url| url.host_str() == Some("api.deepseek.com"))
        {
            payload["thinking"] = serde_json::json!({ "type": "disabled" });
        }
        payload
    }

    pub(crate) async fn translate_titles(
        &self,
        titles: &[String],
        target_lang: &str,
    ) -> Result<TitleBatchResult, TitleBatchFailure> {
        let mut translations = Vec::with_capacity(titles.len());
        let mut characters = 0;
        for batch in titles.chunks(MAX_BATCH_ITEMS) {
            let batch_chars = batch
                .iter()
                .map(|title| title.chars().count())
                .sum::<usize>();
            match self.translate_texts(batch, target_lang, None).await {
                Ok(parts) => translations.extend(parts),
                Err(error) => {
                    let usage = self.take_usage();
                    return Err(TitleBatchFailure {
                        requests: usage.requests as usize,
                        characters: characters + batch_chars,
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        error,
                    });
                }
            }
            characters += batch_chars;
        }
        let usage = self.take_usage();
        Ok(TitleBatchResult {
            translations,
            requests: usage.requests as usize,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
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
        self.translate_with_cached_segments(req, HashMap::new(), |_, _, _| Ok(()))
            .await
    }
}

fn is_size_output_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("exceeds the model context limit")
        || message.contains("truncated at the output-token limit")
}

fn is_batch_output_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("returned an empty response")
        || message.contains("Translation response must contain a translations array")
        || message.starts_with("Model returned ")
        || message.contains("response is not JSON")
        || message.contains("invalid JSON response")
        || message.contains("Translation response has")
        || message.contains("placeholder")
        || message.contains("unprotected HTML markup")
        || message.contains("exceeds the model context limit")
        || message.contains("truncated at the output-token limit")
}

fn parse_identified_translations(
    content: &str,
    inputs: &[IdentifiedInput],
) -> anyhow::Result<Vec<IdentifiedInput>> {
    let content = strip_json_fence(content);
    let parsed = parse_json_object(content)?;
    let values = parsed
        .get("translations")
        .and_then(Value::as_array)
        .context("Translation response must contain a translations array")?;
    let expected: HashSet<_> = inputs.iter().map(|item| item.id.as_str()).collect();
    let mut output = Vec::with_capacity(values.len());
    let mut seen = HashSet::new();
    for item in values {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .context("Translation response has an item without an id")?;
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .context("Translation response has an item without text")?;
        if !expected.contains(id) {
            bail!("Translation response has an unexpected segment id");
        }
        if !seen.insert(id.to_owned()) {
            bail!("Translation response has a duplicate segment id");
        }
        if text.trim().is_empty() {
            bail!("Translation response has an empty segment");
        }
        output.push(IdentifiedInput {
            id: id.to_owned(),
            text: text.to_owned(),
        });
    }
    if seen.len() != expected.len() {
        bail!("Translation response has missing segment ids");
    }
    Ok(output)
}

fn strip_json_fence(content: &str) -> &str {
    let content = content.trim();
    content
        .strip_prefix("```json")
        .or_else(|| content.strip_prefix("```JSON"))
        .or_else(|| content.strip_prefix("```"))
        .and_then(|content| content.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(content)
}

fn parse_json_object(content: &str) -> anyhow::Result<Value> {
    serde_json::from_str(content).or_else(|_| {
        let start = content.find('{').context("response is not JSON")?;
        let end = content.rfind('}').context("response is not JSON")?;
        serde_json::from_str(&content[start..=end]).context("invalid JSON response")
    })
}

fn request_gate(key: &str) -> Arc<Semaphore> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();
    let gates = GATES.get_or_init(|| Mutex::new(HashMap::new()));
    gates
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(key.to_owned())
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_BATCHES)))
        .clone()
}

fn response_finish_reason(api: LlmApi, value: &Value) -> Option<String> {
    let path = match api {
        LlmApi::OpenAiCompatible => "/choices/0/finish_reason",
        LlmApi::Anthropic => "/stop_reason",
        LlmApi::Gemini => "/candidates/0/finishReason",
    };
    value
        .pointer(path)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn is_truncated_finish_reason(reason: &str) -> bool {
    matches!(
        reason.to_ascii_lowercase().as_str(),
        "length" | "max_tokens" | "max_output_tokens"
    )
}

fn response_token_usage(api: LlmApi, value: &Value) -> (Option<u64>, Option<u64>) {
    let paths = match api {
        LlmApi::OpenAiCompatible => ("/usage/prompt_tokens", "/usage/completion_tokens"),
        LlmApi::Anthropic => ("/usage/input_tokens", "/usage/output_tokens"),
        LlmApi::Gemini => (
            "/usageMetadata/promptTokenCount",
            "/usageMetadata/candidatesTokenCount",
        ),
    };
    (
        value.pointer(paths.0).and_then(Value::as_u64),
        value.pointer(paths.1).and_then(Value::as_u64),
    )
}

fn html_placeholders(text: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("⟪PANDA_HTML_") {
        let after = &rest[start..];
        let Some(end) = after.find('⟫') else {
            output.push(after);
            break;
        };
        let token_end = end + '⟫'.len_utf8();
        output.push(&after[..token_end]);
        rest = &after[token_end..];
    }
    output
}

fn split_identified_input(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    if text.chars().count() <= max_chars {
        return vec![text.to_owned()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    let mut cursor = 0usize;
    while cursor < text.len() {
        let rest = &text[cursor..];
        let unit_end = if rest.starts_with("⟪PANDA_HTML_") {
            rest.find('⟫')
                .map(|end| cursor + end + '⟫'.len_utf8())
                .unwrap_or_else(|| cursor + rest.chars().next().unwrap().len_utf8())
        } else {
            cursor + rest.chars().next().unwrap().len_utf8()
        };
        let unit = &text[cursor..unit_end];
        let unit_chars = unit.chars().count();
        let is_space = unit.chars().all(char::is_whitespace);
        if !current.is_empty()
            && ((current_chars >= max_chars)
                || (is_space && current_chars >= max_chars.saturating_sub(200)))
        {
            current.push_str(unit);
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
        } else if !current.is_empty() && current_chars + unit_chars > max_chars {
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
            current.push_str(unit);
            current_chars += unit_chars;
        } else {
            current.push_str(unit);
            current_chars += unit_chars;
        }
        cursor = unit_end;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
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
    if let Some(message) = value
        .pointer("/error/message")
        .or_else(|| value.pointer("/error/error/message"))
        .and_then(Value::as_str)
    {
        let lower_message = message.to_ascii_lowercase();
        if lower_message.contains("rate_limit")
            || lower_message.contains("rate limit")
            || lower_message.contains("too many requests")
        {
            bail!("Translation provider rate limit reached. Wait a moment before retrying.");
        }
        bail!("{} request failed: {message}", api.display_name());
    }
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
    if status == 429 {
        return "Translation provider rate limit reached (HTTP 429). Wait a moment before retrying."
            .to_owned();
    }
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
    let lower_detail = detail.to_ascii_lowercase();
    if status == 413
        || lower_detail.contains("context length")
        || lower_detail.contains("maximum context")
        || lower_detail.contains("prompt is too long")
        || lower_detail.contains("too many tokens")
    {
        return format!("Translation segment exceeds the model context limit ({status}): {detail}");
    }
    if lower_detail.contains("rate_limit")
        || lower_detail.contains("rate limit")
        || lower_detail.contains("too many requests")
    {
        return format!(
            "Translation provider rate limit reached. Wait a moment before retrying. ({})",
            detail
        );
    }
    if detail.trim().is_empty() {
        format!("{} request failed ({status})", api.display_name())
    } else {
        format!("{} request failed ({status}): {detail}", api.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead as _, Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn inputs() -> Vec<IdentifiedInput> {
        vec![
            IdentifiedInput {
                id: "a".into(),
                text: "first".into(),
            },
            IdentifiedInput {
                id: "b".into(),
                text: "second".into(),
            },
        ]
    }

    #[test]
    fn identified_responses_map_by_id_when_model_reorders_items() {
        let parsed = parse_identified_translations(
            r#"{"translations":[{"id":"b","text":"第二"},{"id":"a","text":"第一"}]}"#,
            &inputs(),
        )
        .unwrap();
        assert_eq!(parsed[0].id, "b");
        assert_eq!(parsed[1].id, "a");
    }

    #[test]
    fn identified_responses_reject_missing_duplicate_unknown_and_empty_items() {
        for response in [
            r#"{"translations":[{"id":"a","text":"第一"}]}"#,
            r#"{"translations":[{"id":"a","text":"一"},{"id":"a","text":"二"}]}"#,
            r#"{"translations":[{"id":"a","text":"一"},{"id":"x","text":"二"}]}"#,
            r#"{"translations":[{"id":"a","text":""},{"id":"b","text":"二"}]}"#,
        ] {
            assert!(parse_identified_translations(response, &inputs()).is_err());
        }
    }

    #[test]
    fn html_placeholder_order_is_checked_exactly() {
        let a = "⟪PANDA_HTML_0000⟫";
        let b = "⟪PANDA_HTML_0001⟫";
        assert_eq!(html_placeholders(&format!("{a}text{b}")), vec![a, b]);
        assert_ne!(
            html_placeholders(&format!("{a}{b}")),
            html_placeholders(&format!("{b}{a}"))
        );
        assert_ne!(html_placeholders(&format!("{a}{a}")), vec![a]);
    }

    #[test]
    fn oversized_segment_splits_without_breaking_markup_tokens() {
        let token = "⟪PANDA_HTML_0000⟫";
        let source = format!(
            "{} {token} {} {token}",
            "word ".repeat(20),
            "text ".repeat(20)
        );
        let chunks = split_identified_input(&source, 40);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), source);
        assert_eq!(
            chunks
                .iter()
                .flat_map(|chunk| html_placeholders(chunk))
                .collect::<Vec<_>>(),
            vec![token, token]
        );
    }

    #[test]
    fn provider_errors_explain_rate_limit_and_context_limit() {
        assert!(api_error_message(LlmApi::Anthropic, 429, "").contains("rate limit"));
        assert!(
            api_error_message(
                LlmApi::OpenAiCompatible,
                400,
                r#"{"error":{"message":"maximum context length exceeded"}}"#
            )
            .contains("exceeds the model context limit")
        );
        assert!(is_truncated_finish_reason("length"));
        assert!(is_truncated_finish_reason("MAX_TOKENS"));
        assert!(!is_truncated_finish_reason("stop"));
    }

    fn mock_openai_server(content: String) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    content_length = value.trim().parse().unwrap();
                }
            }
            let mut request_body = vec![0; content_length];
            reader.read_exact(&mut request_body).unwrap();
            let body = serde_json::json!({
                "id": "mock-request",
                "choices": [{ "finish_reason": "stop", "message": { "content": content } }],
                "usage": { "prompt_tokens": 12, "completion_tokens": 7 }
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        (format!("http://{address}/v1"), handle)
    }

    fn mock_rate_limit_then_success() -> (String, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let handle = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        content_length = value.trim().parse().unwrap();
                    }
                }
                let mut request_body = vec![0; content_length];
                reader.read_exact(&mut request_body).unwrap();
                seen.fetch_add(1, Ordering::SeqCst);
                let (status, headers, body) = if index == 0 {
                    (
                        "429 Too Many Requests",
                        "Retry-After: 0\r\n",
                        "{}".to_owned(),
                    )
                } else {
                    (
                        "200 OK",
                        "",
                        serde_json::json!({
                            "choices": [{"message":{"content":"{\"translations\":[\"你好\"]}"}}],
                            "usage":{"prompt_tokens":3,"completion_tokens":2}
                        })
                        .to_string(),
                    )
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        (format!("http://{address}/v1"), requests, handle)
    }

    fn mock_scripted_openai_server(
        responses: Vec<(String, String)>,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            for (status, content) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        content_length = value.trim().parse().unwrap();
                    }
                }
                let mut request_body = vec![0; content_length];
                reader.read_exact(&mut request_body).unwrap();
                let body = if status.starts_with("200")
                    && !serde_json::from_str::<Value>(&content)
                        .ok()
                        .is_some_and(|value| value.get("choices").is_some())
                {
                    serde_json::json!({
                        "choices": [{"message":{"content":content}}]
                    })
                    .to_string()
                } else {
                    content
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        (format!("http://{address}/v1"), handle)
    }

    #[tokio::test]
    async fn rate_limit_retry_is_bounded_and_obeys_retry_after() {
        let (base_url, requests, server) = mock_rate_limit_then_success();
        let translator = LlmTranslator::new(
            LlmApi::OpenAiCompatible,
            &base_url,
            "test-key",
            "test-model",
        )
        .unwrap();
        let result = translator
            .translate_titles(&["hello".into()], "zh-Hans")
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(result.translations, ["你好"]);
        assert_eq!(result.requests, 2);
        assert_eq!(result.input_tokens, Some(3));
        assert_eq!(result.output_tokens, Some(2));
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn translation_disables_thinking_only_for_the_deepseek_endpoint() {
        for api in [LlmApi::OpenAiCompatible, LlmApi::Anthropic] {
            for (url, expected) in [
                ("https://api.deepseek.com/anthropic", true),
                ("https://api.anthropic.com/v1", false),
                ("https://api.deepseek.com.example.org", false),
            ] {
                let translator =
                    LlmTranslator::new(api, url, "test-key", "hand-entered-model").unwrap();
                let payload =
                    translator.translation_payload(serde_json::json!({"max_tokens": 8192}));
                assert_eq!(payload.get("thinking").is_some(), expected);
                if expected {
                    assert_eq!(payload["thinking"]["type"], "disabled");
                }
            }
        }
    }

    #[tokio::test]
    async fn truncated_single_segment_is_split_without_losing_its_id_or_placeholders() {
        let input = IdentifiedInput {
            id: "paragraph".into(),
            text: format!("⟪PANDA_HTML_0000⟫{}⟪PANDA_HTML_0001⟫", "A ".repeat(700)),
        };
        let mut responses = vec![(
            "200 OK".into(),
            serde_json::json!({
                "choices": [{"finish_reason": "length", "message": {"content": ""}}]
            })
            .to_string(),
        )];
        let chunks = split_identified_input(&input.text, input.text.chars().count() / 2);
        for (index, chunk) in chunks.into_iter().enumerate() {
            responses.push((
                "200 OK".into(),
                serde_json::json!({
                    "translations": [{
                        "id": format!("paragraph:fallback-0-{index}"),
                        "text": chunk.replace("A ", "甲 "),
                    }]
                })
                .to_string(),
            ));
        }
        let (url, server) = mock_scripted_openai_server(responses);
        let translator =
            LlmTranslator::new(LlmApi::OpenAiCompatible, &url, "test-key", "test-model").unwrap();
        let (output, error) = translator
            .translate_identified(&[input.clone()], "zh-Hans", None)
            .await;
        server.join().unwrap();
        assert!(error.is_none(), "{error:?}");
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].id, input.id);
        assert_eq!(
            html_placeholders(&output[0].text),
            html_placeholders(&input.text)
        );
        assert_eq!(output[0].text.matches("甲").count(), 700);
    }

    #[tokio::test]
    async fn progress_emits_all_completed_segments_in_one_batch() {
        let source = (0..8)
            .map(|index| format!("<p>Paragraph {index}</p>"))
            .collect::<String>();
        let segments = prepare_html_translation_segments(&source);
        let response = serde_json::json!({
            "translations": segments.iter().rev().map(|segment| serde_json::json!({
                "id": segment.id,
                "text": segment.input.replace("Paragraph", "段落"),
            })).collect::<Vec<_>>()
        })
        .to_string();
        let (url, server) = mock_openai_server(response);
        let translator =
            LlmTranslator::new(LlmApi::OpenAiCompatible, &url, "test-key", "test-model").unwrap();
        let mut updates = Vec::new();
        let result = translator
            .translate_with_cached_segments_and_progress(
                TranslateRequest {
                    html: source,
                    title: None,
                    target_lang: "zh-Hans".into(),
                },
                HashMap::new(),
                |_, _, _| Ok(()),
                |batch| {
                    updates.push(batch.to_vec());
                    Ok(())
                },
            )
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].len(), 8);
        for (index, (id, text)) in updates[0].iter().enumerate() {
            assert_eq!(id, &segments[index].id);
            assert_eq!(text, &format!("<p>段落 {index}</p>"));
        }
        assert_eq!(result.html.matches("段落").count(), 8);
    }

    #[tokio::test]
    async fn malformed_batch_fallback_saves_successful_items_before_a_later_failure() {
        let source = "<p>First</p><p>Second</p>";
        let segments = prepare_html_translation_segments(source);
        let first_success = serde_json::json!({
            "translations": [{
                "id": segments[0].id,
                "text": segments[0].input.replace("First", "第一"),
            }]
        })
        .to_string();
        let (base_url, server) = mock_scripted_openai_server(vec![
            ("200 OK".into(), r#"{"translations":[]}"#.into()),
            ("200 OK".into(), first_success),
            (
                "500 Internal Server Error".into(),
                r#"{"error":{"message":"temporary outage"}}"#.into(),
            ),
        ]);
        let translator = LlmTranslator::new(
            LlmApi::OpenAiCompatible,
            &base_url,
            "test-key",
            "test-model",
        )
        .unwrap();
        let saved = Arc::new(Mutex::new(HashMap::<String, String>::new()));
        let saved_callback = saved.clone();
        let result = translator
            .translate_with_cached_segments(
                TranslateRequest {
                    html: source.into(),
                    title: None,
                    target_lang: "zh-Hans".into(),
                },
                HashMap::new(),
                move |id, _, value| {
                    saved_callback
                        .lock()
                        .unwrap()
                        .insert(id.to_owned(), value.to_owned());
                    Ok(())
                },
            )
            .await;
        server.join().unwrap();
        assert!(result.is_err());
        assert_eq!(
            saved
                .lock()
                .unwrap()
                .get(&segments[0].id)
                .map(String::as_str),
            Some("<p>第一</p>")
        );
        assert!(!saved.lock().unwrap().contains_key(&segments[1].id));
    }

    #[tokio::test]
    async fn resumable_translation_maps_ids_preserves_markup_and_reuses_saved_segments() {
        let source = r#"<p>Rust is useful <a href="https://example.com">here</a>.</p>"#;
        let segments = prepare_html_translation_segments(source);
        let response = serde_json::json!({
            "translations": segments.iter().map(|segment| serde_json::json!({
                "id": segment.id,
                "text": segment.input.replace("Rust is useful", "Rust 很有用"),
            })).collect::<Vec<_>>()
        })
        .to_string();
        let (base_url, server) = mock_openai_server(response);
        let translator = LlmTranslator::new(
            LlmApi::OpenAiCompatible,
            &base_url,
            "test-key",
            "test-model",
        )
        .unwrap();
        let saved = Arc::new(Mutex::new(HashMap::<String, String>::new()));
        let saved_callback = saved.clone();
        let translated = translator
            .translate_with_cached_segments(
                TranslateRequest {
                    html: source.into(),
                    title: None,
                    target_lang: "zh-Hans".into(),
                },
                HashMap::new(),
                move |id, source_hash, value| {
                    assert!(!source_hash.is_empty());
                    saved_callback
                        .lock()
                        .unwrap()
                        .insert(id.to_owned(), value.to_owned());
                    Ok(())
                },
            )
            .await
            .unwrap();
        server.join().unwrap();
        assert!(translated.html.contains("Rust 很有用"));
        assert!(translated.html.contains("href=\"https://example.com\""));
        assert_eq!(translated.metrics.requests, 1);
        assert_eq!(translated.metrics.input_tokens, Some(12));
        assert_eq!(translated.metrics.output_tokens, Some(7));

        let saved = saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        let second = translator
            .translate_with_cached_segments(
                TranslateRequest {
                    html: source.into(),
                    title: None,
                    target_lang: "zh-Hans".into(),
                },
                saved,
                |_, _, _| panic!("cached segment should not be requested again"),
            )
            .await
            .unwrap();
        assert_eq!(second.html, translated.html);
        assert_eq!(second.metrics.requests, 0);
    }
}
