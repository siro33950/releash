use crate::domain::state_subscription::{StateChangeSource, SubscriptionTarget};
use crate::usecase::state_subscription::{
    StateReadError, StateSubscriptionRead, WorkspaceStateReads,
};

pub(crate) struct StateSubscriptionReads(pub WorkspaceStateReads);

#[async_trait::async_trait]
impl StateSubscriptionRead for StateSubscriptionReads {
    async fn read(
        &self,
        target: &SubscriptionTarget,
    ) -> Result<crate::usecase::state_subscription::StateValue, StateReadError> {
        use SubscriptionTarget as T;
        if matches!(
            target,
            T::Failures(..)
                | T::AgentSession(_)
                | T::SessionHistory(..)
                | T::Selection(..)
                | T::NodeDetail(..)
                | T::SessionNode(..)
        ) {
            return self.0.read(target).await;
        }
        let reads = self.0.clone();
        let target = target.clone();
        let runtime = tokio::runtime::Handle::current();
        crate::common::operation_context::spawn_blocking(move || {
            runtime.block_on(reads.read(&target))
        })
        .await
        .map_err(task_error)?
    }
    async fn refresh_external(&self, target: &SubscriptionTarget) -> Result<(), StateReadError> {
        if !matches!(target, SubscriptionTarget::Issues(_)) {
            return self.0.refresh_external(target).await;
        }
        let reads = self.0.clone();
        let target = target.clone();
        let runtime = tokio::runtime::Handle::current();
        crate::common::operation_context::spawn_blocking(move || {
            runtime.block_on(reads.refresh_external(&target))
        })
        .await
        .map_err(task_error)?
    }
    async fn refresh_workspaces(&self, source: Option<StateChangeSource>) {
        self.0.refresh_workspaces(source).await;
    }
    fn repositories(&self) -> Vec<String> {
        self.0.repositories()
    }
}
fn task_error(error: tokio::task::JoinError) -> StateReadError {
    StateReadError {
        kind: crate::domain::failure::FailureKind::Internal,
        message: error.to_string(),
    }
}

#[cfg(test)]
#[path = "state_subscription_reads_test.rs"]
mod state_subscription_reads_tests;
