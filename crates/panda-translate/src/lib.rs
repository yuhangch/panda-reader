use anyhow::bail;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

mod azure;
mod llm;
pub mod html {
    pub use panda_content::html::*;
}
mod volcengine;

pub use azure::AzureTranslator;
pub use html::{
    HtmlTranslation, TranslationInput, block_needs_translation, source_hash, split_blocks,
    split_text_for_translate, translate_html_blocks, translation_cache_hash,
    translation_revision_hash,
};
pub use llm::{LlmApi, LlmTranslator};
pub use volcengine::VolcengineTranslator;

pub fn title_source_hash(title: &str) -> String {
    hex::encode(Sha256::digest(title.as_bytes()))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Azure,
    Volcengine,
    OpenAiCompatible,
    Anthropic,
    Gemini,
    DeepL,
    LibreTranslate,
}

impl Provider {
    pub const ALL: [Self; 7] = [
        Self::Azure,
        Self::Volcengine,
        Self::OpenAiCompatible,
        Self::Anthropic,
        Self::Gemini,
        Self::DeepL,
        Self::LibreTranslate,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Azure => "azure",
            Self::Volcengine => "volcengine",
            Self::OpenAiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::DeepL => "deepl",
            Self::LibreTranslate => "libretranslate",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Azure => "Azure Translator",
            Self::Volcengine => "Volcengine",
            Self::OpenAiCompatible => "OpenAI-compatible",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Gemini",
            Self::DeepL => "DeepL",
            Self::LibreTranslate => "LibreTranslate",
        }
    }

    pub fn is_ready(self) -> bool {
        matches!(
            self,
            Self::Azure
                | Self::Volcengine
                | Self::OpenAiCompatible
                | Self::Anthropic
                | Self::Gemini
        )
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|provider| *provider == self)
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslatorConfig {
    pub provider: Provider,
    pub azure_key: String,
    pub azure_region: String,
    pub volcengine_access_key: String,
    pub volcengine_secret_key: String,
    pub openai_url: String,
    pub openai_key: String,
    pub openai_model: String,
    pub anthropic_url: String,
    pub anthropic_key: String,
    pub anthropic_model: String,
    pub gemini_url: String,
    pub gemini_key: String,
    pub gemini_model: String,
}

impl Default for TranslatorConfig {
    fn default() -> Self {
        Self {
            provider: Provider::Azure,
            azure_key: String::new(),
            azure_region: "eastasia".into(),
            volcengine_access_key: String::new(),
            volcengine_secret_key: String::new(),
            openai_url: "https://api.openai.com/v1".into(),
            openai_key: String::new(),
            openai_model: "gpt-4.1-mini".into(),
            anthropic_url: "https://api.anthropic.com/v1".into(),
            anthropic_key: String::new(),
            anthropic_model: "claude-haiku-4-5-20251001".into(),
            gemini_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            gemini_key: String::new(),
            gemini_model: "gemini-2.5-flash".into(),
        }
    }
}

impl TranslatorConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)?;
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

    pub fn is_configured(&self) -> bool {
        match self.provider {
            Provider::Azure => !self.azure_key.trim().is_empty(),
            Provider::Volcengine => {
                !self.volcengine_access_key.trim().is_empty()
                    && !self.volcengine_secret_key.trim().is_empty()
            }
            Provider::OpenAiCompatible => {
                !self.openai_url.trim().is_empty()
                    && !self.openai_key.trim().is_empty()
                    && !self.openai_model.trim().is_empty()
            }
            Provider::Anthropic => {
                !self.anthropic_url.trim().is_empty()
                    && !self.anthropic_key.trim().is_empty()
                    && !self.anthropic_model.trim().is_empty()
            }
            Provider::Gemini => {
                !self.gemini_url.trim().is_empty()
                    && !self.gemini_key.trim().is_empty()
                    && !self.gemini_model.trim().is_empty()
            }
            Provider::DeepL | Provider::LibreTranslate => false,
        }
    }

    pub fn cache_id(&self) -> String {
        match self.provider {
            Provider::Azure => "azure".into(),
            Provider::Volcengine => "volcengine".into(),
            Provider::OpenAiCompatible => translation_backend_cache_id(
                "openai-compatible",
                &self.openai_url,
                &self.openai_model,
            ),
            Provider::Anthropic => translation_backend_cache_id(
                "anthropic",
                &self.anthropic_url,
                &self.anthropic_model,
            ),
            Provider::Gemini => {
                translation_backend_cache_id("gemini", &self.gemini_url, &self.gemini_model)
            }
            Provider::DeepL => "deepl".into(),
            Provider::LibreTranslate => "libretranslate".into(),
        }
    }
}

pub(crate) fn translation_backend_cache_id(id: &str, url: &str, model: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    hasher.update([0]);
    hasher.update(url.trim().trim_end_matches('/').as_bytes());
    hasher.update([0]);
    hasher.update(model.trim().as_bytes());
    format!("{id}:{}", hex::encode(hasher.finalize()))
}

