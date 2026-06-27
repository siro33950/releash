use crate::adaptor::gateway::workflow::event::CliMutationRejectionReason;
use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolverError, WorkflowDefinitionResolverError,
};
use crate::domain::workflow::WorkflowStepFailureKind;
use crate::infrastructure::agent_session::runtime::AgentRuntimeError;

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
    /// 承認操作が現在の execution / step を対象にしていない
    UnauthorizedApprovalTarget(String),
    /// セッションストアのIO/シリアライズエラー
    SessionStore(String),
    /// AgentSession起動エラー
    AgentSession(String),
    /// Agent runtime が分類済み failure metadata とともに返したエラー
    AgentRuntime {
        message: String,
        failure_kind: WorkflowStepFailureKind,
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
    pub(crate) fn workflow_failure_kind(&self) -> WorkflowStepFailureKind {
        match self {
            Self::AgentRuntime { failure_kind, .. } => *failure_kind,
            Self::SessionStore(_) | Self::AgentSession(_) => {
                WorkflowStepFailureKind::InfrastructureCrash
            }
            Self::ExecutionNotFound(_)
            | Self::SessionNotFound(_)
            | Self::InvalidWorkflow(_)
            | Self::AlreadyActive(_)
            | Self::InvalidState(_)
            | Self::ValidationError(_)
            | Self::UnauthorizedWorktree(_)
            | Self::UnauthorizedApprovalTarget(_) => WorkflowStepFailureKind::ValidationFailure,
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
                failure_kind: WorkflowStepFailureKind::StartupTimeout,
                retry_count: Some(retry_count),
            },
            AgentRuntimeError::Other(message) => {
                Self::AgentSession(format!("{context}: {message}"))
            }
        }
    }
}

