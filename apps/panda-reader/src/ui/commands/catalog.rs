//! Command catalog for the palette and shared `run_command` dispatch.

use crate::ui::i18n::{self, Language};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandGroup {
    Navigate,
    Article,
    Feeds,
    View,
    Application,
}

impl CommandGroup {
    pub fn title(self, language: Language) -> &'static str {
        i18n::text(
            language,
            match self {
                Self::Navigate => "Navigate",
                Self::Article => "Article",
                Self::Feeds => "Feeds",
                Self::View => "View",
                Self::Application => "Application",
            },
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    TogglePalette,
    NextArticle,
    PreviousArticle,
    NextUnread,
    PreviousUnread,
    GoAll,
    GoUnread,
    GoStarred,
    GoLater,
    ToggleRead,
    ToggleStar,
    ToggleLater,
    ExtractFullText,
    Translate,
    ToggleHideImages,
    ToggleTranslationLayout,
    OpenOriginal,
    CopyLink,
    CopyTitle,
    Refresh,
    ForceRefreshArticle,
    AddFeed,
    MarkAllRead,
    LoadMoreArticles,
    ToggleSidebar,
    OpenSettings,
    OpenAppearance,
    OpenReading,
    OpenAbout,
    FocusSearch,
    ShowKeyboardShortcuts,
}

#[derive(Clone, Copy, Debug)]
pub struct CommandItem {
    pub kind: CommandKind,
    pub group: CommandGroup,
    pub title: &'static str,
    pub aliases: &'static [&'static str],
    pub chord: Option<&'static str>,
}

impl CommandItem {
    pub fn localized_title(self, language: Language) -> &'static str {
        i18n::text(language, self.title)
    }
}

