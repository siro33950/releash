//! Pure approval rules and input validation.

use std::collections::HashSet;

use crate::domain::workflow::value_objects::NodeHistoryEntry;
use crate::domain::workflow::WorkflowError;
#[cfg(test)]
use crate::domain::workflow::NODE_STATUS_COMPLETED;

pub const MAX_APPROVAL_COMMENT_CHARS: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalInputError {
    Empty { label: &'static str },
    TooLong { label: &'static str, limit: usize },
}

impl std::fmt::Display for ApprovalInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty { label } => write!(f, "{label} must not be empty"),
            Self::TooLong { label, limit } => write!(f, "{label} exceeds {limit} characters"),
        }
    }
}

impl std::error::Error for ApprovalInputError {}

pub fn validate_optional_comment_text(
    value: Option<&str>,
    label: &'static str,
) -> Result<(), ApprovalInputError> {
    if let Some(text) = value {
        validate_text_length(text, label)?;
    }
    Ok(())
}

pub fn validate_required_comment_text(
    value: &str,
    label: &'static str,
) -> Result<(), ApprovalInputError> {
    if value.trim().is_empty() {
        return Err(ApprovalInputError::Empty { label });
    }
    validate_text_length(value, label)?;
    Ok(())
}

#[cfg(test)]
pub fn should_auto_approve_workflow_approval(
    node_is_waiting_approval: bool,
    approval_auto_approve_enabled: bool,
) -> bool {
    approval_auto_approve_enabled && node_is_waiting_approval
}

pub struct ApprovalChatInstructionContext {
    pub is_current_approval_session: bool,
    pub is_prior_approval_gate_session: bool,
    pub node_is_waiting_approval: bool,
}

pub fn validate_approval_chat_instruction(
    context: ApprovalChatInstructionContext,
    content: &str,
) -> Result<(), WorkflowError> {
    if !context.is_current_approval_session {
        if context.is_prior_approval_gate_session {
            return Err(WorkflowError::invalid_state(
                "Workflow is not waiting for approval",
            ));
        }
        return Ok(());
    }
    if !context.node_is_waiting_approval {
        return Err(WorkflowError::invalid_state(
            "Node is not waiting for approval",
        ));
    }
    validate_required_comment_text(content, "approval chat instruction")
        .map_err(|err| WorkflowError::validation(err.to_string()))
}

pub struct ApprovalChatSessionSnapshot<'a> {
    pub is_active: bool,
    pub node_is_waiting_approval: bool,
    pub is_current_approval_session: bool,
    pub current_session_id: Option<&'a str>,
}

pub fn resolve_chat_session_for_approval<'a>(
    snapshot: ApprovalChatSessionSnapshot<'a>,
) -> Result<&'a str, WorkflowError> {
    if !snapshot.is_active {
        return Err(WorkflowError::invalid_state(
            "workflow execution is not active",
        ));
    }
    if !snapshot.node_is_waiting_approval {
        return Err(WorkflowError::invalid_state(
            "Node is not waiting for approval",
        ));
    }
    if !snapshot.is_current_approval_session {
        return Err(WorkflowError::invalid_state(
            "current node is not an approval session",
        ));
    }
    snapshot.current_session_id.ok_or_else(|| {
        WorkflowError::invalid_state("workflow has no current node session for approval chat")
    })
}

#[cfg(test)]
pub struct ApprovalTargetSnapshot<'a> {
    pub execution_id: &'a str,
    pub node_is_waiting_approval: bool,
    pub current_node_name: &'a str,
    pub is_approval_gate_session: bool,
}

