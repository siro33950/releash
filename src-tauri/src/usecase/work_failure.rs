use crate::domain::failure::{BusinessFailure, Failure, TechnicalFailureNature};

impl From<&crate::usecase::code_error::CodeUsecaseError> for Failure {
    fn from(error: &crate::usecase::code_error::CodeUsecaseError) -> Self {
        use crate::usecase::code_error::CodeUsecaseError as E;
        match error {
            E::Code(error) => Failure::from(error),
        }
    }
}

impl From<&crate::usecase::repository_error::UsecaseError> for Failure {
    fn from(error: &crate::usecase::repository_error::UsecaseError) -> Self {
        use crate::usecase::repository_error::UsecaseError as E;
        match error {
            E::Workflow(error) => Failure::from(error),
            E::Repository(error) => Failure::from(error),
            E::Rule(_) => Failure::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::repository::error::RepositoryError> for Failure {
    fn from(error: &crate::domain::repository::error::RepositoryError) -> Self {
        use crate::domain::repository::error::RepositoryError as E;
        match error {
            E::Technical(error) => Failure::from(error),
            E::External(_) => Failure::Technical(TechnicalFailureNature::Other),
            E::Rule(_) => Failure::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::agent_session::ProviderSessionTitleGatewayError> for Failure {
    fn from(error: &crate::domain::agent_session::ProviderSessionTitleGatewayError) -> Self {
        use crate::domain::agent_session::ProviderSessionTitleGatewayError as E;
        match error {
            E::Unavailable => Failure::Technical(TechnicalFailureNature::Transient),
            E::Corrupt => Failure::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::domain::agent_session::repository::AgentSessionRepositoryError> for Failure {
    fn from(error: &crate::domain::agent_session::repository::AgentSessionRepositoryError) -> Self {
        use crate::domain::agent_session::repository::AgentSessionRepositoryError as E;
        match error {
            E::Store(kind) => Failure::from(kind),
            E::Conflict => Failure::Business(BusinessFailure::VersionConflict),
            E::ProviderSessionAlreadyOwned { .. } => Failure::Business(BusinessFailure::Other),
            E::InvalidRequest => Failure::Business(BusinessFailure::Other),
            E::Corrupt => Failure::Technical(TechnicalFailureNature::Other),
            E::Unavailable => Failure::Technical(TechnicalFailureNature::Transient),
        }
    }
}

impl From<&crate::domain::app_config::error::AppConfigError> for Failure {
    fn from(error: &crate::domain::app_config::error::AppConfigError) -> Self {
        use crate::domain::app_config::error::AppConfigError as E;
        match error {
            E::Repository(_) => Failure::Technical(TechnicalFailureNature::Other),
            E::InvalidInput(_) => Failure::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::code::error::CodeError> for Failure {
    fn from(error: &crate::domain::code::error::CodeError) -> Self {
        use crate::domain::code::error::CodeError as E;
        match error {
            E::Technical(error) => Failure::from(error),
            E::External(_) => Failure::Technical(TechnicalFailureNature::Other),
            E::Rule(_) => Failure::Business(BusinessFailure::Other),
            E::StaleReviewBlobVersion { .. } | E::StaleReviewGroupTarget { .. } => {
                Failure::Business(BusinessFailure::VersionConflict)
            }
        }
    }
}

impl From<&crate::domain::external_editor::gateway::EditorError> for Failure {
    fn from(error: &crate::domain::external_editor::gateway::EditorError) -> Self {
        use crate::domain::external_editor::gateway::EditorError as E;
        match error {
            E::InvalidInput(_) => Failure::Business(BusinessFailure::Other),
            E::Settings(error) => Failure::from(error),
            E::Launch(_) => Failure::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::local_event::batch::CommitBatchError> for Failure {
    fn from(error: &crate::domain::local_event::batch::CommitBatchError) -> Self {
        use crate::domain::local_event::batch::CommitBatchError as E;
        if error.is_version_conflict() {
            return Failure::Business(BusinessFailure::VersionConflict);
        }
        match error {
            E::Technical(error) => Failure::from(error),
            E::PayloadConflict => Failure::Business(BusinessFailure::Other),
            E::QueueBusy => Failure::Technical(TechnicalFailureNature::Transient),
            E::StreamHeadConflict { .. } | E::TreeHeadConflict => unreachable!(),
            E::OutcomeUnknown { .. } | E::AppendOutcomeUnknown => {
                Failure::Technical(TechnicalFailureNature::Transient)
            }
            E::CapacityExceeded | E::SequenceExhausted => Failure::Business(BusinessFailure::Other),
            E::StorageUnavailable { failure } => Failure::from(failure),
            E::StorageAccessRequired { .. } => Failure::Business(BusinessFailure::Other),
            E::Corrupt { .. } => Failure::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::domain::local_event::query::LocalEventQueryError> for Failure {
    fn from(error: &crate::domain::local_event::query::LocalEventQueryError) -> Self {
        use crate::domain::local_event::query::LocalEventQueryError as E;
        match error {
            E::Technical(error) => Failure::from(error),
            E::InvalidRequest => Failure::Business(BusinessFailure::Other),
            E::CanonicalWriterRequired => Failure::Business(BusinessFailure::Other),
            E::QueryBusy => Failure::Technical(TechnicalFailureNature::Transient),
            E::ResponseTooLarge => Failure::Business(BusinessFailure::Other),
            E::IncompatibleStoredEvent { .. } => Failure::Business(BusinessFailure::Other),
            E::StorageUnavailable { failure } => Failure::from(failure),
            E::StorageAccessRequired { .. } => Failure::Business(BusinessFailure::Other),
            E::Corrupt { .. } => Failure::Technical(TechnicalFailureNature::Other),
            E::Internal { .. } => Failure::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::domain::workflow::error::WorkflowError> for Failure {
    fn from(error: &crate::domain::workflow::error::WorkflowError) -> Self {
        use crate::domain::workflow::error::WorkflowError as E;
        match error {
            E::Technical(error) => Failure::from(error),
            E::Store(kind) => Failure::from(kind),
            E::External(_) => Failure::Technical(TechnicalFailureNature::Other),
            E::Editor(error) => Failure::from(error),
            E::CorruptStoredState(_) => Failure::Technical(TechnicalFailureNature::Other),
            E::IncompatibleStoredEvent(_) | E::InvalidState(_) => {
                Failure::Business(BusinessFailure::Other)
            }
            E::Validation(_) => Failure::Business(BusinessFailure::Other),
            E::Conflict(_) => Failure::Business(BusinessFailure::VersionConflict),
            E::NotFound(_) => Failure::Business(BusinessFailure::Other),
            E::UnauthorizedApprovalTarget(_) => Failure::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError> for Failure {
    fn from(error: &crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError) -> Self {
        use crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError as E;
        match error {
            E::Conflict => Failure::Business(BusinessFailure::VersionConflict),
            E::Store(kind) => Failure::from(kind),
            E::InvalidInput => Failure::Business(BusinessFailure::Other),
            E::StorageUnavailable => Failure::Technical(TechnicalFailureNature::Transient),
            E::Corrupt => Failure::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError> for Failure {
    fn from(error: &crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError) -> Self {
        use crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError as E;
        match error {
            E::Store(kind) => Failure::from(kind),
            E::InvalidInput => Failure::Business(BusinessFailure::Other),
            E::Conflict => Failure::Business(BusinessFailure::VersionConflict),
            E::StorageUnavailable => Failure::Technical(TechnicalFailureNature::Transient),
            E::Corrupt => Failure::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::usecase::workflow::runtime_error::WorkflowRuntimeError> for Failure {
    fn from(error: &crate::usecase::workflow::runtime_error::WorkflowRuntimeError) -> Self {
        use crate::usecase::workflow::runtime_error::WorkflowRuntimeError as E;
        match error {
            E::Store(failure) => Failure::from(failure),
            E::Technical(stopped) => Failure::from(stopped),
            E::AlreadyActive(_) | E::InvalidState(_) => Failure::Business(BusinessFailure::Other),
            E::Conflict(_) => Failure::Business(BusinessFailure::VersionConflict),
            E::ExecutionNotFound(_) | E::SessionNotFound(_) => {
                Failure::Business(BusinessFailure::Other)
            }
            E::InvalidWorkflow(_) | E::ValidationError(_) => {
                Failure::Business(BusinessFailure::Other)
            }
            E::UnauthorizedWorktree(_) | E::UnauthorizedApprovalTarget(_) => {
                Failure::Business(BusinessFailure::Other)
            }
            E::SessionStore(_) => Failure::Technical(TechnicalFailureNature::Other),
            E::AgentSession(_) => Failure::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::usecase::repository_state::error::RepositoryStateError> for Failure {
    fn from(error: &crate::usecase::repository_state::error::RepositoryStateError) -> Self {
        use crate::usecase::repository_state::error::RepositoryStateError as E;
        match error {
            E::Background { kind, .. } => *kind,
            E::ScanInvalidated => Failure::Business(BusinessFailure::VersionConflict),
            E::Repository(error) => Failure::from(error),
            E::Code(error) => Failure::from(error),
            E::Watcher(_) => Failure::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::domain::failure::TechnicalFailure> for Failure {
    fn from(error: &crate::domain::failure::TechnicalFailure) -> Self {
        Self::Technical(error.nature)
    }
}

impl From<&crate::domain::local_event::SafeOperationFailure> for Failure {
    fn from(error: &crate::domain::local_event::SafeOperationFailure) -> Self {
        Self::Technical(error.nature)
    }
}

impl From<&crate::domain::failure::StorageFailure> for Failure {
    fn from(error: &crate::domain::failure::StorageFailure) -> Self {
        Self::Technical(error.nature)
    }
}
impl From<&super::work_queue::WorkFailure> for Failure {
    fn from(error: &super::work_queue::WorkFailure) -> Self {
        error.kind
    }
}

impl From<&crate::domain::agent_session::AgentSessionDisplayNameError> for Failure {
    fn from(_: &crate::domain::agent_session::AgentSessionDisplayNameError) -> Self {
        Self::Business(BusinessFailure::Other)
    }
}

#[cfg(test)]
#[path = "work_failure_test.rs"]
mod work_failure_tests;
