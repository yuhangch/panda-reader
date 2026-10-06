#[derive(Clone, Debug)]
pub struct Feed {
    pub id: i64,
    pub title: String,
    pub feed_url: String,
    pub site_url: Option<String>,
    pub folder: Option<String>,
    pub unread: i64,
    pub last_error: Option<String>,
    pub auto_translate_titles: bool,
}

#[derive(Clone, Debug)]
pub struct ArticleSummary {
    pub id: i64,
    pub feed_title: String,
    /// Language declared by this article's feed, when the provider exposes it.
    pub feed_language: Option<String>,
    pub feed_auto_translate_titles: bool,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub snippet: String,
    pub published_at: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub read_later: bool,
    pub auto_translated_title: Option<String>,
    pub auto_translated_title_lang: Option<String>,
    pub auto_translated_title_source_hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranslationUsage {
    pub day: String,
    pub provider: String,
    pub requests: u64,
    pub characters: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentExtractor {
    /// Mozilla Readability-style via `dom_smoothie` (current default).
    #[default]
    DomSmoothie,
    /// Defuddle-style extraction (`decruft` Rust port).
    Decruft,
    /// Trafilatura-style extraction (`rs-trafilatura`).
    Trafilatura,
    /// CSS-selector heuristics only (no Readability library).
    Heuristic,
}

impl ContentExtractor {
    pub const ALL: [Self; 4] = [
        Self::DomSmoothie,
        Self::Decruft,
        Self::Trafilatura,
        Self::Heuristic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::DomSmoothie => "Readability (dom_smoothie)",
            Self::Decruft => "Defuddle (decruft)",
            Self::Trafilatura => "Trafilatura",
            Self::Heuristic => "Heuristic",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::DomSmoothie => Self::Decruft,
            Self::Decruft => Self::Trafilatura,
            Self::Trafilatura => Self::Heuristic,
            Self::Heuristic => Self::DomSmoothie,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationLayout {
    /// Original block + translation underneath (Immersive Translate style).
    #[default]
    Immersive,
    /// Replace the article with the translation only.
    Replaced,
}

impl TranslationLayout {
    pub fn label(self) -> &'static str {
        match self {
            Self::Immersive => "Immersive",
            Self::Replaced => "Translation only",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Article {
    pub summary: ArticleSummary,
    pub url: Option<String>,
    pub content_html: String,
    pub extracted_html: Option<String>,
    pub translated_html: Option<String>,
    pub translated_title: Option<String>,
    pub translated_lang: Option<String>,
    /// SHA-256 hex of the HTML+title that produced the cached translation.
    pub translation_source_hash: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    All,
    Unread,
    Starred,
    Later,
    Feed(i64),
    Folder(String),
}

#[derive(Clone, Copy, Debug)]
pub enum MarkField {
    Read,
    Starred,
    Later,
}

/// Keyset cursor for article list pages (`ORDER BY published_at DESC, id DESC`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArticleCursor {
    pub published_at: Option<String>,
    pub id: i64,
}

impl ArticleCursor {
    pub fn from_summary(summary: &ArticleSummary) -> Self {
        Self {
            published_at: summary.published_at.clone(),
            id: summary.id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReaderSnapshot {
    pub feeds: Vec<Feed>,
    pub articles: Vec<ArticleSummary>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct PreparedArticle {
    pub article: Article,
    pub body_html: String,
}

pub struct ParsedArticle {
    pub guid: String,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub snippet: String,
    pub content_html: String,
}
