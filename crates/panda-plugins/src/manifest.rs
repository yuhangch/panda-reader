use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginStage {
    Prepare,
    Cleanup,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub kind: PluginKind,
    pub domains: Vec<String>,
    #[serde(default)]
    pub path_prefixes: Vec<String>,
    pub stage: PluginStage,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub capabilities: Vec<PluginCapability>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    Rules,
    Wasm,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    DocumentRead,
    DocumentWrite,
    ArticleBodyWrite,
}
