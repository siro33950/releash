use crate::domain::failure::{
    StorageFailure, StorageFailureSource, TechnicalFailure, TechnicalFailureNature,
};
use crate::domain::local_event::{CommitBatchError, LocalEventQueryError};

impl From<CommitBatchError> for StorageFailure {
    fn from(error: CommitBatchError) -> Self {
        let nature = match &error {
            CommitBatchError::Technical(failure) => failure.nature,
            CommitBatchError::QueueBusy
            | CommitBatchError::OutcomeUnknown { .. }
            | CommitBatchError::AppendOutcomeUnknown => TechnicalFailureNature::Transient,
            CommitBatchError::StorageUnavailable { failure }
            | CommitBatchError::StorageAccessRequired { failure } => failure.nature,
            _ => TechnicalFailureNature::Other,
        };
        Self {
            nature,
            context: None,
            source: StorageFailureSource::Commit(error),
        }
    }
}

impl From<LocalEventQueryError> for StorageFailure {
    fn from(error: LocalEventQueryError) -> Self {
        let nature = match &error {
            LocalEventQueryError::Technical(failure) => failure.nature,
            LocalEventQueryError::QueryBusy => TechnicalFailureNature::Transient,
            LocalEventQueryError::StorageUnavailable { failure }
            | LocalEventQueryError::StorageAccessRequired { failure } => failure.nature,
            _ => TechnicalFailureNature::Other,
        };
        Self {
            nature,
            context: None,
            source: StorageFailureSource::Query(error),
        }
    }
}

impl From<TechnicalFailure> for StorageFailure {
    fn from(error: TechnicalFailure) -> Self {
        Self {
            nature: error.nature,
            context: None,
            source: StorageFailureSource::Technical(error),
        }
    }
}

impl From<crate::domain::workflow::WorkflowError> for StorageFailure {
    fn from(error: crate::domain::workflow::WorkflowError) -> Self {
        let error = match error {
            crate::domain::workflow::WorkflowError::Store(failure) => return failure,
            error => error,
        };
        let nature = match &error {
            crate::domain::workflow::WorkflowError::Technical(failure) => failure.nature,
            _ => TechnicalFailureNature::Other,
        };
        Self {
            nature,
            context: None,
            source: StorageFailureSource::Workflow(Box::new(error)),
        }
    }
}

impl From<crate::adaptor::gateway::workflow::fact_log::FactReadError> for StorageFailure {
    fn from(error: crate::adaptor::gateway::workflow::fact_log::FactReadError) -> Self {
        LocalEventQueryError::from(error).into()
    }
}

impl From<crate::domain::repository::RepositoryError> for StorageFailure {
    fn from(error: crate::domain::repository::RepositoryError) -> Self {
        let nature = match &error {
            crate::domain::repository::RepositoryError::Technical(failure) => failure.nature,
            _ => TechnicalFailureNature::Other,
        };
        Self {
            nature,
            context: None,
            source: StorageFailureSource::Repository(error),
        }
    }
}

impl From<crate::domain::agent_session::repository::AgentSessionRepositoryError>
    for StorageFailure
{
    fn from(error: crate::domain::agent_session::repository::AgentSessionRepositoryError) -> Self {
        use crate::domain::agent_session::repository::AgentSessionRepositoryError;
        if let AgentSessionRepositoryError::Store(failure) = error {
            return failure;
        }
        let nature = if matches!(error, AgentSessionRepositoryError::Unavailable) {
            TechnicalFailureNature::Transient
        } else {
            TechnicalFailureNature::Other
        };
        Self {
            nature,
            context: None,
            source: StorageFailureSource::AgentSession(Box::new(error)),
        }
    }
}

impl From<crate::usecase::agent_session::AgentSessionQueryError> for StorageFailure {
    fn from(error: crate::usecase::agent_session::AgentSessionQueryError) -> Self {
        use crate::domain::agent_session::repository::AgentSessionRepositoryError as R;
        use crate::usecase::agent_session::AgentSessionQueryError as E;
        match error {
            E::Store(failure) => failure,
            E::Unavailable => R::Unavailable.into(),
            E::Corrupt => R::Corrupt.into(),
            E::InvalidRequest => R::InvalidRequest.into(),
        }
    }
}

impl From<crate::usecase::workflow::runtime_error::WorkflowRuntimeError> for StorageFailure {
    fn from(error: crate::usecase::workflow::runtime_error::WorkflowRuntimeError) -> Self {
        use crate::domain::workflow::WorkflowError as W;
        use crate::usecase::workflow::runtime_error::WorkflowRuntimeError as E;
        let source = match error {
            E::Store(failure) => return failure,
            E::Technical(failure) => return failure.into(),
            E::Conflict(message) => W::Conflict(message),
            E::InvalidState(message) | E::AgentSession(message) => W::InvalidState(message),
            error @ E::AlreadyActive(_) => W::InvalidState(error.to_string()),
            E::ExecutionNotFound(message) | E::SessionNotFound(message) => W::NotFound(message),
            E::InvalidWorkflow(message) | E::ValidationError(message) => W::Validation(message),
            E::UnauthorizedWorktree(message) | E::UnauthorizedApprovalTarget(message) => {
                W::UnauthorizedApprovalTarget(message)
            }
            E::SessionStore(message) => W::External(message),
        };
        source.into()
    }
}

