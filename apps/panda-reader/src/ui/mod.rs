//! Desktop presentation. Feature modules own their state; the window coordinates operations.

mod article_list;
mod article_view;
mod commands;
mod components;
mod context_menus;
mod date;
mod feed_editor;
pub(crate) mod i18n;
mod quote_poster;
mod settings;
mod sidebar;
mod status;
mod subscriptions;
pub(crate) mod theme;
mod window;

pub use commands::bind_keys;
pub use window::ReaderWindow;
