use crate::adaptor::protocol::connect::{rpc, to_rpc};

pub(crate) fn invalid_request(message: impl Into<String>) -> connectrpc::ConnectError {
    connectrpc::ConnectError::new(connectrpc::ErrorCode::InvalidArgument, message.into())
}
pub(crate) fn invalid_response(message: impl Into<String>) -> connectrpc::ConnectError {
    connectrpc::ConnectError::new(connectrpc::ErrorCode::Internal, message.into())
}

pub(crate) fn classified_error(
    error: impl ConnectFailure + std::fmt::Display,
) -> connectrpc::ConnectError {
    connectrpc::ConnectError::new(error.connect_code(), error.to_string())
}

pub(crate) fn command_error(
    error: crate::adaptor::protocol::client::CommandFailure,
) -> connectrpc::ConnectError {
    match to_rpc::<rpc::CommandError>(&error.detail) {
        Ok(detail) => connectrpc::ConnectError::new(error.kind, "Command failed").with_detail(
            connectrpc::ErrorDetail::from_message("releash.client.v1.CommandError", &detail),
        ),
        Err(error) => error,
    }
}

#[cfg(test)]
#[path = "connect_test.rs"]
mod tests;

pub(crate) fn failure_classification(failure: crate::domain::failure::Failure) -> &'static str {
    use crate::domain::failure::{BusinessFailure, Failure, TechnicalFailureNature};
    match failure {
        Failure::Business(BusinessFailure::VersionConflict) => "VersionConflict",
        Failure::Business(BusinessFailure::Other) => "BusinessFailure",
        Failure::Technical(TechnicalFailureNature::Transient) => "Transient",
        Failure::Technical(TechnicalFailureNature::TimedOut) => "TimedOut",
        Failure::Technical(TechnicalFailureNature::Cancelled) => "Cancelled",
        Failure::Technical(TechnicalFailureNature::Other) => "TechnicalFailure",
    }
}

pub(crate) trait ConnectFailure {
    fn connect_code(&self) -> connectrpc::ErrorCode;
}

impl ConnectFailure for crate::usecase::code_error::CodeUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Code(error) => error.connect_code(),
        }
    }
}

impl ConnectFailure for crate::usecase::work_queue::WorkFailure {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        self.kind.connect_code()
    }
}

impl ConnectFailure for crate::usecase::watcher::UsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Subscription(error) => error.connect_code(),
            Self::Repository(error) => error.connect_code(),
            Self::File(_) => connectrpc::ErrorCode::Internal,
            Self::RepositoryUnavailable => connectrpc::ErrorCode::FailedPrecondition,
        }
    }
}

impl ConnectFailure for crate::usecase::repository_error::UsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Workflow(error) => error.connect_code(),
            Self::Repository(error) => error.connect_code(),
            Self::Rule(_) => connectrpc::ErrorCode::FailedPrecondition,
        }
    }
}

impl ConnectFailure for crate::domain::application_lifecycle::ApplicationLifecycleError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        connectrpc::ErrorCode::Internal
    }
}

impl ConnectFailure for crate::domain::workspace_state::error::WorkspaceStateError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Message(_) => connectrpc::ErrorCode::Internal,
        }
    }
}

impl ConnectFailure for crate::domain::repository::error::RepositoryError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Technical(error) => ConnectFailure::connect_code(error),
            Self::External(_) => connectrpc::ErrorCode::Internal,
            Self::Rule(_) => connectrpc::ErrorCode::FailedPrecondition,
        }
    }
}

impl ConnectFailure for crate::domain::repository::watch_subscriptions::WatchSubscriptionError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::NotFound => connectrpc::ErrorCode::NotFound,
            Self::AlreadyExists => connectrpc::ErrorCode::AlreadyExists,
            Self::Limit => connectrpc::ErrorCode::ResourceExhausted,
        }
    }
}

impl ConnectFailure for crate::domain::agent_session::AgentSessionDisplayNameError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        connectrpc::ErrorCode::InvalidArgument
    }
}

impl ConnectFailure for crate::domain::agent_session::AgentSessionHistoryGatewayError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::ProviderSessionAlreadyOwned { .. } => connectrpc::ErrorCode::FailedPrecondition,
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidRequest => connectrpc::ErrorCode::InvalidArgument,
            Self::Unavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::domain::agent_session::ProviderSessionTitleGatewayError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Unavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::domain::agent_session::repository::AgentSessionRepositoryError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Store(kind) => kind.connect_code(),
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::ProviderSessionAlreadyOwned { .. } => connectrpc::ErrorCode::FailedPrecondition,
            Self::InvalidRequest => connectrpc::ErrorCode::InvalidArgument,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
            Self::Unavailable => connectrpc::ErrorCode::Unavailable,
        }
    }
}

