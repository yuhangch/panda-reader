//! Application operations backed by the store and provider crates.
//! The worker transports requests; feature modules execute them.

mod articles;
mod command;
mod connections;
mod database;
mod diagnostics;
mod dispatch;
pub(crate) mod favicon;
mod feed_fetch;
mod subscriptions;
mod sync;
mod task_coordinator;
mod worker;

pub(crate) use articles::fetch::ArticleFetchNetwork;
pub use command::Command;
#[allow(unused_imports)]
pub(crate) use command::TitleTranslationInput;
#[allow(unused_imports)]
pub(crate) use command::TitleTranslationStatus;
#[allow(unused_imports)]
pub(crate) use diagnostics::{redact_diagnostic_message, write_sync_summary_log};
pub use worker::AppServices;
