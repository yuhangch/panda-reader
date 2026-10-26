use crate::app::preferences::{ArticleNetworkMode, Language, LibrarySource};
use crate::ui::i18n;
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::*;
use panda_translate::Provider;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct LanguageOption {
    language: Language,
    interface_language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct LogRetentionOption {
    days: u16,
    language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct ArticleNetworkModeOption {
    pub(in crate::ui) mode: ArticleNetworkMode,
    language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct BackupModeOption {
    pub(in crate::ui) mode: panda_backup::BackupMode,
    language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct BackupTargetOption {
    pub(in crate::ui) s3: bool,
    language: Language,
}

impl BackupTargetOption {
    pub(in crate::ui) fn all(language: Language) -> Vec<Self> {
        [false, true]
            .into_iter()
            .map(|s3| Self { s3, language })
            .collect()
    }
}

impl SearchableListItem for BackupTargetOption {
    type Value = Self;

    fn title(&self) -> SharedString {
        SharedString::from(i18n::text(
            self.language,
            if self.s3 { "S3 compatible" } else { "WebDAV" },
        ))
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

impl BackupModeOption {
    pub(in crate::ui) fn all(language: Language) -> Vec<Self> {
        [
            panda_backup::BackupMode::Config,
            panda_backup::BackupMode::Full,
        ]
        .into_iter()
        .map(|mode| Self { mode, language })
        .collect()
    }
}

impl SearchableListItem for BackupModeOption {
    type Value = Self;

    fn title(&self) -> SharedString {
        let label = match self.mode {
            panda_backup::BackupMode::Config => "Configuration only (Recommended)",
            panda_backup::BackupMode::Full => "Full backup",
        };
        SharedString::from(i18n::text(self.language, label))
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

impl ArticleNetworkModeOption {
    pub(in crate::ui) fn all(language: Language) -> Vec<Self> {
        ArticleNetworkMode::ALL
            .into_iter()
            .map(|mode| Self { mode, language })
            .collect()
    }

    pub(in crate::ui) fn index(mode: ArticleNetworkMode) -> usize {
        mode.index()
    }
}

impl SearchableListItem for ArticleNetworkModeOption {
    type Value = Self;

    fn title(&self) -> SharedString {
        SharedString::from(i18n::text(self.language, self.mode.label()))
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

impl LogRetentionOption {
    pub(in crate::ui) fn all(language: Language) -> Vec<Self> {
        [7, 30, 90]
            .into_iter()
            .map(|days| Self { days, language })
            .collect()
    }

    pub(in crate::ui) fn index(days: u16) -> usize {
        [7, 30, 90]
            .iter()
            .position(|option| *option == days)
            .unwrap_or(1)
    }
}

impl SearchableListItem for LogRetentionOption {
    type Value = u16;

    fn title(&self) -> SharedString {
        SharedString::from(i18n::format(self.language, "{} days", self.days))
    }

    fn value(&self) -> &Self::Value {
        &self.days
    }
}

impl LanguageOption {
    pub(in crate::ui) fn all(interface_language: Language) -> Vec<Self> {
        Language::ALL
            .into_iter()
            .map(|language| Self {
                language,
                interface_language,
            })
            .collect()
    }
}

impl SearchableListItem for LanguageOption {
    type Value = Language;
    fn title(&self) -> SharedString {
        let label = if self.interface_language == Language::English {
            self.language.english_name()
        } else {
            self.language.native_name()
        };
        SharedString::from(label)
    }
    fn value(&self) -> &Self::Value {
        &self.language
    }
}

impl SearchableListItem for LibrarySource {
    type Value = Self;
    fn title(&self) -> SharedString {
        SharedString::from(self.label())
    }
    fn value(&self) -> &Self::Value {
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct TranslatorProvider {
    pub(in crate::ui) provider: Provider,
    language: Language,
}

impl TranslatorProvider {
    const PROVIDERS: [Provider; 7] = [
        Provider::Azure,
        Provider::Volcengine,
        Provider::OpenAiCompatible,
        Provider::Anthropic,
        Provider::Gemini,
        Provider::DeepL,
        Provider::LibreTranslate,
    ];

    pub(in crate::ui) fn all(language: Language) -> Vec<Self> {
        Self::PROVIDERS
            .into_iter()
            .map(|provider| Self { provider, language })
            .collect()
    }

    pub(in crate::ui) fn label(self) -> &'static str {
        i18n::text(self.language, self.provider.display_name())
    }
}

impl SearchableListItem for TranslatorProvider {
    type Value = Self;

    fn title(&self) -> SharedString {
        if self.provider.is_ready() {
            SharedString::from(self.label())
        } else {
            SharedString::from(format!(
                "{} ({})",
                self.label(),
                i18n::text(self.language, "Coming soon")
            ))
        }
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum SettingsPage {
    General,
    Appearance,
    Reading,
    Translation,
    Plugins,
    Backup,
    Developer,
    About,
}
