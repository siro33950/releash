use super::{ActionsJobId, IssueInfo, NewPullRequest, PrCommentText, PrStatus, PullRequest};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitHostError {
    #[error("{0}")]
    External(String),
    #[error("{0}")]
    Technical(crate::domain::failure::TechnicalFailure),
}

#[async_trait::async_trait]

pub trait GitHostProvider: Send + Sync {
    async fn fetch_pr_status(&self, repo_path: &str) -> Result<PrStatus, GitHostError>;
    async fn list_issues(&self, repo_path: &str) -> Result<Vec<IssueInfo>, GitHostError>;
    /// その番号の PR の内容を GitHub から取る。
    /// 内容は、番号、URL、状態、draft かどうか、タイトル、本文、check の全部、
    /// PR 全体への Comment の全部、レビューの本文の全部である。コードの行へのレビューの Comment は含めない。
    async fn fetch_pull_request(
        &self,
        repo_path: &str,
        number: u64,
    ) -> Result<PullRequest, GitHostError>;
    /// `head` のブランチを head とする PR を GitHub に作る。ブランチを push することはない。
    async fn create_pull_request(
        &self,
        repo_path: &str,
        head: &str,
        new: &NewPullRequest,
    ) -> Result<(), GitHostError>;
    /// その番号の PR に、PR 全体への Comment を投稿する。
    async fn post_pull_request_comment(
        &self,
        repo_path: &str,
        number: u64,
        text: &PrCommentText,
    ) -> Result<(), GitHostError>;
    /// GitHub Actions の job のログの全体を取る。
    async fn fetch_check_log(
        &self,
        repo_path: &str,
        job: ActionsJobId,
    ) -> Result<String, GitHostError>;
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
/// PR の面が出ている Worktree ごとに、最後に取れた PR の内容と、直近の取得の失敗を持つ。
/// 持つのは `hold` から `forget` までの間である。期限では捨てない。
pub trait PullRequestCache: Send + Sync {
    /// その Worktree の分を持ち始める。
    fn hold(&self, worktree_path: &str);
    fn result(&self, worktree_path: &str) -> CachedResult<PullRequest>;
    /// 取得の結果を記録する。失敗のときは、最後に取れた内容を残す。
    /// `hold` していない Worktree の結果は捨てる。
    fn record(&self, worktree_path: &str, result: Result<PullRequest, GitHostError>);
    /// その Worktree の分を捨てる。
    fn forget(&self, worktree_path: &str);
}
pub trait IssueCache: Send + Sync {
    fn result(&self, repo_path: &str) -> CachedResult<Vec<IssueInfo>>;
    fn record(&self, repo_path: &str, result: Result<Vec<IssueInfo>, GitHostError>);
}

#[cfg(test)]
#[path = "git_host_test.rs"]
mod git_host_tests;