#[derive(Clone, Debug)]
pub struct TranslateRequest {
    pub html: String,
    pub title: Option<String>,
    pub target_lang: String,
}

#[derive(Clone, Debug)]
pub struct TranslateResult {
    pub html: String,
    pub title: Option<String>,
    pub detected_source_lang: Option<String>,
}

/// Thin provider contract. New backends implement this; the app builds one via [`build`].
pub trait Translator: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn translate(
        &self,
        req: TranslateRequest,
    ) -> impl std::future::Future<Output = anyhow::Result<TranslateResult>> + Send;
}

pub enum AnyTranslator {
    Azure(AzureTranslator),
    Volcengine(VolcengineTranslator),
    OpenAiCompatible(LlmTranslator),
    Anthropic(LlmTranslator),
    Gemini(LlmTranslator),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleBatchResult {
    pub translations: Vec<String>,
    pub requests: usize,
}

#[derive(Debug)]
pub struct TitleBatchFailure {
    pub requests: usize,
    pub characters: usize,
    pub error: anyhow::Error,
}

impl AnyTranslator {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Azure(translator) => translator.id(),
            Self::Volcengine(translator) => translator.id(),
            Self::OpenAiCompatible(translator)
            | Self::Anthropic(translator)
            | Self::Gemini(translator) => translator.id(),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Azure(translator) => translator.display_name(),
            Self::Volcengine(translator) => translator.display_name(),
            Self::OpenAiCompatible(translator)
            | Self::Anthropic(translator)
            | Self::Gemini(translator) => translator.display_name(),
        }
    }

    pub async fn translate(&self, req: TranslateRequest) -> anyhow::Result<TranslateResult> {
        match self {
            Self::Azure(translator) => translator.translate(req).await,
            Self::Volcengine(translator) => translator.translate(req).await,
            Self::OpenAiCompatible(translator)
            | Self::Anthropic(translator)
            | Self::Gemini(translator) => translator.translate(req).await,
        }
    }

    pub async fn translate_titles(
        &self,
        titles: &[String],
        target_lang: &str,
    ) -> Result<TitleBatchResult, TitleBatchFailure> {
        match self {
            Self::Azure(translator) => translator.translate_titles(titles, target_lang).await,
            Self::Volcengine(translator) => translator.translate_titles(titles, target_lang).await,
            Self::OpenAiCompatible(translator)
            | Self::Anthropic(translator)
            | Self::Gemini(translator) => translator.translate_titles(titles, target_lang).await,
        }
    }

    pub fn cache_id(&self) -> String {
        match self {
            Self::Azure(_) => "azure".into(),
            Self::Volcengine(_) => "volcengine".into(),
            Self::OpenAiCompatible(translator)
            | Self::Anthropic(translator)
            | Self::Gemini(translator) => translator.cache_id(),
        }
    }
}

pub fn build(config: &TranslatorConfig) -> anyhow::Result<AnyTranslator> {
    match config.provider {
        Provider::Azure => Ok(AnyTranslator::Azure(AzureTranslator::new(
            config.azure_key.trim(),
            config.azure_region.trim(),
        )?)),
        Provider::Volcengine => Ok(AnyTranslator::Volcengine(VolcengineTranslator::new(
            config.volcengine_access_key.trim(),
            config.volcengine_secret_key.trim(),
        )?)),
        Provider::OpenAiCompatible => Ok(AnyTranslator::OpenAiCompatible(LlmTranslator::new(
            LlmApi::OpenAiCompatible,
            &config.openai_url,
            &config.openai_key,
            &config.openai_model,
        )?)),
        Provider::Anthropic => Ok(AnyTranslator::Anthropic(LlmTranslator::new(
            LlmApi::Anthropic,
            &config.anthropic_url,
            &config.anthropic_key,
            &config.anthropic_model,
        )?)),
        Provider::Gemini => Ok(AnyTranslator::Gemini(LlmTranslator::new(
            LlmApi::Gemini,
            &config.gemini_url,
            &config.gemini_key,
            &config.gemini_model,
        )?)),
        Provider::DeepL => bail!("DeepL is not available yet"),
        Provider::LibreTranslate => bail!("LibreTranslate is not available yet"),
    }
}

/// Map Panda Reader UI language ids to translator target codes.
pub fn target_lang_for_ui(language: &str) -> &'static str {
    match language {
        "zh_cn" => "zh-Hans",
        "zh_tw" => "zh-Hant",
        "japanese" => "ja",
        "french" => "fr",
        "german" => "de",
        _ => "en",
    }
}

pub fn target_lang_from_serde_language(language: impl AsRef<str>) -> String {
    target_lang_for_ui(language.as_ref()).to_owned()
}
