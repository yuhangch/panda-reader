use panda_core::{
    ArticleCursor, MarkField, PreparedArticle, ReaderSnapshot, Scope, TranslationLayout,
};
use panda_plugins::{CommunityPlugin, PluginSummary};
use panda_providers::ProviderKind;
use std::path::PathBuf;
use tokio::sync::oneshot;

pub struct ConnectOutcome {
    pub account_name: String,
    pub initial_sync_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TitleTranslationInput {
    pub id: i64,
    pub title: String,
}
#[derive(Clone, Debug, Default)]
pub struct TitleTranslationOutcome {
    pub translated: Vec<(i64, String, String, String)>,
    pub usage: Vec<panda_core::TranslationUsage>,
    pub status: Option<TitleTranslationStatus>,
}

#[derive(Clone, Debug)]
pub enum TitleTranslationStatus {
    Info(String),
    Error(String),
}

pub enum Command {
    CommunityPluginCatalog {
        reply: oneshot::Sender<Result<Vec<CommunityPlugin>, String>>,
    },
    InstallCommunityPlugin {
        id: String,
        reply: oneshot::Sender<Result<Vec<PluginSummary>, String>>,
    },
    PluginList {
        reply: oneshot::Sender<Result<Vec<PluginSummary>, String>>,
    },
    ReloadPlugins {
        reply: oneshot::Sender<Result<Vec<PluginSummary>, String>>,
    },
    SetPluginEnabled {
        id: String,
        enabled: bool,
        reply: oneshot::Sender<Result<Vec<PluginSummary>, String>>,
    },
    ImportPlugin {
        source: String,
        reply: oneshot::Sender<Result<Vec<PluginSummary>, String>>,
    },
    RemovePlugin {
        id: String,
        reply: oneshot::Sender<Result<Vec<PluginSummary>, String>>,
    },
    Snapshot {
        scope: Scope,
        search: String,
        limit: i64,
        after: Option<ArticleCursor>,
        include_feeds: bool,
        reply: oneshot::Sender<Result<ReaderSnapshot, String>>,
    },
    Article {
        id: i64,
        show_translation: bool,
        translation_layout: TranslationLayout,
        hide_images: bool,
        paragraph_indent: bool,
        reply: oneshot::Sender<Result<PreparedArticle, String>>,
    },
    SaveReadingProgress {
        id: i64,
        progress: f32,
        reply: oneshot::Sender<Result<(), String>>,
    },
    EnsureFavicon {
        host: String,
        site_url: String,
        icons_dir: PathBuf,
        reply: oneshot::Sender<Result<PathBuf, String>>,
    },
    Connect {
        kind: ProviderKind,
        endpoint: String,
        username: String,
        secret: String,
        reply: oneshot::Sender<Result<ConnectOutcome, String>>,
    },
    Disconnect {
        kind: ProviderKind,
        reply: oneshot::Sender<Result<(), String>>,
    },
    AddFeed {
        url: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    RemoveFeed {
        id: i64,
        reply: oneshot::Sender<Result<(), String>>,
    },
    UpdateFeed {
        id: i64,
        title: String,
        folder: Option<String>,
        feed_url: String,
        auto_translate_titles: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SetFeedAutoTranslateTitles {
        id: i64,
        enabled: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    RefreshFeed {
        id: i64,
        reply: oneshot::Sender<Result<usize, String>>,
    },
    Refresh {
        force: bool,
        reply: oneshot::Sender<Result<usize, String>>,
    },
    Mark {
        id: i64,
        field: MarkField,
        value: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    MarkAllRead {
        scope: Scope,
        reply: oneshot::Sender<Result<usize, String>>,
    },
    Extract {
        id: i64,
        /// When false, skip network fetch if `extracted_html` already exists.
        force: bool,
        extractor: panda_core::ContentExtractor,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Translate {
        id: i64,
        target_lang: String,
        translation_layout: TranslationLayout,
        hide_images: bool,
        paragraph_indent: bool,
        reply: oneshot::Sender<Result<PreparedArticle, String>>,
    },
    TranslateTitles {
        items: Vec<TitleTranslationInput>,
        target_lang: String,
        reply: oneshot::Sender<Result<TitleTranslationOutcome, String>>,
    },
    TranslationUsage {
        reply: oneshot::Sender<Result<Vec<panda_core::TranslationUsage>, String>>,
    },
    ImportOpml {
        content: String,
        reply: oneshot::Sender<Result<usize, String>>,
    },
    ExportOpml {
        reply: oneshot::Sender<Result<String, String>>,
    },
}