#[cfg(test)]
pub fn resolve_approval_target<'a>(
    snapshot: ApprovalTargetSnapshot<'a>,
    expected_execution_id: Option<&str>,
    expected_node_name: Option<&str>,
) -> Result<&'a str, WorkflowError> {
    if !snapshot.node_is_waiting_approval {
        return Err(WorkflowError::invalid_state(
            "Node is not waiting for approval",
        ));
    }
    if !snapshot.is_approval_gate_session {
        return Err(WorkflowError::UnauthorizedApprovalTarget(
            "current node is not an approval-gated session".to_string(),
        ));
    }
    let expected_execution_id = expected_execution_id.ok_or_else(|| {
        WorkflowError::UnauthorizedApprovalTarget("execution_id is required".to_string())
    })?;
    if expected_execution_id != snapshot.execution_id {
        return Err(WorkflowError::UnauthorizedApprovalTarget(
            "execution_id does not match".to_string(),
        ));
    }
    if expected_node_name.is_some_and(|expected| expected != snapshot.current_node_name) {
        return Err(WorkflowError::UnauthorizedApprovalTarget(
            "node does not match".to_string(),
        ));
    }
    Ok(snapshot.current_node_name)
}

#[cfg(test)]
pub fn validate_approval_target(
    snapshot: ApprovalTargetSnapshot<'_>,
    expected_execution_id: Option<&str>,
    expected_node_name: Option<&str>,
) -> Result<(), WorkflowError> {
    resolve_approval_target(snapshot, expected_execution_id, expected_node_name)?;
    if expected_node_name.is_none() {
        return Err(WorkflowError::UnauthorizedApprovalTarget(
            "node_name is required".to_string(),
        ));
    }
    Ok(())
}

pub fn is_approval_gate_session(
    session_id: &str,
    current_session_id: Option<&str>,
    current_node_name: &str,
    approval_gate_session_names: &HashSet<String>,
    node_history: &[NodeHistoryEntry],
) -> bool {
    if current_session_id == Some(session_id)
        && approval_gate_session_names.contains(current_node_name)
    {
        return true;
    }

    node_history.iter().any(|entry| {
        entry.session_id.as_deref() == Some(session_id)
            && approval_gate_session_names.contains(&entry.node_name)
    })
}

fn validate_text_length(value: &str, label: &'static str) -> Result<(), ApprovalInputError> {
    if value.chars().count() > MAX_APPROVAL_COMMENT_CHARS {
        return Err(ApprovalInputError::TooLong {
            label,
            limit: MAX_APPROVAL_COMMENT_CHARS,
        });
    }
    Ok(())
}

#[cfg(test)]
mod approval_rules_tests {
    use super::*;

    #[test]
    fn auto_approve_requires_waiting_approval_and_enabled_flag() {
        assert!(should_auto_approve_workflow_approval(true, true,));
        assert!(!should_auto_approve_workflow_approval(true, false,));
        assert!(!should_auto_approve_workflow_approval(false, true,));
    }

