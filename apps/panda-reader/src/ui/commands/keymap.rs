//! Global actions and key bindings for Panda Reader.

use gpui_kit::{App, KeyBinding, actions};

actions!(
    panda_reader,
    [
        TogglePalette,
        RefreshFeeds,
        OpenSettings,
        FocusSearch,
        ToggleSidebar,
        NextArticle,
        PreviousArticle,
        NextUnread,
        PreviousUnread,
        ToggleRead,
        ToggleStar,
        ToggleLater,
        OpenOriginal,
        MarkAllRead,
        ShowKeyboardShortcuts,
        PaletteMoveUp,
        PaletteMoveDown,
        PaletteConfirm,
        PaletteDismiss,
        LoadMoreArticles,
        // Vim-only (gated in handlers)
        VimNextArticle,
        VimPreviousArticle,
        VimNextUnread,
        VimPreviousUnread,
        VimToggleRead,
        VimToggleStar,
        VimToggleLater,
        VimOpenOriginal,
        VimMarkAllRead,
        VimRefresh,
        VimShowHelp,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-k", TogglePalette, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-r", RefreshFeeds, None),
        KeyBinding::new("secondary-f", FocusSearch, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-]", NextArticle, None),
        KeyBinding::new("secondary-[", PreviousArticle, None),
        KeyBinding::new("secondary-/", ShowKeyboardShortcuts, None),
        // Vim-style (handlers no-op when typing or vim_navigation is off)
        KeyBinding::new("j", VimNextArticle, Some("Reader")),
        KeyBinding::new("k", VimPreviousArticle, Some("Reader")),
        KeyBinding::new("shift-j", VimNextUnread, Some("Reader")),
        KeyBinding::new("shift-k", VimPreviousUnread, Some("Reader")),
        KeyBinding::new("u", VimToggleRead, Some("Reader")),
        KeyBinding::new("s", VimToggleStar, Some("Reader")),
        KeyBinding::new("l", VimToggleLater, Some("Reader")),
        KeyBinding::new("r", VimRefresh, Some("Reader")),
        KeyBinding::new("o", VimOpenOriginal, Some("Reader")),
        KeyBinding::new("x", VimMarkAllRead, Some("Reader")),
        KeyBinding::new("shift-/", VimShowHelp, Some("Reader")),
        // Palette chrome
        KeyBinding::new("escape", PaletteDismiss, Some("Palette")),
        KeyBinding::new("up", PaletteMoveUp, Some("Palette")),
        KeyBinding::new("down", PaletteMoveDown, Some("Palette")),
        KeyBinding::new("ctrl-p", PaletteMoveUp, Some("Palette")),
        KeyBinding::new("ctrl-n", PaletteMoveDown, Some("Palette")),
        KeyBinding::new("enter", PaletteConfirm, Some("Palette")),
    ]);
}
