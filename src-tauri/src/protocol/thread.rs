use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadEntry {
    pub id: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "author_name"
    )]
    pub author_name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "author_avatar_url"
    )]
    pub author_avatar_url: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "pr_comment_id"
    )]
    pub pr_comment_id: Option<u64>,
    #[serde(default, alias = "created_at")]
    pub created_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineAnchor {
    #[serde(alias = "target_line")]
    pub target_line: String,
    #[serde(alias = "context_before")]
    pub context_before: Vec<String>,
    #[serde(alias = "context_after")]
    pub context_after: Vec<String>,
    #[serde(alias = "original_line_number")]
    pub original_line_number: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    #[serde(alias = "file_path")]
    pub file_path: String,
    #[serde(alias = "line_number")]
    pub line_number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "end_line")]
    pub end_line: Option<u32>,
    pub entries: Vec<ThreadEntry>,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<LineAnchor>,
    #[serde(default, alias = "created_at")]
    pub created_at: f64,
}

// --- WebSocket message payloads ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadsSync {
    pub threads: Vec<Thread>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateThread {
    pub file_path: String,
    pub line_number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddThreadEntry {
    pub thread_id: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveThread {
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteThread {
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateThreadEntry {
    pub thread_id: String,
    pub entry_id: String,
    pub content: String,
}