    #[test]
    fn test_optional_comment_空文字は許容し上限超過は拒否する() {
        assert!(validate_optional_comment_text(Some(""), "Approve comment").is_ok());
        let over = "a".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);
        assert!(matches!(
            validate_optional_comment_text(Some(&over), "Approve comment"),
            Err(ApprovalInputError::TooLong { .. })
        ));
    }

    #[test]
    fn approval_chat_instruction_allows_unrelated_sessions_without_validating_content() {
        let context = ApprovalChatInstructionContext {
            is_current_approval_session: false,
            is_prior_approval_gate_session: false,
            node_is_waiting_approval: false,
        };

        assert!(validate_approval_chat_instruction(context, "").is_ok());
    }

    #[test]
    fn approval_chat_instruction_rejects_prior_approval_gate_sessions() {
        let context = ApprovalChatInstructionContext {
            is_current_approval_session: false,
            is_prior_approval_gate_session: true,
            node_is_waiting_approval: false,
        };

        assert!(matches!(
            validate_approval_chat_instruction(context, "retry").unwrap_err(),
            WorkflowError::InvalidState(_)
        ));
    }

    #[test]
    fn approval_chat_instruction_requires_waiting_current_approval_session_and_content() {
        let not_waiting = ApprovalChatInstructionContext {
            is_current_approval_session: true,
            is_prior_approval_gate_session: false,
            node_is_waiting_approval: false,
        };
        assert!(matches!(
            validate_approval_chat_instruction(not_waiting, "ok").unwrap_err(),
            WorkflowError::InvalidState(_)
        ));

        let waiting = ApprovalChatInstructionContext {
            is_current_approval_session: true,
            is_prior_approval_gate_session: false,
            node_is_waiting_approval: true,
        };
        assert!(matches!(
            validate_approval_chat_instruction(waiting, "   ").unwrap_err(),
            WorkflowError::Validation(_)
        ));

        let valid = ApprovalChatInstructionContext {
            is_current_approval_session: true,
            is_prior_approval_gate_session: false,
            node_is_waiting_approval: true,
        };
        assert!(validate_approval_chat_instruction(valid, "please revise").is_ok());
    }

    #[test]
    fn resolve_chat_session_requires_active_waiting_approval_gate_session() {
        let snapshot = ApprovalChatSessionSnapshot {
            is_active: true,
            node_is_waiting_approval: true,
            is_current_approval_session: true,
            current_session_id: Some("session-1"),
        };

        assert_eq!(
            resolve_chat_session_for_approval(snapshot).unwrap(),
            "session-1"
        );

        let inactive = ApprovalChatSessionSnapshot {
            is_active: false,
            node_is_waiting_approval: true,
            is_current_approval_session: true,
            current_session_id: Some("session-1"),
        };
        assert_eq!(
            resolve_chat_session_for_approval(inactive)
                .unwrap_err()
                .to_string(),
            "invalid_state: workflow execution is not active"
        );

        let no_session = ApprovalChatSessionSnapshot {
            is_active: true,
            node_is_waiting_approval: true,
            is_current_approval_session: true,
            current_session_id: None,
        };
        assert_eq!(
            resolve_chat_session_for_approval(no_session)
                .unwrap_err()
                .to_string(),
            "invalid_state: workflow has no current node session for approval chat"
        );
    }

    #[test]
    fn resolve_approval_target_validates_execution_and_node_identity() {
        let snapshot = ApprovalTargetSnapshot {
            execution_id: "execution-1",
            node_is_waiting_approval: true,
            current_node_name: "review",
            is_approval_gate_session: true,
        };

        assert_eq!(
            resolve_approval_target(snapshot, Some("execution-1"), Some("review")).unwrap(),
            "review"
        );

        let snapshot = ApprovalTargetSnapshot {
            execution_id: "execution-1",
            node_is_waiting_approval: true,
            current_node_name: "review",
            is_approval_gate_session: true,
        };
        assert_eq!(
            resolve_approval_target(snapshot, Some("execution-2"), Some("review"))
                .unwrap_err()
                .to_string(),
            "unauthorized_approval_target: execution_id does not match"
        );

        let snapshot = ApprovalTargetSnapshot {
            execution_id: "execution-1",
            node_is_waiting_approval: true,
            current_node_name: "review",
            is_approval_gate_session: true,
        };
        assert_eq!(
            validate_approval_target(snapshot, Some("execution-1"), None)
                .unwrap_err()
                .to_string(),
            "unauthorized_approval_target: node_name is required"
        );

        let snapshot = ApprovalTargetSnapshot {
            execution_id: "execution-1",
            node_is_waiting_approval: true,
            current_node_name: "review",
            is_approval_gate_session: false,
        };
        assert_eq!(
            resolve_approval_target(snapshot, Some("execution-1"), Some("review"))
                .unwrap_err()
                .to_string(),
            "unauthorized_approval_target: current node is not an approval-gated session"
        );
    }

    #[test]
    fn is_approval_gate_session_matches_current_or_history_gated_sessions() {
        let approval_gate_sessions = HashSet::from(["review".to_string()]);
        let history = vec![NodeHistoryEntry {
            node_name: "review".to_string(),
            completed_at: 1.0,
            result: None,
            session_id: Some("old-review".to_string()),
            token_usage: None,
            artifact: None,
            attempt: 1,
            fanout_children: None,
            state: NODE_STATUS_COMPLETED.to_string(),
        }];

        assert!(is_approval_gate_session(
            "current-review",
            Some("current-review"),
            "review",
            &approval_gate_sessions,
            &history,
        ));
        assert!(is_approval_gate_session(
            "old-review",
            None,
            "plan",
            &approval_gate_sessions,
            &history,
        ));
        assert!(!is_approval_gate_session(
            "agent-session",
            Some("agent-session"),
            "plan",
            &approval_gate_sessions,
            &history,
        ));
    }
}