impl ConnectFailure for crate::domain::git_host::git_host::GitHostError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::External(_) => connectrpc::ErrorCode::Internal,
            Self::Technical(error) => ConnectFailure::connect_code(error),
        }
    }
}

impl ConnectFailure for crate::domain::comment::ReviewError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Technical(error) => ConnectFailure::connect_code(error),
            Self::InvalidInput(_) => connectrpc::ErrorCode::InvalidArgument,
            Self::NotFound(_) => connectrpc::ErrorCode::NotFound,
            Self::AlreadyResolved(_) => connectrpc::ErrorCode::FailedPrecondition,
            Self::PermissionDenied(_) => connectrpc::ErrorCode::PermissionDenied,
            Self::Io(_) | Self::Serialize(_) => connectrpc::ErrorCode::Internal,
        }
    }
}

impl ConnectFailure for crate::domain::app_config::error::AppConfigError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Repository(_) => connectrpc::ErrorCode::Internal,
            Self::InvalidInput(_) => connectrpc::ErrorCode::InvalidArgument,
        }
    }
}

impl ConnectFailure for crate::domain::code::error::CodeError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Technical(error) => ConnectFailure::connect_code(error),
            Self::External(_) => connectrpc::ErrorCode::Internal,
            Self::Rule(_) => connectrpc::ErrorCode::FailedPrecondition,
            Self::StaleReviewBlobVersion { .. } | Self::StaleReviewGroupTarget { .. } => {
                connectrpc::ErrorCode::Aborted
            }
        }
    }
}

impl ConnectFailure for crate::domain::external_editor::gateway::EditorError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::InvalidInput(_) => connectrpc::ErrorCode::InvalidArgument,
            Self::Settings(error) => error.connect_code(),
            Self::Launch(_) => connectrpc::ErrorCode::FailedPrecondition,
        }
    }
}

impl ConnectFailure for crate::domain::local_event::batch::CommitBatchError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Technical(error) => ConnectFailure::connect_code(error),
            Self::PayloadConflict => connectrpc::ErrorCode::FailedPrecondition,
            Self::QueueBusy => connectrpc::ErrorCode::Unavailable,
            Self::StreamHeadConflict { .. }
            | Self::OutcomeUnknown { .. }
            | Self::TreeHeadConflict
            | Self::AppendOutcomeUnknown => connectrpc::ErrorCode::Aborted,
            Self::CapacityExceeded | Self::SequenceExhausted => {
                connectrpc::ErrorCode::ResourceExhausted
            }
            Self::StorageUnavailable { failure } => failure.connect_code(),
            Self::StorageAccessRequired { .. } => connectrpc::ErrorCode::FailedPrecondition,
            Self::Corrupt { .. } => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::domain::local_event::query::LocalEventQueryError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Technical(error) => ConnectFailure::connect_code(error),
            Self::InvalidRequest => connectrpc::ErrorCode::InvalidArgument,
            Self::CanonicalWriterRequired => connectrpc::ErrorCode::Aborted,
            Self::QueryBusy => connectrpc::ErrorCode::Unavailable,
            Self::ResponseTooLarge => connectrpc::ErrorCode::ResourceExhausted,
            Self::IncompatibleStoredEvent { .. } => connectrpc::ErrorCode::FailedPrecondition,
            Self::StorageUnavailable { failure } => failure.connect_code(),
            Self::StorageAccessRequired { .. } => connectrpc::ErrorCode::FailedPrecondition,
            Self::Corrupt { .. } => connectrpc::ErrorCode::DataLoss,
            Self::Internal { .. } => connectrpc::ErrorCode::Internal,
        }
    }
}

impl ConnectFailure for crate::domain::notion::error::NotionError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Technical(error) => ConnectFailure::connect_code(error),
            Self::RequestFailed(_) => connectrpc::ErrorCode::Unavailable,
            Self::ApiError(_) => connectrpc::ErrorCode::FailedPrecondition,
            Self::ParseError(_) => connectrpc::ErrorCode::Internal,
        }
    }
}

