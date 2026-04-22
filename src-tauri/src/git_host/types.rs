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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrAuthor {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueLabel {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    pub author: PrAuthor,
    #[serde(alias = "createdAt")]
    pub created_at: String,
    #[serde(alias = "updatedAt")]
    pub updated_at: String,
    #[serde(default)]
    pub labels: Vec<IssueLabel>,
    #[serde(default)]
    pub assignees: Vec<PrAuthor>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub milestone: Option<Milestone>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrReviewComment {
    pub id: u64,
    pub path: String,
    pub line: Option<u32>,
    pub original_line: Option<u32>,
    pub body: String,
    #[serde(alias = "user")]
    pub author: PrReviewCommentAuthor,
    pub in_reply_to_id: Option<u64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrReviewCommentAuthor {
    pub login: String,
    pub avatar_url: Option<String>,
}

pub trait GitHostProvider: Send + Sync {
    fn detect_open_prs(&self, repo_path: &str) -> HashMap<String, PrInfo>;
    fn detect_merged_prs(&self, repo_path: &str) -> Vec<String>;
    fn list_issues(&self, repo_path: &str) -> Vec<IssueInfo>;
    #[allow(dead_code)]
    fn get_pr_review_comments(&self, repo_path: &str, pr_number: u64) -> Vec<PrReviewComment>;
}
