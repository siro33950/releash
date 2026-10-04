use crate::domain::failure::TechnicalFailureNature;
use crate::usecase::failure::{BusinessFailure, Failure};

impl From<&crate::domain::git_host::GitHostError> for Failure {
    fn from(error: &crate::domain::git_host::GitHostError) -> Self {
        match error {
            crate::domain::git_host::GitHostError::Technical(error) => Failure::from(error),
            crate::domain::git_host::GitHostError::External(_) => {
                Failure::Technical(TechnicalFailureNature::Other)
            }
        }
    }
}

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
            E::Technical(error) => Failure::from(error),
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
            E::StaleReviewGroupTarget { .. } => Failure::Business(BusinessFailure::VersionConflict),
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
        if error.version_conflict().is_some() {
            Self::Business(BusinessFailure::VersionConflict)
        } else {
            Self::Technical(error.nature)
        }
    }
}
impl From<&super::failure::WorkFailure> for Failure {
    fn from(error: &super::failure::WorkFailure) -> Self {
        error.kind
    }
}

impl From<&crate::domain::agent_session::AgentSessionDisplayNameError> for Failure {
    fn from(_: &crate::domain::agent_session::AgentSessionDisplayNameError) -> Self {
        Self::Business(BusinessFailure::Other)
    }
}

