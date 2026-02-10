use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitFileStatusMsg {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatusSync {
    pub files: Vec<GitFileStatusMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatusRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStage {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitUnstage {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStageResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub files: Vec<GitFileStatusMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStageHunk {
    pub patch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommitRequest {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommitResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitPushRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitPushResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfoRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfoResponse {
    pub branch: String,
}