impl ConnectFailure for crate::domain::workflow::error::WorkflowError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Technical(error) => error.connect_code(),
            Self::Store(kind) => kind.connect_code(),
            Self::External(_) => connectrpc::ErrorCode::Internal,
            Self::Editor(error) => error.connect_code(),
            Self::CorruptStoredState(_) => connectrpc::ErrorCode::DataLoss,
            Self::IncompatibleStoredEvent(_) | Self::InvalidState(_) => {
                connectrpc::ErrorCode::FailedPrecondition
            }
            Self::Validation(_) => connectrpc::ErrorCode::InvalidArgument,
            Self::Conflict(_) => connectrpc::ErrorCode::Aborted,
            Self::NotFound(_) => connectrpc::ErrorCode::NotFound,
            Self::UnauthorizedApprovalTarget(_) => connectrpc::ErrorCode::PermissionDenied,
        }
    }
}

impl ConnectFailure for crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidInput => connectrpc::ErrorCode::InvalidArgument,
            Self::StorageUnavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidInput => connectrpc::ErrorCode::InvalidArgument,
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::StorageUnavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::domain::state_subscription::SubscriptionError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::InvalidId => connectrpc::ErrorCode::InvalidArgument,
            Self::AlreadyExists => connectrpc::ErrorCode::AlreadyExists,
            Self::StreamEnded | Self::UnknownTarget | Self::SnapshotRequired => {
                connectrpc::ErrorCode::NotFound
            }
            Self::VersionExhausted => connectrpc::ErrorCode::Internal,
        }
    }
}

impl ConnectFailure for crate::usecase::terminal_surface::error::UsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::OwnerConflict => connectrpc::ErrorCode::FailedPrecondition,
            Self::Gateway(_) | Self::PtySpawn { .. } | Self::OtherSpawnFailure { .. } => {
                connectrpc::ErrorCode::Internal
            }
        }
    }
}

impl ConnectFailure for crate::usecase::agent_session::AgentSessionHistoryQueryError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::ProviderSessionAlreadyOwned { .. } => connectrpc::ErrorCode::FailedPrecondition,
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidRequest => connectrpc::ErrorCode::InvalidArgument,
            Self::Unavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::agent_session::AgentSessionReadUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Lifecycle(error) => error.connect_code(),
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidRequest => connectrpc::ErrorCode::InvalidArgument,
            Self::StorageUnavailable => connectrpc::ErrorCode::Unavailable,
            Self::TerminalUnavailable => connectrpc::ErrorCode::FailedPrecondition,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::agent_session::ProviderAvailabilityUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::InvalidInput => connectrpc::ErrorCode::InvalidArgument,
            Self::ConfigUnavailable | Self::RefreshUnavailable => {
                connectrpc::ErrorCode::Unavailable
            }
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::agent_session::AgentSessionRenameError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Store(kind) => kind.connect_code(),
            Self::NotFound => connectrpc::ErrorCode::NotFound,
            Self::InvalidOperation => connectrpc::ErrorCode::FailedPrecondition,
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::ProviderSessionAlreadyOwned => connectrpc::ErrorCode::FailedPrecondition,
            Self::Unavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::agent_session::AgentSessionUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Store(kind) => kind.connect_code(),
            Self::NotFound => connectrpc::ErrorCode::NotFound,
            Self::InvalidOperation => connectrpc::ErrorCode::FailedPrecondition,
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::ProviderSessionAlreadyOwned { .. } => connectrpc::ErrorCode::FailedPrecondition,
            Self::Unavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::agent_session::AgentSessionLifecycleUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Workflow(error) => error.connect_code(),
            Self::Store(kind) => kind.connect_code(),
            Self::NotFound => connectrpc::ErrorCode::NotFound,
            Self::InvalidOperation => connectrpc::ErrorCode::FailedPrecondition,
            Self::Conflict(kind) => kind.connect_code(),
            Self::StorageUnavailable => connectrpc::ErrorCode::Unavailable,
            Self::LaunchUnavailable => connectrpc::ErrorCode::FailedPrecondition,
            Self::TerminalUnavailable => connectrpc::ErrorCode::FailedPrecondition,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::agent_session::AgentSessionLaunchUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Store(kind) => kind.connect_code(),
            Self::Technical(stopped) => stopped.connect_code(),
            Self::ProviderUnavailable => connectrpc::ErrorCode::FailedPrecondition,
            Self::InvalidInput => connectrpc::ErrorCode::InvalidArgument,
            Self::Conflict(kind) => kind.connect_code(),
            Self::StorageUnavailable => connectrpc::ErrorCode::Unavailable,
            Self::LaunchUnavailable => connectrpc::ErrorCode::FailedPrecondition,
            Self::TerminalUnavailable => connectrpc::ErrorCode::FailedPrecondition,
            Self::TerminalSpawn(_) => connectrpc::ErrorCode::FailedPrecondition,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::agent_session::AgentSessionInitialInstructionError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidInput => connectrpc::ErrorCode::InvalidArgument,
            Self::NotFound => connectrpc::ErrorCode::NotFound,
            Self::Conflict(kind) => kind.connect_code(),
            Self::StorageUnavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::agent_session::AgentSessionQueryError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidRequest => connectrpc::ErrorCode::InvalidArgument,
            Self::Unavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::app_config::error::UsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::AppConfig(error) => error.connect_code(),
            Self::InvalidInput(_) => connectrpc::ErrorCode::InvalidArgument,
        }
    }
}

