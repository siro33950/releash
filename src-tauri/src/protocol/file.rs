use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContentRequest {
    pub path: String,
    #[serde(default = "default_diff_base")]
    pub diff_base: String,
}

fn default_diff_base() -> String {
    "HEAD".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContentResponse {
    pub path: String,
    pub original: String,
    pub modified: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staged: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: String,
}
