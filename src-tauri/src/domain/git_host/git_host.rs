use super::{IssueInfo, PrStatus};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct GitHostError(pub String);

pub trait GitHostProvider: Send + Sync {
    fn fetch_pr_status(&self, repo_path: &str) -> Result<PrStatus, GitHostError>;
    fn list_issues(&self, repo_path: &str) -> Vec<IssueInfo>;
}

pub trait PrStatusCache: Send + Sync {
    fn lookup(&self, repo_path: &str) -> Option<PrStatus>;
    fn store(&self, repo_path: &str, value: PrStatus);
}

pub trait IssueCache: Send + Sync {
    fn lookup(&self, repo_path: &str) -> Option<Vec<IssueInfo>>;
    fn store(&self, repo_path: &str, value: Vec<IssueInfo>);
}
