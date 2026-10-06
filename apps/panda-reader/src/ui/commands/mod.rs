//! Commands shared by the palette, shortcuts, and context menus.

mod catalog;
mod dispatch;
pub(in crate::ui) mod keymap;
mod palette;

pub(in crate::ui) use catalog::CommandKind;
pub use keymap::bind_keys;
pub(in crate::ui) use palette::PaletteState;
