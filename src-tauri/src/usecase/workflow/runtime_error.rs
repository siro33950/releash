use crate::domain::local_event::SessionOperationFailureKind;
use crate::domain::workflow::NodeExecutionFailureKind;
use crate::usecase::agent_session::runtime::usecase::{
    AgentRuntimeError, DurableWorkflowSendError,
};
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolverError, WorkflowDefinitionResolverError,
};

/// Workflow runtime boundary error.
#[derive(Debug)]
pub enum WorkflowRuntimeError {
    /// ワークフロー実行が見つからない
    ExecutionNotFound(String),
    /// セッションが見つからない
    SessionNotFound(String),
    /// ワークフロー定義エラー（ステップなし、ステップ未発見等）
    InvalidWorkflow(String),
    /// ワークフローが既にアクティブ
    AlreadyActive(String),
    /// 不正な状態遷移（WaitingApprovalでない時にapproval等）
    InvalidState(String),
    /// Workflow stream head が候補作成後に進んだため再評価が必要
    Conflict(String),
    /// 入力検証エラー（表示用の安定 kind: validation_error）
    ValidationError(String),
    /// 承認操作が指定 worktree の実行を対象にしていない
    UnauthorizedWorktree(String),
    /// 承認操作が現在の execution / node を対象にしていない
    UnauthorizedApprovalTarget(String),
    /// セッションストアのIO/シリアライズエラー
    SessionStore(String),
    /// AgentSession起動エラー
    AgentSession(String),
    /// Agent runtime が分類済み failure metadata とともに返したエラー
    AgentRuntime {
        message: String,
        failure_kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
    },
}

impl std::fmt::Display for WorkflowRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExecutionNotFound(id) => {
                write!(f, "No workflow execution found for session '{id}'")
            }
            Self::SessionNotFound(id) => write!(f, "ChatSession not found: {id}"),
            Self::InvalidWorkflow(msg) => write!(f, "{msg}"),
            Self::AlreadyActive(name) => {
                write!(f, "Workflow '{name}' is already running for this session")
            }
            Self::InvalidState(msg) => write!(f, "invalid_state: {msg}"),
            Self::Conflict(msg) => write!(f, "conflict: {msg}"),
            Self::ValidationError(msg) => write!(f, "validation_error: {msg}"),
            Self::UnauthorizedWorktree(msg) => write!(f, "unauthorized_worktree: {msg}"),
            Self::UnauthorizedApprovalTarget(msg) => {
                write!(f, "unauthorized_approval_target: {msg}")
            }
            Self::SessionStore(msg) | Self::AgentSession(msg) => write!(f, "{msg}"),
            Self::AgentRuntime { message, .. } => write!(f, "{message}"),
        }
    }
}

impl WorkflowRuntimeError {
    pub(crate) fn workflow_failure_kind(&self) -> NodeExecutionFailureKind {
        match self {
            Self::AgentRuntime { failure_kind, .. } => *failure_kind,
            Self::SessionStore(_) => NodeExecutionFailureKind::InfrastructureCrash,
            Self::AgentSession(_) => NodeExecutionFailureKind::ValidationFailure,
            Self::ExecutionNotFound(_)
            | Self::SessionNotFound(_)
            | Self::InvalidWorkflow(_)
            | Self::AlreadyActive(_)
            | Self::InvalidState(_)
            | Self::Conflict(_)
            | Self::ValidationError(_)
            | Self::UnauthorizedWorktree(_)
            | Self::UnauthorizedApprovalTarget(_) => NodeExecutionFailureKind::ValidationFailure,
        }
    }

    #[cfg(test)]
    pub(crate) fn retry_count(&self) -> Option<u32> {
        match self {
            Self::AgentRuntime { retry_count, .. } => *retry_count,
            _ => None,
        }
    }

    pub(crate) fn with_agent_runtime_context(
        context: impl Into<String>,
        error: AgentRuntimeError,
    ) -> Self {
        let context = context.into();
        let (failure_kind, retry_count) = agent_runtime_failure_metadata(&error);
        Self::AgentRuntime {
            message: format!("{context}: {error}"),
            failure_kind,
            retry_count,
        }
    }
}

