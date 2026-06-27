use super::{IssueInfo, PrStatus, ProviderStatus};

pub trait GitHostProvider: Send + Sync {
    fn provider_status(&self, repo_path: &str) -> ProviderStatus;
    fn fetch_pr_status(&self, repo_path: &str) -> PrStatus;
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
