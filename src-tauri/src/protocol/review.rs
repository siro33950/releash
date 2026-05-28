use serde::{Deserialize, Serialize};

pub use crate::review_comments::{
    ReviewErrorCode, ReviewHistoryEntry, ReviewStanceValue, ReviewTarget, ReviewThread,
    ReviewThreadFilter,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewErrorPayload {
    pub code: ReviewErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewListRequest {
    pub worktree_name: Option<String>,
    pub filter: Option<ReviewThreadFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewListResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    pub threads: Vec<ReviewThread>,
    pub error: Option<ReviewErrorPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewGetRequest {
    pub worktree_name: Option<String>,
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThreadResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    pub thread: Option<ReviewThread>,
    pub error: Option<ReviewErrorPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCreateRequest {
    pub worktree_name: Option<String>,
    pub target: ReviewTarget,
    pub content: String,
}

/// Comment 追記と (任意で) 同 actor の Stance 表明を atomic に行う request。
///
/// spec issues-1022 design.md L45 Stance contract / Boundaries L77: Stance の書き込みは
/// Comment 追記操作の任意フラグとしてのみ提供する。`stance` 指定:
/// - `Some(Agree)` / `Some(Disagree)`: 現在 Stance を上書き
/// - `Some(None)`: 現在 Stance を未表明状態に撤回
/// - `None` (未指定): 現在 Stance を維持
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAppendCommentRequest {
    pub worktree_name: Option<String>,
    pub thread_id: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stance: Option<ReviewStanceValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResolveRequest {
    pub worktree_name: Option<String>,
    pub thread_id: String,
    pub outcome: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewHistoryRequest {
    pub worktree_name: Option<String>,
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewHistoryResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    pub events: Vec<ReviewHistoryEntry>,
    pub error: Option<ReviewErrorPayload>,
}