pub fn catalog() -> &'static [CommandItem] {
    &[
        CommandItem {
            kind: CommandKind::NextArticle,
            group: CommandGroup::Navigate,
            title: "Next article",
            aliases: &["next", "j", "down"],
            chord: Some("Ctrl+] / j"),
        },
        CommandItem {
            kind: CommandKind::PreviousArticle,
            group: CommandGroup::Navigate,
            title: "Previous article",
            aliases: &["prev", "previous", "k", "up"],
            chord: Some("Ctrl+[ / k"),
        },
        CommandItem {
            kind: CommandKind::NextUnread,
            group: CommandGroup::Navigate,
            title: "Next unread",
            aliases: &["unread next", "J"],
            chord: Some("J"),
        },
        CommandItem {
            kind: CommandKind::PreviousUnread,
            group: CommandGroup::Navigate,
            title: "Previous unread",
            aliases: &["unread prev", "K"],
            chord: Some("K"),
        },
        CommandItem {
            kind: CommandKind::GoAll,
            group: CommandGroup::Navigate,
            title: "Go to All Articles",
            aliases: &["all", "scope all"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::GoUnread,
            group: CommandGroup::Navigate,
            title: "Go to Unread",
            aliases: &["unread", "scope unread"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::GoStarred,
            group: CommandGroup::Navigate,
            title: "Go to Starred",
            aliases: &["starred", "stars", "favorites"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::GoLater,
            group: CommandGroup::Navigate,
            title: "Go to Read Later",
            aliases: &["later", "read later", "bookmark"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::ToggleRead,
            group: CommandGroup::Article,
            title: "Toggle read",
            aliases: &["mark read", "unread", "u"],
            chord: Some("u"),
        },
        CommandItem {
            kind: CommandKind::ToggleStar,
            group: CommandGroup::Article,
            title: "Toggle star",
            aliases: &["star", "favorite", "s"],
            chord: Some("s"),
        },
        CommandItem {
            kind: CommandKind::ToggleLater,
            group: CommandGroup::Article,
            title: "Toggle Read Later",
            aliases: &["later", "l"],
            chord: Some("l"),
        },
        CommandItem {
            kind: CommandKind::ExtractFullText,
            group: CommandGroup::Article,
            title: "Extract full text",
            aliases: &["extract", "fulltext", "readability"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::ForceRefreshArticle,
            group: CommandGroup::Article,
            title: "Re-fetch current article",
            aliases: &["force refresh", "refresh article", "re-fetch"],
            chord: Some("Shift+Ctrl/Cmd+R"),
        },
        CommandItem {
            kind: CommandKind::Translate,
            group: CommandGroup::Article,
            title: "Translate",
            aliases: &["translation", "azure"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::ToggleHideImages,
            group: CommandGroup::Article,
            title: "Toggle hide images",
            aliases: &["images", "hide pictures"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::ToggleTranslationLayout,
            group: CommandGroup::Article,
            title: "Toggle immersive translation",
            aliases: &["bilingual", "immersive", "side by side", "display mode"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::OpenOriginal,
            group: CommandGroup::Article,
            title: "Open original",
            aliases: &["browser", "open url", "o"],
            chord: Some("o"),
        },
        CommandItem {
            kind: CommandKind::CopyLink,
            group: CommandGroup::Article,
            title: "Copy link",
            aliases: &["copy url"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::CopyTitle,
            group: CommandGroup::Article,
            title: "Copy title",
            aliases: &["copy name"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::Refresh,
            group: CommandGroup::Feeds,
            title: "Refresh feeds",
            aliases: &["sync", "reload", "r"],
            chord: Some("Ctrl+R / r"),
        },
        CommandItem {
            kind: CommandKind::AddFeed,
            group: CommandGroup::Feeds,
            title: "Add feed",
            aliases: &["subscribe", "new feed"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::MarkAllRead,
            group: CommandGroup::Feeds,
            title: "Mark all as read",
            aliases: &["mark all", "read all", "x"],
            chord: Some("x"),
        },
        CommandItem {
            kind: CommandKind::LoadMoreArticles,
            group: CommandGroup::Feeds,
            title: "Load more articles",
            aliases: &["pagination", "more", "next page"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::ToggleSidebar,
            group: CommandGroup::View,
            title: "Toggle sidebar",
            aliases: &["sidebar", "collapse"],
            chord: Some("Ctrl+B"),
        },
        CommandItem {
            kind: CommandKind::OpenSettings,
            group: CommandGroup::View,
            title: "Open Settings",
            aliases: &["preferences", "general"],
            chord: Some("Ctrl+,"),
        },
        CommandItem {
            kind: CommandKind::OpenAppearance,
            group: CommandGroup::View,
            title: "Open Appearance",
            aliases: &["theme", "appearance"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::OpenReading,
            group: CommandGroup::View,
            title: "Open Reading settings",
            aliases: &["reading", "vim"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::OpenAbout,
            group: CommandGroup::View,
            title: "Open About",
            aliases: &["about", "version"],
            chord: None,
        },
        CommandItem {
            kind: CommandKind::FocusSearch,
            group: CommandGroup::View,
            title: "Focus search",
            aliases: &["find", "filter articles"],
            chord: Some("Ctrl+F"),
        },
        CommandItem {
            kind: CommandKind::TogglePalette,
            group: CommandGroup::Application,
            title: "Command Palette",
            aliases: &["palette", "commands", "search everywhere"],
            chord: Some("Ctrl+K"),
        },
        CommandItem {
            kind: CommandKind::ShowKeyboardShortcuts,
            group: CommandGroup::Application,
            title: "Keyboard shortcuts",
            aliases: &["hotkeys", "keymap", "vim", "help"],
            chord: Some("Ctrl+/"),
        },
    ]
}

pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let needle: Vec<char> = query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .collect();
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    if needle.len() > hay.len() {
        return None;
    }
    let best = (0..hay.len())
        .filter(|&i| hay[i] == needle[0])
        .filter_map(|start| align(&needle, &hay, start))
        .max()?;
    let mut score = best;
    if hay == needle {
        score += 120;
    } else if hay.starts_with(&needle) {
        score += 50;
    }
    score -= (hay.len() as i32) / 6;
    Some(score)
}

fn align(needle: &[char], hay: &[char], start: usize) -> Option<i32> {
    let mut qi = 0usize;
    let mut score = 0i32;
    let mut run = 0i32;
    let mut prev_hit = false;
    for (i, ch) in hay.iter().enumerate().skip(start) {
        if qi >= needle.len() {
            break;
        }
        if *ch != needle[qi] {
            prev_hit = false;
            run = 0;
            continue;
        }
        score += 1;
        let word_start = i == 0 || !hay[i - 1].is_alphanumeric();
        if word_start {
            score += 12;
        }
        if i == 0 {
            score += 10;
        }
        if prev_hit {
            run += 1;
            score += 6 + run.min(8);
        } else {
            run = 0;
        }
        prev_hit = true;
        qi += 1;
    }
    (qi == needle.len()).then_some(score)
}

pub fn item_score(query: &str, item: &CommandItem, language: Language) -> Option<i32> {
    let title = fuzzy_score(query, item.localized_title(language));
    let english = fuzzy_score(query, item.title);
    let alias = item
        .aliases
        .iter()
        .filter_map(|a| fuzzy_score(query, a))
        .max()
        .map(|s| s - 10);
    [title, english, alias].into_iter().flatten().max()
}

pub fn filtered_commands(query: &str, language: Language) -> Vec<&'static CommandItem> {
    let mut scored: Vec<(i32, usize, &'static CommandItem)> = catalog()
        .iter()
        .filter_map(|item| {
            item_score(query, item, language).map(|score| (score, item.kind as usize, item))
        })
        .collect();
    if query.trim().is_empty() {
        scored.sort_by(|a, b| {
            (a.2.group as u8)
                .cmp(&(b.2.group as u8))
                .then(a.1.cmp(&b.1))
        });
    } else {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    scored.into_iter().map(|(_, _, item)| item).collect()
}
