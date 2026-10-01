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

#[derive(Debug, Clone, PartialEq)]
pub struct CachedResult<T> {
    pub value: Option<T>,
    pub error: Option<GitHostError>,
}
impl<T> Default for CachedResult<T> {
    fn default() -> Self {
        Self {
            value: None,
            error: None,
        }
    }
}
impl<T> CachedResult<T> {
    pub fn record(&mut self, result: Result<T, GitHostError>) {
        match result {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}
pub trait PrStatusCache: Send + Sync {
    fn result(&self, repo_path: &str) -> CachedResult<PrStatus>;
    fn record(&self, repo_path: &str, result: Result<PrStatus, GitHostError>);
}
pub trait IssueCache: Send + Sync {
    fn result(&self, repo_path: &str) -> CachedResult<Vec<IssueInfo>>;
    fn record(&self, repo_path: &str, result: Result<Vec<IssueInfo>, GitHostError>);
}

#[cfg(test)]
#[path = "git_host_test.rs"]
mod git_host_tests;
