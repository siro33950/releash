use super::{IssueInfo, PrStatus};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitHostError {
    #[error("{0}")]
    External(String),
    #[error("{0}")]
    Technical(crate::domain::failure::TechnicalFailure),
}

pub trait GitHostProvider: Send + Sync {
    fn fetch_pr_status(&self, repo_path: &str) -> Result<PrStatus, GitHostError>;
    fn list_issues(&self, repo_path: &str) -> Result<Vec<IssueInfo>, GitHostError>;
}

pub trait PrStatusCache: Send + Sync {
    fn lookup(&self, repo_path: &str) -> Option<PrStatus>;
    fn store(&self, repo_path: &str, value: PrStatus);
}

pub trait IssueCache: Send + Sync {
    fn lookup(&self, repo_path: &str) -> Option<Vec<IssueInfo>>;
    fn store(&self, repo_path: &str, value: Vec<IssueInfo>);
}
