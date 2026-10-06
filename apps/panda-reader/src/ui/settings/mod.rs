//! Settings form state, page rendering, and persisted preference actions.

mod about;
mod actions;
mod appearance;
mod general;
mod plugins;
mod reading;
mod selectors;
mod state;
mod translation;
mod view;

pub(in crate::ui) use selectors::SettingsPage;
pub(in crate::ui) use state::Settings;
