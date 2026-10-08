use crate::common::retry::AttemptProgress;
use crate::domain::failure::TechnicalFailureNature;

pub use crate::domain::failure::{BusinessFailure, Failure, FailureKey, WorkFailure};

pub const ATTEMPT_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

pub fn attempt_expired() -> WorkFailure {
    WorkFailure {
        kind: Failure::Technical(TechnicalFailureNature::TimedOut),
        message: "試行の期限（20秒）を超えました".into(),
    }
}

pub trait RetryFailure {
    fn work_failure(&self) -> WorkFailure;
}

impl RetryFailure for WorkFailure {
    fn work_failure(&self) -> WorkFailure {
        self.clone()
    }
}

macro_rules! retry_failure_from_debug {
    ($($error:ty),* $(,)?) => {
        $(
            impl RetryFailure for $error {
                fn work_failure(&self) -> WorkFailure {
                    WorkFailure::from_error(self)
                }
            }
        )*
    };
}

retry_failure_from_debug!(
    crate::domain::workflow::WorkflowError,
    crate::usecase::workflow::runtime_error::WorkflowRuntimeError,
    crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError,
    crate::domain::agent_session::repository::AgentSessionRepositoryError,
    crate::usecase::repository_state::error::RepositoryStateError,
);

pub struct FailureRecordingUsecase {
    repository: std::sync::Arc<dyn crate::domain::failure::FailureRecordRepository>,
    subscriptions: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
}

impl FailureRecordingUsecase {
    pub fn new(
        repository: std::sync::Arc<dyn crate::domain::failure::FailureRecordRepository>,
        subscriptions: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    ) -> Self {
        Self {
            repository,
            subscriptions,
        }
    }

    pub fn observed(&self, key: &FailureKey, failure: WorkFailure) {
        let attention = requires_attention(failure.kind);
        let changed = self.repository.record_observed(key, failure, attention);
        self.notify(key, changed);
    }

    pub fn resolved(&self, key: &FailureKey) {
        let changed = self.repository.record_resolved(key);
        self.notify(key, changed);
    }

    fn notify(&self, key: &FailureKey, attention_changed: bool) {
        let Some(subscriptions) = &self.subscriptions else {
            return;
        };
        if attention_changed && key.operation.starts_with("workflow_") {
            subscriptions
                .notify(crate::usecase::state_subscription::StateChangeSource::WorkspaceList);
        }
    }
}

pub(crate) fn requires_attention(kind: Failure) -> bool {
    matches!(
        kind,
        Failure::Business(BusinessFailure::Other)
            | Failure::Technical(TechnicalFailureNature::TimedOut | TechnicalFailureNature::Other)
    )
}

pub(crate) fn next_attempt(failure: Failure) -> Option<AttemptProgress> {
    match failure {
        Failure::Business(BusinessFailure::VersionConflict) => Some(AttemptProgress::Reload),
        Failure::Technical(TechnicalFailureNature::Transient) => Some(AttemptProgress::Continue),
        _ => None,
    }
}

#[cfg(test)]
#[path = "failure_test.rs"]
mod failure_tests;
