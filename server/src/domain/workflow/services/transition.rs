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
