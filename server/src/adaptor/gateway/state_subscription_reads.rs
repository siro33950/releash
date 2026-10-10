use crate::usecase::state_subscription::SubscriptionTarget;
use crate::usecase::state_subscription::{
    StateReadError, StateSubscriptionRead, WorkspaceStateReads,
};

pub struct StateSubscriptionReads(pub WorkspaceStateReads);

#[async_trait::async_trait]
impl StateSubscriptionRead for StateSubscriptionReads {
    async fn read(
        &self,
        target: &SubscriptionTarget,
    ) -> Result<crate::usecase::state_subscription::StateValue, StateReadError> {
        use SubscriptionTarget as T;
        if matches!(
            target,
            T::DaemonInfo
                | T::WorkflowExecution(_)
                | T::WorkflowOutput(..)
                | T::ReviewSessionThreads(..)
                | T::ReviewWorktreeThreads(..)
                | T::ReviewSessionThread(..)
                | T::ReviewSessionThreadHistory(..)
                | T::Issues(_)
                | T::FilteredIssues(..)
                | T::Workspaces
                | T::Workflows
                | T::AgentSession(_)
                | T::SessionHistory(..)
                | T::Selection(..)
                | T::NodeDetail(..)
                | T::ProviderHookHealth
        ) {
            return self.0.read(target).await;
        }
        let reads = self.0.clone();
        let target = target.clone();
        crate::common::operation_context::spawn_blocking(move || reads.read_blocking(&target))
            .await
            .map_err(task_error)?
    }
    async fn refresh_external(&self, target: &SubscriptionTarget) -> Result<(), StateReadError> {
        self.0.refresh_external_blocking(target).await
    }
    fn acquire_external(&self, target: &SubscriptionTarget) {
        self.0.acquire_external(target);
    }
    fn release_external(&self, target: &SubscriptionTarget) {
        self.0.release_external(target);
    }
    fn repositories(&self) -> Vec<String> {
        self.0.repositories()
    }
    fn watch_paths(
        &self,
    ) -> (
        Vec<String>,
        Vec<(String, crate::domain::failure::WorkFailure)>,
    ) {
        self.0.watch_paths()
    }
    fn review_comments_dir(&self) -> String {
        self.0.review_comments_dir()
    }
    fn workflows_dir(&self) -> String {
        self.0.workflows_dir()
    }
}
fn task_error(error: tokio::task::JoinError) -> StateReadError {
    StateReadError::from_error(crate::domain::failure::TechnicalFailure::from(error))
}