impl ConnectFailure for crate::usecase::notion::error::NotionUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::ConfigNotFound => connectrpc::ErrorCode::FailedPrecondition,
            Self::AppConfig(error) => error.connect_code(),
            Self::Notion(error) => error.connect_code(),
        }
    }
}

impl ConnectFailure for crate::usecase::workflow::runtime_error::WorkflowRuntimeError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Store(failure) => failure.connect_code(),
            Self::Technical(stopped) => stopped.connect_code(),
            Self::AlreadyActive(_) | Self::InvalidState(_) => {
                connectrpc::ErrorCode::FailedPrecondition
            }
            Self::Conflict(_) => connectrpc::ErrorCode::Aborted,
            Self::ExecutionNotFound(_) | Self::SessionNotFound(_) => {
                connectrpc::ErrorCode::NotFound
            }
            Self::InvalidWorkflow(_) | Self::ValidationError(_) => {
                connectrpc::ErrorCode::InvalidArgument
            }
            Self::UnauthorizedWorktree(_) | Self::UnauthorizedApprovalTarget(_) => {
                connectrpc::ErrorCode::PermissionDenied
            }
            Self::SessionStore(_) => connectrpc::ErrorCode::Internal,
            Self::AgentSession(_) => connectrpc::ErrorCode::FailedPrecondition,
        }
    }
}

impl ConnectFailure for crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Unavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidInput => connectrpc::ErrorCode::InvalidArgument,
            Self::StorageUnavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidInput => connectrpc::ErrorCode::InvalidArgument,
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::StorageUnavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Conflict => connectrpc::ErrorCode::Aborted,
            Self::Store(kind) => kind.connect_code(),
            Self::InvalidInput => connectrpc::ErrorCode::InvalidArgument,
            Self::StorageUnavailable => connectrpc::ErrorCode::Unavailable,
            Self::Corrupt => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::usecase::repository_state::error::RepositoryStateError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Background { kind, .. } => kind.connect_code(),
            Self::ScanInvalidated => connectrpc::ErrorCode::Aborted,
            Self::Repository(error) => error.connect_code(),
            Self::Code(error) => error.connect_code(),
            Self::Watcher(_) => connectrpc::ErrorCode::Internal,
        }
    }
}

impl ConnectFailure for crate::usecase::state_subscription::StateReadError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        use crate::usecase::state_subscription::StateReadFailure as S;
        match &self.source {
            S::InvalidTerminalInput => connectrpc::ErrorCode::InvalidArgument,
            S::TerminalSubscriptionEnded => connectrpc::ErrorCode::NotFound,
            S::Workflow(error) => error.connect_code(),
            S::Session(error) => error.connect_code(),
            S::History(error) => error.connect_code(),
            S::Providers(error) => error.connect_code(),
            S::Repository(error) => error.connect_code(),
            S::RepositoryState(error) => error.connect_code(),
            S::GitHost(error) => error.connect_code(),
            S::Watcher(error) => error.connect_code(),
            S::Subscription(error) => error.connect_code(),
            S::Technical(error) => error.connect_code(),
        }
    }
}

impl ConnectFailure for crate::adaptor::presenter::error::AppError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Presented { kind, .. } => *kind,
            Self::Internal(_) => connectrpc::ErrorCode::Internal,
            Self::Coded { kind, .. } => *kind,
        }
    }
}

