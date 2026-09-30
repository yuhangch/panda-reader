use panda_core::{
    ArticleCursor, MarkField, PreparedArticle, ReaderSnapshot, Scope, TranslationLayout,
};
use panda_providers::ProviderKind;
use std::path::PathBuf;
use tokio::sync::oneshot;

pub enum Command {
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
        reply: oneshot::Sender<Result<PreparedArticle, String>>,
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
        reply: oneshot::Sender<Result<String, String>>,
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
        reply: oneshot::Sender<Result<(), String>>,
    },
    RefreshFeed {
        id: i64,
        reply: oneshot::Sender<Result<usize, String>>,
    },
    Refresh {
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
        reply: oneshot::Sender<Result<PreparedArticle, String>>,
    },
    ImportOpml {
        content: String,
        reply: oneshot::Sender<Result<usize, String>>,
    },
    ExportOpml {
        reply: oneshot::Sender<Result<String, String>>,
    },
}
