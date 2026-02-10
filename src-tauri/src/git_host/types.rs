use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Available,
    CliNotFound { cli: String },
    NotAuthenticated,
    UnsupportedPlatform,
    NoRemote,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PrStatus {
    pub open_prs: HashMap<String, PrInfo>,
    pub merged_branches: Vec<String>,
}

pub trait GitHostProvider: Send + Sync {
    fn detect_open_prs(&self, repo_path: &str) -> HashMap<String, PrInfo>;
    fn detect_merged_prs(&self, repo_path: &str) -> Vec<String>;
}