impl From<&crate::usecase::watcher::UsecaseError> for Failure {
    fn from(error: &crate::usecase::watcher::UsecaseError) -> Self {
        use crate::usecase::watcher::UsecaseError as E;
        match error {
            E::Repository(error) => Self::from(error),
            E::RepositoryUnavailable => Self::Business(BusinessFailure::Other),
            E::File(_) => Self::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::usecase::state_subscription::StateReadError> for Failure {
    fn from(error: &crate::usecase::state_subscription::StateReadError) -> Self {
        use crate::usecase::state_subscription::StateReadFailure as E;
        match &error.source {
            E::TerminalSubscriptionEnded => Self::Business(BusinessFailure::Other),
            E::Terminal(error) => Self::from(error.as_ref()),
            E::Workflow(error) => Self::from(error.as_ref()),
            E::Session(error) => Self::from(error.as_ref()),
            E::History(error) => Self::from(error.as_ref()),
            E::Providers(error) => Self::from(error.as_ref()),
            E::Repository(error) => Self::from(error.as_ref()),
            E::RepositoryState(error) => Self::from(error.as_ref()),
            E::GitHost(error) => Self::from(error.as_ref()),
            E::Watcher(error) => Self::from(error.as_ref()),
            E::Subscription(error) => Self::from(error.as_ref()),
            E::Code(error) => Self::from(error.as_ref()),
            E::Review(error) => Self::from(error.as_ref()),
            E::AppConfig(error) => Self::from(error.as_ref()),
            E::Notion(error) => Self::from(error.as_ref()),
            E::Editor(error) => Self::from(error.as_ref()),
            E::HookHealth(error) => Self::from(error.as_ref()),
            E::Technical(error) => Self::from(error.as_ref()),
            E::WorkspaceState(error) => Self::from(error.as_ref()),
        }
    }
}

impl From<&crate::usecase::terminal_surface::error::UsecaseError> for Failure {
    fn from(error: &crate::usecase::terminal_surface::error::UsecaseError) -> Self {
        use crate::usecase::terminal_surface::error::UsecaseError as E;
        match error {
            E::Technical(error) => Self::from(error),
            E::NotFound(_) | E::InvalidOperation(_) | E::StaleAttachment | E::OwnerConflict => {
                Self::Business(BusinessFailure::Other)
            }
        }
    }
}

impl From<&crate::usecase::agent_session::AgentSessionReadUsecaseError> for Failure {
    fn from(error: &crate::usecase::agent_session::AgentSessionReadUsecaseError) -> Self {
        use crate::usecase::agent_session::AgentSessionReadUsecaseError as E;
        match error {
            E::Lifecycle(error) => Self::from(error),
            E::Store(error) => Self::from(error),
            E::InvalidRequest => Self::Business(BusinessFailure::Other),
            E::StorageUnavailable => Self::Technical(TechnicalFailureNature::Transient),
            E::Corrupt => Self::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::usecase::agent_session::AgentSessionLifecycleUsecaseError> for Failure {
    fn from(error: &crate::usecase::agent_session::AgentSessionLifecycleUsecaseError) -> Self {
        use crate::usecase::agent_session::AgentSessionLifecycleUsecaseError as E;
        match error {
            E::Workflow(error) => Self::from(error),
            E::Store(error) | E::Conflict(error) => Self::from(error),
            E::Launch(error) => Self::from(error),
            E::Terminal(error) => Self::from(error),
            E::NotFound | E::InvalidOperation | E::ProviderUnavailable => {
                Self::Business(BusinessFailure::Other)
            }
            E::StorageUnavailable => Self::Technical(TechnicalFailureNature::Transient),
            E::Corrupt => Self::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::domain::agent_session::ProviderAgentLaunchGatewayError> for Failure {
    fn from(error: &crate::domain::agent_session::ProviderAgentLaunchGatewayError) -> Self {
        use crate::domain::agent_session::ProviderAgentLaunchGatewayError as E;
        match error {
            E::Technical(error) => Self::from(error),
            E::InvalidInput => Self::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::agent_session::ProviderAgentTerminalGatewayError> for Failure {
    fn from(error: &crate::domain::agent_session::ProviderAgentTerminalGatewayError) -> Self {
        use crate::domain::agent_session::ProviderAgentTerminalGatewayError as E;
        match error {
            E::Technical(error) => Self::from(error),
            E::NotFound(_) | E::InvalidOperation(_) | E::StaleAttachment | E::OwnerConflict => {
                Self::Business(BusinessFailure::Other)
            }
        }
    }
}

impl From<&crate::usecase::agent_session::AgentSessionHistoryQueryError> for Failure {
    fn from(error: &crate::usecase::agent_session::AgentSessionHistoryQueryError) -> Self {
        use crate::usecase::agent_session::AgentSessionHistoryQueryError as E;
        match error {
            E::Technical(error) => Self::from(error),
            E::Store(error) => Self::from(error),
            E::Conflict => Self::Business(BusinessFailure::VersionConflict),
            E::InvalidRequest | E::ProviderSessionAlreadyOwned { .. } => {
                Self::Business(BusinessFailure::Other)
            }
            E::Corrupt => Self::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::usecase::agent_session::ProviderAvailabilityUsecaseError> for Failure {
    fn from(error: &crate::usecase::agent_session::ProviderAvailabilityUsecaseError) -> Self {
        use crate::usecase::agent_session::ProviderAvailabilityUsecaseError as E;
        match error {
            E::Config(error) => Self::from(error),
            E::Refresh(error) => Self::from(error),
            E::InvalidInput => Self::Business(BusinessFailure::Other),
            E::Corrupt => Self::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::usecase::app_config::error::UsecaseError> for Failure {
    fn from(error: &crate::usecase::app_config::error::UsecaseError) -> Self {
        use crate::usecase::app_config::error::UsecaseError as E;
        match error {
            E::AppConfig(error) => Self::from(error),
            E::InvalidInput(_) => Self::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::usecase::notion::error::NotionUsecaseError> for Failure {
    fn from(error: &crate::usecase::notion::error::NotionUsecaseError) -> Self {
        use crate::usecase::notion::error::NotionUsecaseError as E;
        match error {
            E::AppConfig(error) => Self::from(error),
            E::Notion(error) => Self::from(error),
            E::ConfigNotFound => Self::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::notion::NotionError> for Failure {
    fn from(error: &crate::domain::notion::NotionError) -> Self {
        use crate::domain::notion::NotionError as E;
        match error {
            E::Technical(error) => Self::from(error),
            E::ApiError(_) => Self::Business(BusinessFailure::Other),
            E::RequestFailed(_) => Self::Technical(TechnicalFailureNature::Transient),
            E::ParseError(_) => Self::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError> for Failure {
    fn from(error: &crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError) -> Self {
        use crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError as E;
        match error {
            E::Technical(error) => Self::from(error),
            E::Store(error) => Self::from(error),
            E::Conflict => Self::Business(BusinessFailure::VersionConflict),
            E::InvalidInput => Self::Business(BusinessFailure::Other),
            E::StorageUnavailable => Self::Technical(TechnicalFailureNature::Transient),
            E::Corrupt => Self::Technical(TechnicalFailureNature::Other),
        }
    }
}

impl From<&crate::usecase::state_subscription::SubscriptionError> for Failure {
    fn from(error: &crate::usecase::state_subscription::SubscriptionError) -> Self {
        use crate::usecase::state_subscription::SubscriptionError as E;
        match error {
            E::EncodingFailed | E::VersionExhausted => {
                Self::Technical(TechnicalFailureNature::Other)
            }
            E::InvalidId
            | E::AlreadyExists
            | E::StreamEnded
            | E::UnknownTarget
            | E::SnapshotRequired => Self::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::comment::ReviewError> for Failure {
    fn from(error: &crate::domain::comment::ReviewError) -> Self {
        use crate::domain::comment::ReviewError as E;
        match error {
            E::Technical(error) => Self::from(error),
            E::Io(_) | E::Serialize(_) => Self::Technical(TechnicalFailureNature::Other),
            E::InvalidInput(_)
            | E::NotFound(_)
            | E::AlreadyResolved(_)
            | E::PermissionDenied(_) => Self::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::workspace_state::WorkspaceStateError> for Failure {
    fn from(error: &crate::domain::workspace_state::WorkspaceStateError) -> Self {
        match error {
            crate::domain::workspace_state::WorkspaceStateError::Message(_) => {
                Self::Technical(TechnicalFailureNature::Other)
            }
        }
    }
}

impl From<&crate::domain::agent_session::ProviderExecutableConfigRepositoryError> for Failure {
    fn from(error: &crate::domain::agent_session::ProviderExecutableConfigRepositoryError) -> Self {
        use crate::domain::agent_session::ProviderExecutableConfigRepositoryError as E;
        match error {
            E::Technical(error) => Self::from(error),
            E::InvalidInput => Self::Business(BusinessFailure::Other),
        }
    }
}

impl From<&crate::domain::agent_session::ProviderExecutableProbeGatewayError> for Failure {
    fn from(error: &crate::domain::agent_session::ProviderExecutableProbeGatewayError) -> Self {
        match error {
            crate::domain::agent_session::ProviderExecutableProbeGatewayError::Technical(error) => {
                Self::from(error)
            }
        }
    }
}

#[cfg(test)]
#[path = "work_failure_test.rs"]
mod work_failure_tests;
