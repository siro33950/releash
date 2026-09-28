use crate::common::retry::AttemptProgress;
use crate::domain::failure::TechnicalFailureNature;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusinessFailure {
    VersionConflict,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Business(BusinessFailure),
    Technical(TechnicalFailureNature),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FailureKey {
    pub operation: String,
    pub target: String,
}

impl FailureKey {
    pub fn new(operation: &str, target: &str) -> Self {
        Self {
            operation: operation.into(),
            target: target.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkFailure {
    pub kind: Failure,
    pub message: String,
}

impl WorkFailure {
    pub fn from_error<E: std::fmt::Debug>(error: &E) -> Self
    where
        for<'a> Failure: From<&'a E>,
    {
        Self {
            kind: Failure::from(error),
            message: format!("{error:?}"),
        }
    }
}

impl From<crate::domain::failure::TechnicalFailure> for WorkFailure {
    fn from(failure: crate::domain::failure::TechnicalFailure) -> Self {
        Self {
            kind: Failure::Technical(failure.nature),
            message: failure.message,
        }
    }
}

pub const ATTEMPT_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

pub fn attempt_expired() -> WorkFailure {
    WorkFailure {
        kind: Failure::Technical(TechnicalFailureNature::TimedOut),
        message: "試行の期限（20秒）を超えました".into(),
    }
}

impl std::fmt::Display for WorkFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
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
pub struct FailureRecord {
    pub operation: String,
    pub target: String,
    pub kind: Failure,
    pub message: String,
    pub active: bool,
    pub count: u64,
    pub first_observed_ms: u64,
    pub last_observed_ms: u64,
}

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

pub trait FailureOutput: Send + Sync {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any;
    fn observed(&self, key: &FailureKey, failure: WorkFailure);
    fn resolved(&self, key: &FailureKey);
}

#[async_trait::async_trait]
pub trait FailureQueryService: Send + Sync {
    async fn page(&self, targets: &[String], offset: usize) -> FailurePage;
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
