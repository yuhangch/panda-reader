//! Local content plugins and the host-owned document interface.

mod catalog;
mod manifest;
mod registry;
mod rules;
mod wasm;

pub use catalog::{CommunityCatalog, CommunityPlugin, CommunityPluginFile};
pub use manifest::{PluginCapability, PluginKind, PluginManifest, PluginStage};
pub use registry::{
    PluginDiagnostic, PluginRegistry, PluginResult, PluginSettings, PluginSummary,
    UrlResolutionResult,
};
pub use rules::{ArticleDocument, RuleAction, RuleSet};