impl From<AgentRuntimeError> for WorkflowRuntimeError {
    fn from(error: AgentRuntimeError) -> Self {
        let (failure_kind, retry_count) = agent_runtime_failure_metadata(&error);
        Self::AgentRuntime {
            message: error.to_string(),
            failure_kind,
            retry_count,
        }
    }
}

fn agent_runtime_failure_metadata(
    error: &AgentRuntimeError,
) -> (NodeExecutionFailureKind, Option<u32>) {
    match error {
        AgentRuntimeError::StartupTimeout { retry_count, .. } => {
            (NodeExecutionFailureKind::StartupTimeout, Some(*retry_count))
        }
        AgentRuntimeError::BackendSessionLost { .. } => {
            (NodeExecutionFailureKind::InfrastructureCrash, None)
        }
        AgentRuntimeError::WorkflowTurnSend(error) => {
            (workflow_turn_send_failure_kind(error), None)
        }
        AgentRuntimeError::WorkspaceQuery(
            crate::domain::workflow::WorkflowError::StorageUnavailable { .. },
        ) => (NodeExecutionFailureKind::InfrastructureCrash, None),
        AgentRuntimeError::BackendSelectionLocked
        | AgentRuntimeError::AcceptedEffectAdmissionDeferred
        | AgentRuntimeError::AcceptedEffectAdmissionFailed { .. }
        | AgentRuntimeError::WorkspaceQuery(_)
        | AgentRuntimeError::Other(_) => (NodeExecutionFailureKind::ValidationFailure, None),
    }
}

fn workflow_turn_send_failure_kind(error: &DurableWorkflowSendError) -> NodeExecutionFailureKind {
    match error {
        DurableWorkflowSendError::SessionStore(_) => NodeExecutionFailureKind::InfrastructureCrash,
        DurableWorkflowSendError::Admission(failure)
            if matches!(
                failure.kind,
                SessionOperationFailureKind::StorageUnavailable
                    | SessionOperationFailureKind::StorageCorrupt
                    | SessionOperationFailureKind::PersistFailure
            ) =>
        {
            NodeExecutionFailureKind::InfrastructureCrash
        }
        DurableWorkflowSendError::Admission(failure)
            if failure.kind == SessionOperationFailureKind::DeadlineExceeded =>
        {
            NodeExecutionFailureKind::StartupTimeout
        }
        DurableWorkflowSendError::SessionNotFound(_)
        | DurableWorkflowSendError::InvalidWorkflowTarget
        | DurableWorkflowSendError::AuthorityMismatch
        | DurableWorkflowSendError::PayloadEncoding
        | DurableWorkflowSendError::Operation(_)
        | DurableWorkflowSendError::Admission(_)
        | DurableWorkflowSendError::OutcomeUnknown(_)
        | DurableWorkflowSendError::IncompatibleReceipt
        | DurableWorkflowSendError::DriverUnavailable => {
            NodeExecutionFailureKind::ValidationFailure
        }
    }
}

impl From<WorkflowRuntimeError> for String {
    fn from(e: WorkflowRuntimeError) -> Self {
        e.to_string()
    }
}

impl From<WorkflowDefinitionResolverError> for WorkflowRuntimeError {
    fn from(e: WorkflowDefinitionResolverError) -> Self {
        match e {
            WorkflowDefinitionResolverError::InvalidWorkflow(message) => {
                Self::InvalidWorkflow(message)
            }
            WorkflowDefinitionResolverError::Infrastructure(message) => Self::SessionStore(message),
        }
    }
}

impl From<ManagedWorktreeResolverError> for WorkflowRuntimeError {
    fn from(e: ManagedWorktreeResolverError) -> Self {
        match e {
            ManagedWorktreeResolverError::Validation(message) => Self::ValidationError(message),
        }
    }
}

impl From<crate::usecase::workflow::node_lifecycle::NodeExecutionLifecycleError>
    for WorkflowRuntimeError
{
    fn from(e: crate::usecase::workflow::node_lifecycle::NodeExecutionLifecycleError) -> Self {
        match e {
            crate::usecase::workflow::node_lifecycle::NodeExecutionLifecycleError::SessionNotFound(id) => {
                Self::SessionNotFound(id)
            }
            crate::usecase::workflow::node_lifecycle::NodeExecutionLifecycleError::SessionStore(message) => {
                Self::SessionStore(message)
            }
            crate::usecase::workflow::node_lifecycle::NodeExecutionLifecycleError::AgentSession(message) => {
                Self::AgentSession(message)
            }
        }
    }
}

