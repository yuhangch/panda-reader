use anyhow::bail;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

mod azure;
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
    DeepL,
    LibreTranslate,
}

impl Provider {
    pub const ALL: [Self; 4] = [
        Self::Azure,
        Self::Volcengine,
        Self::DeepL,
        Self::LibreTranslate,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Azure => "azure",
            Self::Volcengine => "volcengine",
            Self::DeepL => "deepl",
            Self::LibreTranslate => "libretranslate",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Azure => "Azure Translator",
            Self::Volcengine => "Volcengine",
            Self::DeepL => "DeepL",
            Self::LibreTranslate => "LibreTranslate",
        }
    }

    pub fn is_ready(self) -> bool {
        matches!(self, Self::Azure | Self::Volcengine)
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
}

impl Default for TranslatorConfig {
    fn default() -> Self {
        Self {
            provider: Provider::Azure,
            azure_key: String::new(),
            azure_region: "eastasia".into(),
            volcengine_access_key: String::new(),
            volcengine_secret_key: String::new(),
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
            Provider::DeepL | Provider::LibreTranslate => false,
        }
    }
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
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Azure(translator) => translator.display_name(),
            Self::Volcengine(translator) => translator.display_name(),
        }
    }

    pub async fn translate(&self, req: TranslateRequest) -> anyhow::Result<TranslateResult> {
        match self {
            Self::Azure(translator) => translator.translate(req).await,
            Self::Volcengine(translator) => translator.translate(req).await,
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
