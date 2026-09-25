use super::{IssueInfo, PrStatus};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitHostError {
    #[error("{0}")]
    External(String),
    #[error("{0}")]
    Stopped(crate::domain::operation_context::OperationStopped),
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

impl crate::domain::failure::ClassifiedFailure for GitHostError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        match self {
            Self::External(_) => crate::domain::failure::FailureKind::Internal,
            Self::Stopped(error) => crate::domain::failure::ClassifiedFailure::failure_kind(error),
        }
    }
}

#[cfg(test)]
#[path = "git_host_test.rs"]
mod git_host_tests;