pub(crate) fn workflow_error_to_runtime_error(
    err: crate::domain::workflow::WorkflowError,
) -> WorkflowRuntimeError {
    match err {
        crate::domain::workflow::WorkflowError::InvalidState(message) => {
            WorkflowRuntimeError::InvalidState(message)
        }
        crate::domain::workflow::WorkflowError::Conflict(message) => {
            WorkflowRuntimeError::Conflict(message)
        }
        crate::domain::workflow::WorkflowError::Validation(message) => {
            if let Some(node_name) = message.strip_prefix("node not found: ") {
                WorkflowRuntimeError::InvalidWorkflow(format!(
                    "Node '{node_name}' not found in workflow"
                ))
            } else {
                WorkflowRuntimeError::InvalidWorkflow(message)
            }
        }
        crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(message) => {
            WorkflowRuntimeError::UnauthorizedApprovalTarget(message)
        }
        crate::domain::workflow::WorkflowError::NotFound(message)
        | crate::domain::workflow::WorkflowError::External(message) => {
            WorkflowRuntimeError::InvalidWorkflow(message)
        }
        crate::domain::workflow::WorkflowError::StorageUnavailable { message, .. }
        | crate::domain::workflow::WorkflowError::CorruptStoredState(message)
        | crate::domain::workflow::WorkflowError::IncompatibleStoredEvent(message) => {
            WorkflowRuntimeError::SessionStore(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::runtime::usecase::AgentRuntimeError;

    #[test]
    fn workflow_failure_kind_only_uses_crash_for_storage_or_process_loss() {
        let validation_errors = [
            WorkflowRuntimeError::InvalidWorkflow("missing facet".to_string()),
            WorkflowRuntimeError::ValidationError("bad output".to_string()),
            WorkflowRuntimeError::InvalidState("not accepting output".to_string()),
            WorkflowRuntimeError::UnauthorizedApprovalTarget("wrong execution".to_string()),
        ];
        for error in validation_errors {
            assert_eq!(
                error.workflow_failure_kind(),
                NodeExecutionFailureKind::ValidationFailure,
                "unexpected failure kind for {error:?}"
            );
        }

        assert_eq!(
            WorkflowRuntimeError::SessionStore("io".to_string()).workflow_failure_kind(),
            NodeExecutionFailureKind::InfrastructureCrash
        );
        assert_eq!(
            WorkflowRuntimeError::AgentSession("admission rejected".to_string())
                .workflow_failure_kind(),
            NodeExecutionFailureKind::ValidationFailure
        );
        assert_eq!(
            WorkflowRuntimeError::from(AgentRuntimeError::BackendSessionLost {
                requested_resume_id: "lost".to_string(),
            })
            .workflow_failure_kind(),
            NodeExecutionFailureKind::InfrastructureCrash
        );
    }

    #[test]
    fn workflow_failure_kind_preserves_agent_runtime_metadata() {
        let error = WorkflowRuntimeError::from(AgentRuntimeError::StartupTimeout {
            retry_count: 1,
            max_retries: 2,
        });

        assert_eq!(
            error.workflow_failure_kind(),
            NodeExecutionFailureKind::StartupTimeout
        );
        assert_eq!(error.retry_count(), Some(1));
    }

    #[test]
    fn workflow_send_failure_keeps_typed_admission_meaning() {
        let business_rejection =
            AgentRuntimeError::WorkflowTurnSend(DurableWorkflowSendError::Admission(
                crate::domain::local_event::SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "not runnable",
                    "business-rejection",
                ),
            ));
        assert_eq!(
            WorkflowRuntimeError::from(business_rejection).workflow_failure_kind(),
            NodeExecutionFailureKind::ValidationFailure
        );

        let storage_loss =
            AgentRuntimeError::WorkflowTurnSend(DurableWorkflowSendError::Admission(
                crate::domain::local_event::SafeOperationFailure::new(
                    SessionOperationFailureKind::PersistFailure,
                    true,
                    "store unavailable",
                    "storage-loss",
                ),
            ));
        assert_eq!(
            WorkflowRuntimeError::from(storage_loss).workflow_failure_kind(),
            NodeExecutionFailureKind::InfrastructureCrash
        );
    }
}
