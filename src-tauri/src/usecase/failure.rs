use crate::common::retry::AttemptProgress;
use crate::domain::failure::TechnicalFailureNature;

pub use crate::domain::failure::{
    BusinessFailure, Failure, FailureKey, FailureRecord, WorkFailure,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureObservation {
    pub record: FailureRecord,
    pub requires_attention: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailurePage {
    pub items: Vec<FailureObservation>,
    pub next_offset: Option<usize>,
    pub requires_attention: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailurePageDto {
    pub items: Vec<FailureObservationDto>,
    pub next_offset: Option<usize>,
    pub requires_attention: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureObservationDto {
    pub record: FailureRecordDto,
    pub requires_attention: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureRecordDto {
    pub operation: String,
    pub target: String,
    pub classification: &'static str,
    pub message: String,
    pub count: u64,
    pub first_observed_ms: u64,
    pub last_observed_ms: u64,
}

impl From<FailurePage> for FailurePageDto {
    fn from(page: FailurePage) -> Self {
        Self {
            items: page
                .items
                .into_iter()
                .map(|observation| {
                    let record = observation.record;
                    FailureObservationDto {
                        record: FailureRecordDto {
                            operation: record.operation,
                            target: record.target,
                            classification: failure_classification(record.kind),
                            message: record.message,
                            count: record.count,
                            first_observed_ms: record.first_observed_ms,
                            last_observed_ms: record.last_observed_ms,
                        },
                        requires_attention: observation.requires_attention,
                    }
                })
                .collect(),
            next_offset: page.next_offset,
            requires_attention: page.requires_attention,
        }
    }
}

pub(crate) fn failure_classification(failure: Failure) -> &'static str {
    match failure {
        Failure::Business(BusinessFailure::VersionConflict) => "VersionConflict",
        Failure::Business(BusinessFailure::Other) => "BusinessFailure",
        Failure::Technical(TechnicalFailureNature::Transient) => "Transient",
        Failure::Technical(TechnicalFailureNature::TimedOut) => "TimedOut",
        Failure::Technical(TechnicalFailureNature::Cancelled) => "Cancelled",
        Failure::Technical(TechnicalFailureNature::Other) => "TechnicalFailure",
    }
}

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
        subscriptions.notify(
            crate::usecase::state_subscription::StateChangeSource::Failures(key.target.clone()),
        );
    }
}

#[async_trait::async_trait]
pub trait FailureQueryService: Send + Sync {
    async fn page(&self, targets: &[String], offset: usize) -> FailurePage;

    #[cfg(test)]
    fn records(&self, target: &str) -> Vec<FailureObservation>;
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
