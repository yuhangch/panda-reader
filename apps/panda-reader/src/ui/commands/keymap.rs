//! Global actions and key bindings for Panda Reader.

use gpui_kit::{App, KeyBinding, Unbind, actions};

actions!(
    panda_reader,
    [
        TogglePalette,
        RefreshFeeds,
        ForceRefreshArticle,
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
        KeyBinding::new("secondary-shift-r", ForceRefreshArticle, None),
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
        // Printable Vim keys must reach text inputs. A no-op in the action
        // handler is too late because the key binding has already consumed it.
        KeyBinding::new("j", Unbind("panda_reader::VimNextArticle".into()), Some("Input")),
        KeyBinding::new("k", Unbind("panda_reader::VimPreviousArticle".into()), Some("Input")),
        KeyBinding::new("shift-j", Unbind("panda_reader::VimNextUnread".into()), Some("Input")),
        KeyBinding::new("shift-k", Unbind("panda_reader::VimPreviousUnread".into()), Some("Input")),
        KeyBinding::new("u", Unbind("panda_reader::VimToggleRead".into()), Some("Input")),
        KeyBinding::new("s", Unbind("panda_reader::VimToggleStar".into()), Some("Input")),
        KeyBinding::new("l", Unbind("panda_reader::VimToggleLater".into()), Some("Input")),
        KeyBinding::new("r", Unbind("panda_reader::VimRefresh".into()), Some("Input")),
        KeyBinding::new("o", Unbind("panda_reader::VimOpenOriginal".into()), Some("Input")),
        KeyBinding::new("x", Unbind("panda_reader::VimMarkAllRead".into()), Some("Input")),
        KeyBinding::new("shift-/", Unbind("panda_reader::VimShowHelp".into()), Some("Input")),
        // Palette chrome
        KeyBinding::new("escape", PaletteDismiss, Some("Palette")),
        KeyBinding::new("up", PaletteMoveUp, Some("Palette")),
        KeyBinding::new("down", PaletteMoveDown, Some("Palette")),
        KeyBinding::new("ctrl-p", PaletteMoveUp, Some("Palette")),
        KeyBinding::new("ctrl-n", PaletteMoveDown, Some("Palette")),
        KeyBinding::new("enter", PaletteConfirm, Some("Palette")),
    ]);
}