impl From<AgentRuntimeError> for WorkflowEngineError {
    fn from(error: AgentRuntimeError) -> Self {
        match error {
            error @ AgentRuntimeError::StartupTimeout { retry_count, .. } => Self::AgentRuntime {
                message: error.to_string(),
                failure_kind: WorkflowStepFailureKind::StartupTimeout,
                retry_count: Some(retry_count),
            },
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

impl From<crate::usecase::workflow::step_lifecycle::WorkflowStepLifecycleError>
    for WorkflowEngineError
{
    fn from(e: crate::usecase::workflow::step_lifecycle::WorkflowStepLifecycleError) -> Self {
        match e {
            crate::usecase::workflow::step_lifecycle::WorkflowStepLifecycleError::SessionNotFound(id) => {
                Self::SessionNotFound(id)
            }
            crate::usecase::workflow::step_lifecycle::WorkflowStepLifecycleError::SessionStore(message) => {
                Self::SessionStore(message)
            }
            crate::usecase::workflow::step_lifecycle::WorkflowStepLifecycleError::AgentSession(message) => {
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
                    "Step '{node_name}' not found in workflow"
                ))
            } else {
                WorkflowEngineError::InvalidWorkflow(message)
            }
        }
        crate::domain::workflow::WorkflowError::AlreadyActive(message) => {
            WorkflowEngineError::AlreadyActive(message)
        }
        crate::domain::workflow::WorkflowError::UnauthorizedWorktree(message) => {
            WorkflowEngineError::UnauthorizedWorktree(message)
        }
        crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(message) => {
            WorkflowEngineError::UnauthorizedApprovalTarget(message)
        }
        crate::domain::workflow::WorkflowError::NotFound(message)
        | crate::domain::workflow::WorkflowError::Rule(message)
        | crate::domain::workflow::WorkflowError::External(message) => {
            WorkflowEngineError::InvalidWorkflow(message)
        }
    }
}

pub(crate) fn should_commit_rejected_external_request(error: &WorkflowEngineError) -> bool {
    matches!(
        error,
        WorkflowEngineError::ValidationError(_)
            | WorkflowEngineError::InvalidState(_)
            | WorkflowEngineError::UnauthorizedApprovalTarget(_)
            | WorkflowEngineError::UnauthorizedWorktree(_)
    )
}

/// Classifies engine-rejected external workflow mutations for the auxiliary
/// `CliMutationRejected` event. Human-readable detail remains in the event
/// message; this is intentionally coarse-grained observability metadata.
pub(crate) fn classify_cli_mutation_rejection_reason(
    error: &WorkflowEngineError,
) -> CliMutationRejectionReason {
    use CliMutationRejectionReason::*;
    match error {
        WorkflowEngineError::ExecutionNotFound(_) => RunNotFound,
        WorkflowEngineError::UnauthorizedApprovalTarget(_) => NotWaitingApproval,
        WorkflowEngineError::UnauthorizedWorktree(_) => Other,
        WorkflowEngineError::ValidationError(msg) => {
            if msg.contains("contract mismatch") {
                ContractMismatch
            } else if msg.contains("is not a valid submission target") {
                NodeNotFound
            } else {
                Other
            }
        }
        WorkflowEngineError::InvalidState(msg) => {
            if msg.contains("does not allow reject") {
                NoRejectRule
            } else if msg.contains("is not currently accepting structured output") {
                StepNotAccepting
            } else if msg.contains("is already terminal")
                || msg.contains("is not accepting structured output (state:")
            {
                RunNotActive
            } else {
                Other
            }
        }
        // Retryable/internal I/O paths should be gated by
        // `should_commit_rejected_external_request`; keep classification safe.
        _ => Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::event::CliMutationRejectionReason as R;
    use crate::infrastructure::agent_session::runtime::AgentRuntimeError;

    #[test]
    fn workflow_failure_kind_preserves_validation_and_infrastructure_boundary() {
        let validation_errors = [
            WorkflowEngineError::InvalidWorkflow("missing facet".to_string()),
            WorkflowEngineError::ValidationError("bad output".to_string()),
            WorkflowEngineError::InvalidState("not accepting output".to_string()),
            WorkflowEngineError::UnauthorizedApprovalTarget("wrong run".to_string()),
        ];
        for error in validation_errors {
            assert_eq!(
                error.workflow_failure_kind(),
                WorkflowStepFailureKind::ValidationFailure,
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
                WorkflowStepFailureKind::InfrastructureCrash,
                "unexpected failure kind for {error:?}"
            );
        }
    }

    #[test]
    fn workflow_failure_kind_preserves_agent_runtime_metadata() {
        let error = WorkflowEngineError::from(AgentRuntimeError::startup_timeout(1, 2));

        assert_eq!(
            error.workflow_failure_kind(),
            WorkflowStepFailureKind::StartupTimeout
        );
        assert_eq!(error.retry_count(), Some(1));
    }

    #[test]
    fn rejected_external_request_commit_policy_keeps_user_rejections_observable() {
        assert!(should_commit_rejected_external_request(
            &WorkflowEngineError::ValidationError("bad input".to_string())
        ));
        assert!(should_commit_rejected_external_request(
            &WorkflowEngineError::InvalidState("not waiting".to_string())
        ));
        assert!(should_commit_rejected_external_request(
            &WorkflowEngineError::UnauthorizedApprovalTarget("target".to_string())
        ));
        assert!(should_commit_rejected_external_request(
            &WorkflowEngineError::UnauthorizedWorktree("worktree".to_string())
        ));
        assert!(!should_commit_rejected_external_request(
            &WorkflowEngineError::SessionStore("io".to_string())
        ));
        assert!(!should_commit_rejected_external_request(
            &WorkflowEngineError::AgentSession("runtime".to_string())
        ));
    }

    #[test]
    fn cli_mutation_rejection_reason_maps_known_errors() {
        let cases: Vec<(WorkflowEngineError, R)> = vec![
            (
                WorkflowEngineError::ExecutionNotFound("run".to_string()),
                R::RunNotFound,
            ),
            (
                WorkflowEngineError::UnauthorizedApprovalTarget("target".to_string()),
                R::NotWaitingApproval,
            ),
            (
                WorkflowEngineError::ValidationError(
                    "contract mismatch: step 'r' expects 'a', got 'b'".to_string(),
                ),
                R::ContractMismatch,
            ),
            (
                WorkflowEngineError::ValidationError(
                    "step 'r' is not a valid submission target".to_string(),
                ),
                R::NodeNotFound,
            ),
            (
                WorkflowEngineError::InvalidState("Step 'r' does not allow reject".to_string()),
                R::NoRejectRule,
            ),
            (
                WorkflowEngineError::InvalidState(
                    "step 'r' is not currently accepting structured output".to_string(),
                ),
                R::StepNotAccepting,
            ),
            (
                WorkflowEngineError::InvalidState("run x is already terminal".to_string()),
                R::RunNotActive,
            ),
            (
                WorkflowEngineError::InvalidState(
                    "run x is not accepting structured output (state: Completed)".to_string(),
                ),
                R::RunNotActive,
            ),
            (
                WorkflowEngineError::InvalidState("something else".to_string()),
                R::Other,
            ),
        ];
        for (err, expected) in cases {
            let got = classify_cli_mutation_rejection_reason(&err);
            assert_eq!(got, expected, "unexpected reason for error: {err}");
        }
    }
}
