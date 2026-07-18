use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolverError, WorkflowDefinitionResolverError,
};
use crate::domain::workflow::NodeExecutionFailureKind;
use crate::usecase::agent_session::runtime::usecase::AgentRuntimeError;

/// ワークフローエンジンのエラー型。
#[derive(Debug)]
pub enum WorkflowEngineError {
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

impl std::fmt::Display for WorkflowEngineError {
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

impl WorkflowEngineError {
    pub(crate) fn workflow_failure_kind(&self) -> NodeExecutionFailureKind {
        match self {
            Self::AgentRuntime { failure_kind, .. } => *failure_kind,
            Self::SessionStore(_) | Self::AgentSession(_) => {
                NodeExecutionFailureKind::InfrastructureCrash
            }
            Self::ExecutionNotFound(_)
            | Self::SessionNotFound(_)
            | Self::InvalidWorkflow(_)
            | Self::AlreadyActive(_)
            | Self::InvalidState(_)
            | Self::ValidationError(_)
            | Self::UnauthorizedWorktree(_)
            | Self::UnauthorizedApprovalTarget(_) => NodeExecutionFailureKind::ValidationFailure,
        }
    }

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
        match error {
            error @ AgentRuntimeError::StartupTimeout { retry_count, .. } => Self::AgentRuntime {
                message: format!("{context}: {error}"),
                failure_kind: NodeExecutionFailureKind::StartupTimeout,
                retry_count: Some(retry_count),
            },
            AgentRuntimeError::Other(message) => {
                Self::AgentSession(format!("{context}: {message}"))
            }
            AgentRuntimeError::BackendSelectionLocked => Self::AgentSession(format!(
                "{context}: {}",
                AgentRuntimeError::BackendSelectionLocked
            )),
        }
    }
}

impl From<AgentRuntimeError> for WorkflowEngineError {
    fn from(error: AgentRuntimeError) -> Self {
        match error {
            error @ AgentRuntimeError::StartupTimeout { retry_count, .. } => Self::AgentRuntime {
                message: error.to_string(),
                failure_kind: NodeExecutionFailureKind::StartupTimeout,
                retry_count: Some(retry_count),
            },
            AgentRuntimeError::BackendSelectionLocked => {
                Self::AgentSession(AgentRuntimeError::BackendSelectionLocked.to_string())
            }
            AgentRuntimeError::Other(message) => Self::AgentSession(message),
        }
    }
}

impl From<WorkflowEngineError> for String {
    fn from(e: WorkflowEngineError) -> Self {
        e.to_string()
    }
}

impl From<WorkflowDefinitionResolverError> for WorkflowEngineError {
    fn from(e: WorkflowDefinitionResolverError) -> Self {
        match e {
            WorkflowDefinitionResolverError::InvalidWorkflow(message) => {
                Self::InvalidWorkflow(message)
            }
            WorkflowDefinitionResolverError::Infrastructure(message) => Self::SessionStore(message),
        }
    }
}

impl From<ManagedWorktreeResolverError> for WorkflowEngineError {
    fn from(e: ManagedWorktreeResolverError) -> Self {
        match e {
            ManagedWorktreeResolverError::Validation(message) => Self::ValidationError(message),
        }
    }
}

impl From<crate::usecase::workflow::node_lifecycle::NodeExecutionLifecycleError>
    for WorkflowEngineError
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

pub(crate) fn workflow_error_to_engine_error(
    err: crate::domain::workflow::WorkflowError,
) -> WorkflowEngineError {
    match err {
        crate::domain::workflow::WorkflowError::InvalidState(message) => {
            WorkflowEngineError::InvalidState(message)
        }
        crate::domain::workflow::WorkflowError::Validation(message) => {
            if let Some(node_name) = message.strip_prefix("node not found: ") {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "Node '{node_name}' not found in workflow"
                ))
            } else {
                WorkflowEngineError::InvalidWorkflow(message)
            }
        }
        crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(message) => {
            WorkflowEngineError::UnauthorizedApprovalTarget(message)
        }
        crate::domain::workflow::WorkflowError::NotFound(message)
        | crate::domain::workflow::WorkflowError::External(message) => {
            WorkflowEngineError::InvalidWorkflow(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::runtime::usecase::AgentRuntimeError;

    #[test]
    fn workflow_failure_kind_preserves_validation_and_infrastructure_boundary() {
        let validation_errors = [
            WorkflowEngineError::InvalidWorkflow("missing facet".to_string()),
            WorkflowEngineError::ValidationError("bad output".to_string()),
            WorkflowEngineError::InvalidState("not accepting output".to_string()),
            WorkflowEngineError::UnauthorizedApprovalTarget("wrong execution".to_string()),
        ];
        for error in validation_errors {
            assert_eq!(
                error.workflow_failure_kind(),
                NodeExecutionFailureKind::ValidationFailure,
                "unexpected failure kind for {error:?}"
            );
        }

        let infrastructure_errors = [
            WorkflowEngineError::SessionStore("io".to_string()),
            WorkflowEngineError::AgentSession("backend unavailable".to_string()),
        ];
        for error in infrastructure_errors {
            assert_eq!(
                error.workflow_failure_kind(),
                NodeExecutionFailureKind::InfrastructureCrash,
                "unexpected failure kind for {error:?}"
            );
        }
    }

    #[test]
    fn workflow_failure_kind_preserves_agent_runtime_metadata() {
        let error = WorkflowEngineError::from(AgentRuntimeError::StartupTimeout {
            retry_count: 1,
            max_retries: 2,
        });

        assert_eq!(
            error.workflow_failure_kind(),
            NodeExecutionFailureKind::StartupTimeout
        );
        assert_eq!(error.retry_count(), Some(1));
    }
}
