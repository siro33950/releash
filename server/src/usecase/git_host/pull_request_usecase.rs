use std::sync::Arc;

use crate::domain::failure::WorkFailure;
use crate::domain::git::{GitError, GitService};
use crate::domain::git_host::{
    authorize_pull_request_creation, ActionsJobId, BranchPullRequest, GitHostError,
    GitHostProvider, NewPullRequest, PrCommentError, PrCommentText, PullRequestBases,
    PullRequestCache, PullRequestCreationError,
};
use crate::usecase::fetched::Fetched;
use crate::usecase::git_host::GitHostUsecase;
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionUsecase};

/// PR を作る操作と、Comment を投稿する操作の失敗。
#[derive(Debug, thiserror::Error)]
pub enum PullRequestUsecaseError {
    /// PR を作れない業務上の理由。
    #[error(transparent)]
    Creation(#[from] PullRequestCreationError),
    /// Comment を投稿できない業務上の理由。
    #[error(transparent)]
    Comment(#[from] PrCommentError),
    /// GitHub が操作を拒んだ、または GitHub に届かなかった。
    #[error(transparent)]
    Host(#[from] GitHostError),
    /// Worktree の Repository を読めなかった。
    #[error(transparent)]
    Repository(#[from] GitError),
}

/// PR の面の手順。選んでいる Worktree のブランチの PR を読む、作る、Comment を投稿する、check のログを読む。
#[derive(Clone)]
pub struct PullRequestUsecase {
    state_publisher: Option<StateSubscriptionUsecase>,
    git: Arc<dyn GitService>,
    git_host: Arc<GitHostUsecase>,
    provider: Arc<dyn GitHostProvider>,
    pull_requests: Arc<dyn PullRequestCache>,
}

impl PullRequestUsecase {
    pub fn new(
        git: Arc<dyn GitService>,
        git_host: Arc<GitHostUsecase>,
        provider: Arc<dyn GitHostProvider>,
        pull_requests: Arc<dyn PullRequestCache>,
    ) -> Self {
        Self {
            state_publisher: None,
            git,
            git_host,
            provider,
            pull_requests,
        }
    }

    pub fn with_state_publisher(mut self, publisher: StateSubscriptionUsecase) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    /// Worktree のブランチの PR を、持っている値から読む。GitHub には取りに行かない。
    pub fn read(&self, worktree_path: &str) -> Result<Fetched<BranchPullRequest>, GitError> {
        let repo_path = self.git.main_repo_path(worktree_path)?;
        let branch = self.git.current(worktree_path)?;
        let resolved = BranchPullRequest::resolve(
            self.git_host.pr_status(&repo_path),
            &branch,
            self.pull_requests.result(worktree_path),
        );
        Ok(Fetched {
            value: resolved.value,
            error: resolved.error.as_ref().map(WorkFailure::from_error),
        })
    }

    /// Worktree のブランチの PR を GitHub から取り直して持つ。
    /// Repository の PR の一覧を取り直し、ブランチの PR があれば、その内容を取る。
    /// GitHub から取れなかった失敗は、持っている値に記録し、戻り値では返さない。
    pub async fn refresh(&self, worktree_path: &str) -> Result<(), GitError> {
        let repo_path = self.git.main_repo_path(worktree_path)?;
        let branch = self.git.current(worktree_path)?;
        let _ = self.git_host.refresh_pr_status(&repo_path).await;
        let status = self.git_host.pr_status(&repo_path);
        if let Some(pull_request) = status
            .value
            .as_ref()
            .and_then(|status| status.for_branch(&branch))
        {
            let result = self
                .provider
                .fetch_pull_request(&repo_path, pull_request.number)
                .await;
            self.pull_requests.record(worktree_path, result);
        }
        Ok(())
    }

    /// PR の面の購読が始まる Worktree の、PR の内容を持ち始める。
    pub fn acquire(&self, worktree_path: &str) {
        self.pull_requests.hold(worktree_path);
    }

    /// PR の面の購読が無くなった Worktree の、持っている PR の内容を捨てる。
    pub fn release(&self, worktree_path: &str) {
        self.pull_requests.forget(worktree_path);
    }

    /// PR の base に選べるブランチと、初めに選ぶ既定のブランチを返す。
    pub fn bases(&self, worktree_path: &str) -> Result<PullRequestBases, GitError> {
        let branches = self.git.list_branches(worktree_path)?;
        let default_branch = self.git.default_branch(worktree_path)?;
        Ok(PullRequestBases::new(&branches, default_branch))
    }

    /// Worktree のブランチを head とする PR を作る。
    /// 作れたら、ブランチの PR を取り直し、PR の面の購読へ知らせる。
    /// 取り直しの失敗は、作成の結果にしない。
    pub async fn create(
        &self,
        worktree_path: &str,
        title: String,
        body: String,
        base: String,
        draft: bool,
    ) -> Result<(), PullRequestUsecaseError> {
        let new = NewPullRequest::new(title, body, base, draft)?;
        let repo_path = self.git.main_repo_path(worktree_path)?;
        let head = self.git.current(worktree_path)?;
        let origin_url = self.git.origin_url(&repo_path)?;
        let branches = self.git.list_branches(&repo_path)?;
        authorize_pull_request_creation(origin_url.as_deref(), &head, &branches)?;
        self.provider
            .create_pull_request(&repo_path, &head, &new)
            .await?;
        let _ = self.refresh(worktree_path).await;
        self.notify(worktree_path);
        Ok(())
    }

    /// PR 全体への Comment を投稿する。
    /// 投稿できたら、ブランチの PR を取り直し、PR の面の購読へ知らせる。
    /// 取り直しの失敗は、投稿の結果にしない。
    pub async fn post_comment(
        &self,
        worktree_path: &str,
        number: u64,
        body: String,
    ) -> Result<(), PullRequestUsecaseError> {
        let text = PrCommentText::new(body)?;
        let repo_path = self.git.main_repo_path(worktree_path)?;
        self.provider
            .post_pull_request_comment(&repo_path, number, &text)
            .await?;
        let _ = self.refresh(worktree_path).await;
        self.notify(worktree_path);
        Ok(())
    }

    /// GitHub Actions の job のログの全体を GitHub から取る。
    /// GitHub から取れなかった失敗は、戻り値の `Fetched` に記録する。
    pub async fn check_log(
        &self,
        worktree_path: &str,
        job: ActionsJobId,
    ) -> Result<Fetched<String>, GitError> {
        let repo_path = self.git.main_repo_path(worktree_path)?;
        let mut log = Fetched::default();
        log.record(
            self.provider
                .fetch_check_log(&repo_path, job)
                .await
                .map_err(|error| WorkFailure::from_error(&error)),
        );
        Ok(log)
    }

    fn notify(&self, worktree_path: &str) {
        if let Some(publisher) = &self.state_publisher {
            publisher.notify(StateChangeSource::PullRequest(worktree_path.into()));
        }
    }
}

#[cfg(test)]
#[path = "pull_request_usecase_test.rs"]
mod pull_request_usecase_tests;