impl ConnectFailure for crate::domain::failure::TechnicalFailureNature {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Transient => connectrpc::ErrorCode::Unavailable,
            Self::TimedOut => connectrpc::ErrorCode::DeadlineExceeded,
            Self::Cancelled => connectrpc::ErrorCode::Canceled,
            Self::Other => connectrpc::ErrorCode::Internal,
        }
    }
}

impl ConnectFailure for crate::domain::failure::TechnicalFailure {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        self.nature.connect_code()
    }
}

impl ConnectFailure for crate::adaptor::gateway::workflow::fact_log::FactReadError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Query(error) => error.connect_code(),
            Self::Corrupt(_) => connectrpc::ErrorCode::DataLoss,
        }
    }
}

impl ConnectFailure for crate::domain::local_event::SafeOperationFailure {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        self.nature.connect_code()
    }
}

impl ConnectFailure for crate::domain::failure::StorageFailure {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        use crate::domain::failure::StorageFailureSource;
        let source_code = match &self.source {
            StorageFailureSource::Commit(error) => error.connect_code(),
            StorageFailureSource::Query(error) => error.connect_code(),
            StorageFailureSource::Technical(error) => error.connect_code(),
            StorageFailureSource::Editor(error) => error.connect_code(),
            StorageFailureSource::Workflow(error) => error.connect_code(),
            StorageFailureSource::Repository(error) => error.connect_code(),
            StorageFailureSource::AgentSession(error) => error.connect_code(),
        };
        match source_code {
            connectrpc::ErrorCode::Internal
            | connectrpc::ErrorCode::Unavailable
            | connectrpc::ErrorCode::DeadlineExceeded
            | connectrpc::ErrorCode::Canceled => self.nature.connect_code(),
            code => code,
        }
    }
}

impl ConnectFailure for crate::domain::failure::Failure {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        use crate::domain::failure::{BusinessFailure, Failure};
        match self {
            Failure::Business(BusinessFailure::VersionConflict) => connectrpc::ErrorCode::Aborted,
            Failure::Business(BusinessFailure::Other) => connectrpc::ErrorCode::FailedPrecondition,
            Failure::Technical(nature) => nature.connect_code(),
        }
    }
}

impl ConnectFailure for crate::usecase::application_startup::ApplicationUnavailable {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        connectrpc::ErrorCode::FailedPrecondition
    }
}

pub(crate) fn request_capacity_error() -> connectrpc::ConnectError {
    let message = "Too many pending client commands";
    let mut error = command_error(
        super::error::AppError::capacity_exceeded(message)
            .with_code("CLIENT_REQUEST_LIMIT")
            .into(),
    );
    error.message = Some(message.into());
    error
}

impl ConnectFailure for super::provider_tui::ProviderTuiCodedError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        use super::provider_tui::ProviderTuiCodedError;
        use connectrpc::ErrorCode as F;
        match self {
            ProviderTuiCodedError::AgentSessionLaunchUnavailable(kind)
            | ProviderTuiCodedError::AgentSessionTerminalUnavailable(kind) => *kind,
            ProviderTuiCodedError::ProviderAvailabilityInvalidExecutable
            | ProviderTuiCodedError::AgentSessionInvalidProvider(_)
            | ProviderTuiCodedError::AgentSessionInvalidInput(_)
            | ProviderTuiCodedError::ProviderHookHealthInvalidRequest => F::InvalidArgument,
            ProviderTuiCodedError::ProviderAvailabilityConfigUnavailable
            | ProviderTuiCodedError::ProviderAvailabilityRefreshUnavailable
            | ProviderTuiCodedError::AgentSessionStorageUnavailable
            | ProviderTuiCodedError::ProviderHookHealthStorageUnavailable => F::Unavailable,
            ProviderTuiCodedError::ProviderAvailabilityCorrupt
            | ProviderTuiCodedError::AgentSessionCorrupt
            | ProviderTuiCodedError::ProviderHookHealthCorrupt => F::DataLoss,
            ProviderTuiCodedError::AgentSessionProviderUnavailable
            | ProviderTuiCodedError::AgentSessionInvalidOperation => F::FailedPrecondition,
            ProviderTuiCodedError::AgentSessionConflict(_) => F::Aborted,
            ProviderTuiCodedError::AgentSessionNotFound => F::NotFound,
        }
    }
}

#[cfg(test)]
#[path = "connect_mapping_test.rs"]
mod connect_mapping_tests;