impl From<crate::usecase::work_queue::WorkFailure> for StorageFailure {
    fn from(error: crate::usecase::work_queue::WorkFailure) -> Self {
        use crate::domain::failure::{BusinessFailure, Failure};
        use crate::domain::workflow::WorkflowError as W;
        match error.kind {
            Failure::Business(BusinessFailure::VersionConflict) => {
                W::Conflict(error.message).into()
            }
            Failure::Business(BusinessFailure::Other) => W::InvalidState(error.message).into(),
            Failure::Technical(nature) => TechnicalFailure {
                nature,
                message: error.message,
            }
            .into(),
        }
    }
}

impl From<LocalEventQueryError>
    for crate::domain::agent_session::repository::AgentSessionRepositoryError
{
    fn from(error: LocalEventQueryError) -> Self {
        match error {
            LocalEventQueryError::QueryBusy => Self::Unavailable,
            LocalEventQueryError::Technical(ref failure)
                if failure.nature == TechnicalFailureNature::Transient =>
            {
                Self::Unavailable
            }
            LocalEventQueryError::StorageUnavailable { ref failure }
                if failure.nature == TechnicalFailureNature::Transient =>
            {
                Self::Unavailable
            }
            LocalEventQueryError::InvalidRequest => Self::InvalidRequest,
            LocalEventQueryError::Corrupt { .. } => Self::Corrupt,
            error => Self::Store(error.into()),
        }
    }
}

impl From<LocalEventQueryError>
    for crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError
{
    fn from(error: LocalEventQueryError) -> Self {
        match error {
            LocalEventQueryError::QueryBusy => Self::StorageUnavailable,
            LocalEventQueryError::Technical(ref failure)
                if failure.nature == TechnicalFailureNature::Transient =>
            {
                Self::StorageUnavailable
            }
            LocalEventQueryError::StorageUnavailable { ref failure }
                if failure.nature == TechnicalFailureNature::Transient =>
            {
                Self::StorageUnavailable
            }
            LocalEventQueryError::InvalidRequest => Self::InvalidInput,
            LocalEventQueryError::Corrupt { .. } => Self::Corrupt,
            error => Self::Store(error.into()),
        }
    }
}

impl From<LocalEventQueryError>
    for crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError
{
    fn from(error: LocalEventQueryError) -> Self {
        match error {
            LocalEventQueryError::QueryBusy => Self::StorageUnavailable,
            LocalEventQueryError::Technical(ref failure)
                if failure.nature == TechnicalFailureNature::Transient =>
            {
                Self::StorageUnavailable
            }
            LocalEventQueryError::StorageUnavailable { ref failure }
                if failure.nature == TechnicalFailureNature::Transient =>
            {
                Self::StorageUnavailable
            }
            LocalEventQueryError::InvalidRequest => Self::InvalidInput,
            LocalEventQueryError::Corrupt { .. } => Self::Corrupt,
            error => Self::Store(error.into()),
        }
    }
}

impl From<LocalEventQueryError> for crate::usecase::agent_session::AgentSessionQueryError {
    fn from(error: LocalEventQueryError) -> Self {
        match error {
            LocalEventQueryError::QueryBusy => Self::Unavailable,
            LocalEventQueryError::Technical(ref failure)
                if failure.nature == TechnicalFailureNature::Transient =>
            {
                Self::Unavailable
            }
            LocalEventQueryError::StorageUnavailable { ref failure }
                if failure.nature == TechnicalFailureNature::Transient =>
            {
                Self::Unavailable
            }
            LocalEventQueryError::InvalidRequest => Self::InvalidRequest,
            LocalEventQueryError::Corrupt { .. } => Self::Corrupt,
            error => Self::Store(error.into()),
        }
    }
}

use crate::domain::workflow::WorkflowError;
impl From<crate::domain::local_event::CommitBatchError> for WorkflowError {
    fn from(error: crate::domain::local_event::CommitBatchError) -> Self {
        let message = format!("node fact append failed: {error}");
        Self::storage(error, message)
    }
}

impl WorkflowError {
    pub(crate) fn storage(
        error: impl Into<crate::domain::failure::StorageFailure>,
        message: impl Into<String>,
    ) -> Self {
        Self::Store(error.into().with_message(message))
    }
}
