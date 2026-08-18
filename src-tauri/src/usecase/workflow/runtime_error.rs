use crate::domain::workflow::NodeExecutionFailureKind;
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
}

impl std::fmt::Display for WorkflowRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExecutionNotFound(id) => {
                write!(f, "No workflow execution found for session '{id}'")
            }
            Self::SessionNotFound(id) => write!(f, "AgentSession not found: {id}"),
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
        }
    }
}

impl WorkflowRuntimeError {
    pub(crate) fn workflow_failure_kind(&self) -> NodeExecutionFailureKind {
        match self {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
