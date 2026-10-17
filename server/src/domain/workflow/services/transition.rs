//! Pure workflow approval transition decisions.

use crate::domain::workflow::value_objects::NodeDefinition;

/// 既定の完了条件（session 二信号 / command exit code / fanout 全子完了）を
/// 満たした node の処遇。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionDisposition {
    Complete,
    RequestApproval,
}

pub fn decide_completion_disposition(node: &NodeDefinition) -> CompletionDisposition {
    if node.requires_approval_completion() {
        CompletionDisposition::RequestApproval
    } else {
        CompletionDisposition::Complete
    }
}

pub(crate) fn decide_session_completion(
    signals: crate::domain::workflow::NodeCompletionSignalState,
    standalone: bool,
    node: Option<&NodeDefinition>,
) -> crate::domain::workflow::entities::workflow_execution::NodeCompletionHandshakeDecision {
    use crate::domain::workflow::entities::workflow_execution::NodeCompletionHandshakeDecision as D;
    if !signals.is_ready() {
        return D::AwaitingSignal;
    }
    if standalone {
        return D::CompleteAuto;
    }
    match node
        .filter(|node| node.is_session())
        .map(decide_completion_disposition)
    {
        Some(CompletionDisposition::Complete) => D::CompleteAuto,
        Some(CompletionDisposition::RequestApproval) => D::RequestApproval,
        None => D::NotApplicable,
    }
}
