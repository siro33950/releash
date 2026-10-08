use std::sync::Arc;

use crate::domain::git_host::{
    GitHostError, GitHostProvider, IssueCache, IssueInfo, PrStatus, PrStatusCache,
};

#[derive(Clone)]
pub struct GitHostUsecase {
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    provider: Arc<dyn GitHostProvider>,
    pr_cache: Arc<dyn PrStatusCache>,
    issue_cache: Arc<dyn IssueCache>,
}

impl GitHostUsecase {
    pub fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub fn new(
        provider: Arc<dyn GitHostProvider>,
        pr_cache: Arc<dyn PrStatusCache>,
        issue_cache: Arc<dyn IssueCache>,
    ) -> Self {
        Self {
            state_publisher: None,
            provider,
            pr_cache,
            issue_cache,
        }
    }

    pub(crate) fn pr_status_result(
        &self,
        path: &str,
    ) -> crate::usecase::fetched::Fetched<PrStatus> {
        let result = self.pr_cache.result(path);
        crate::usecase::fetched::Fetched {
            value: result.value,
            error: result
                .error
                .as_ref()
                .map(crate::domain::failure::WorkFailure::from_error),
        }
    }

    /// PR の状態を取りに行って保持する。変わったときは Workspaces の購読へ知らせる。
    pub async fn refresh_pr_status(&self, repo_path: &str) -> Result<(), GitHostError> {
        let result = self.provider.fetch_pr_status(repo_path).await;
        let previous = self.pr_cache.result(repo_path);
        let unchanged = match &result {
            Ok(value) => previous.error.is_none() && previous.value.as_ref() == Some(value),
            Err(error) => previous.error.as_ref() == Some(error),
        };
        if unchanged {
            return result.map(|_| ());
        }
        self.pr_cache.record(repo_path, result.clone());
        if let Some(publisher) = &self.state_publisher {
            publisher.notify(crate::usecase::state_subscription::StateChangeSource::WorkspaceList);
        }
        result.map(|_| ())
    }

    pub async fn fetch_issues(&self, repo_path: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        let result = self.provider.list_issues(repo_path).await;
        self.issue_cache.record(repo_path, result.clone());
        if let Some(publisher) = &self.state_publisher {
            publisher.notify(
                crate::usecase::state_subscription::StateChangeSource::Issues(repo_path.into()),
            );
        }
        result
    }

    pub(crate) async fn get_cached_issues(
        &self,
        repo_path: &str,
    ) -> crate::usecase::fetched::Fetched<Vec<IssueInfo>> {
        let mut result = self.issue_cache.result(repo_path);
        if result.value.is_none() && result.error.is_none() {
            let _ = self.fetch_issues(repo_path).await;
            result = self.issue_cache.result(repo_path);
        }
        crate::usecase::fetched::Fetched {
            value: result.value,
            error: result
                .error
                .as_ref()
                .map(crate::domain::failure::WorkFailure::from_error),
        }
    }
}

#[cfg(test)]
#[path = "git_host_usecase_test.rs"]
mod git_host_usecase_tests;
