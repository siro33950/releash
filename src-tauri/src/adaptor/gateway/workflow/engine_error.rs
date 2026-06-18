use crate::adaptor::gateway::workflow::event::CliMutationRejectionReason;
use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolverError, WorkflowDefinitionResolverError,
};

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
