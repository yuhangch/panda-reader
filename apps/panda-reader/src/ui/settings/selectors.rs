use crate::app::preferences::{Language, LibrarySource};
use crate::ui::i18n;
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::*;
use panda_translate::Provider;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct LanguageOption {
    language: Language,
    interface_language: Language,
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
    const PROVIDERS: [Provider; 4] = [
        Provider::Azure,
        Provider::Volcengine,
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
    About,
}
