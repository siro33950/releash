//! Pure approval rules and input validation.

use std::collections::HashSet;

use serde_json::json;

use crate::domain::workflow::services::secret_masker;
use crate::domain::workflow::value_objects::{
    ApprovalDecision, StepHistoryEntry, TransitionRule, WorkflowExecutionState,
};
use crate::domain::workflow::WorkflowError;
#[cfg(test)]
use crate::domain::workflow::STEP_STATE_COMPLETED;

pub const MAX_APPROVAL_COMMENT_CHARS: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalInputError {
    Empty { label: &'static str },
    TooLong { label: &'static str, limit: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalRuleError {
    UnsupportedMatch { reason: &'static str },
    TooManyRejectRules { reason: &'static str },
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

impl std::fmt::Display for ApprovalRuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedMatch { reason } | Self::TooManyRejectRules { reason } => {
                f.write_str(reason)
            }
        }
    }
}

impl std::error::Error for ApprovalRuleError {}

pub fn validate_optional_comment_text(
    value: Option<&str>,
    label: &'static str,
) -> Result<(), ApprovalInputError> {
    if let Some(text) = value {
        validate_text_length(text, label)?;
    }
    Ok(())
}

pub fn validate_reject_reason_text(
    value: &str,
    label: &'static str,
) -> Result<(), ApprovalInputError> {
    validate_required_comment_text(value, label)
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

pub fn can_reject(rules: &[TransitionRule]) -> bool {
    rules.iter().any(|rule| rule.r#match == "reject")
}

pub fn validate_approval_rules(rules: &[TransitionRule]) -> Result<(), ApprovalRuleError> {
    let reject_count = rules.iter().filter(|rule| rule.r#match == "reject").count();
    if rules.iter().any(|rule| rule.r#match != "reject") {
        return Err(ApprovalRuleError::UnsupportedMatch {
            reason: "match: reject 以外のruleは定義できません",
        });
    }
    if reject_count > 1 {
        return Err(ApprovalRuleError::TooManyRejectRules {
            reason: "match: reject ruleは最大1件です",
        });
    }
    Ok(())
}

pub fn validate_approval_decision(decision: &ApprovalDecision) -> Result<(), ApprovalInputError> {
    match decision {
        ApprovalDecision::Approve { comment } => {
            validate_optional_comment_text(comment.as_deref(), "Approve comment")
        }
        ApprovalDecision::Reject { reason } => {
            validate_reject_reason_text(reason, "Reject comment")
        }
        ApprovalDecision::Abort => Ok(()),
    }
}

pub fn should_auto_approve_workflow_approval(
    state: &WorkflowExecutionState,
    approval_auto_approve_enabled: bool,
) -> bool {
    approval_auto_approve_enabled && matches!(state, WorkflowExecutionState::WaitingApproval)
}

pub fn reject_structured_output(comment: &str, configured_secrets: &[String]) -> serde_json::Value {
    let comment = secret_masker::mask_sensitive_text(comment, configured_secrets);
    json!({
        "decision": "reject",
        "comment": comment,
    })
}

pub struct ApprovalChatInstructionContext<'a> {
    pub is_current_approval_session: bool,
    pub is_prior_approval_step_session: bool,
    pub state: &'a WorkflowExecutionState,
}

pub fn validate_approval_chat_instruction(
    context: ApprovalChatInstructionContext<'_>,
    content: &str,
) -> Result<(), WorkflowError> {
    if !context.is_current_approval_session {
        if context.is_prior_approval_step_session {
            return Err(WorkflowError::invalid_state(
                "Workflow is not waiting for approval",
            ));
        }
        return Ok(());
    }
    if !matches!(context.state, WorkflowExecutionState::WaitingApproval) {
        return Err(WorkflowError::invalid_state(
            "Workflow is not waiting for approval",
        ));
    }
    validate_required_comment_text(content, "approval chat instruction")
        .map_err(|err| WorkflowError::validation(err.to_string()))
}

pub struct ApprovalChatSessionSnapshot<'a> {
    pub is_active: bool,
    pub state: &'a WorkflowExecutionState,
    pub is_current_approval_session: bool,
    pub current_session_id: Option<&'a str>,
}

pub fn resolve_chat_session_for_approval<'a>(
    snapshot: ApprovalChatSessionSnapshot<'a>,
) -> Result<&'a str, WorkflowError> {
    if !snapshot.is_active {
        return Err(WorkflowError::invalid_state("workflow run is not active"));
    }
    if !matches!(snapshot.state, WorkflowExecutionState::WaitingApproval) {
        return Err(WorkflowError::invalid_state(
            "Workflow is not waiting for approval",
        ));
    }
    if !snapshot.is_current_approval_session {
        return Err(WorkflowError::invalid_state(
            "current node is not an approval session",
        ));
    }
    snapshot.current_session_id.ok_or_else(|| {
        WorkflowError::invalid_state("workflow has no current step session for approval chat")
    })
}

pub struct ApprovalTargetSnapshot<'a> {
    pub execution_id: &'a str,
    pub state: &'a WorkflowExecutionState,
    pub current_step_name: &'a str,
}

pub fn resolve_approval_target<'a>(
    snapshot: ApprovalTargetSnapshot<'a>,
    expected_execution_id: Option<&str>,
    expected_step_name: Option<&str>,
) -> Result<&'a str, WorkflowError> {
    if !matches!(snapshot.state, WorkflowExecutionState::WaitingApproval) {
        return Err(WorkflowError::invalid_state(
            "Workflow is not waiting for approval",
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
    if expected_step_name.is_some_and(|expected| expected != snapshot.current_step_name) {
        return Err(WorkflowError::UnauthorizedApprovalTarget(
            "step does not match".to_string(),
        ));
    }
    Ok(snapshot.current_step_name)
}

#[cfg(test)]
pub fn validate_approval_target(
    snapshot: ApprovalTargetSnapshot<'_>,
    expected_execution_id: Option<&str>,
    expected_step_name: Option<&str>,
) -> Result<(), WorkflowError> {
    resolve_approval_target(snapshot, expected_execution_id, expected_step_name)?;
    if expected_step_name.is_none() {
        return Err(WorkflowError::UnauthorizedApprovalTarget(
            "step_name is required".to_string(),
        ));
    }
    Ok(())
}

pub fn is_approval_step_session(
    session_id: &str,
    current_session_id: Option<&str>,
    current_step_name: &str,
    approval_step_names: &HashSet<String>,
    step_history: &[StepHistoryEntry],
) -> bool {
    if current_session_id == Some(session_id) && approval_step_names.contains(current_step_name) {
        return true;
    }

    step_history.iter().any(|entry| {
        entry.session_id.as_deref() == Some(session_id)
            && approval_step_names.contains(&entry.step_name)
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
    fn test_reject_reason_空白のみを拒否する() {
        let err = validate_reject_reason_text("  \n", "Reject comment").unwrap_err();
        assert_eq!(
            err,
            ApprovalInputError::Empty {
                label: "Reject comment"
            }
        );
        assert_eq!(err.to_string(), "Reject comment must not be empty");
    }

    #[test]
    fn auto_approve_requires_waiting_approval_and_enabled_flag() {
        assert!(should_auto_approve_workflow_approval(
            &WorkflowExecutionState::WaitingApproval,
            true,
        ));
        assert!(!should_auto_approve_workflow_approval(
            &WorkflowExecutionState::WaitingApproval,
            false,
        ));
        assert!(!should_auto_approve_workflow_approval(
            &WorkflowExecutionState::Running,
            true,
        ));
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
            is_prior_approval_step_session: false,
            state: &WorkflowExecutionState::Running,
        };

        assert!(validate_approval_chat_instruction(context, "").is_ok());
    }

    #[test]
    fn approval_chat_instruction_rejects_prior_approval_step_sessions() {
        let context = ApprovalChatInstructionContext {
            is_current_approval_session: false,
            is_prior_approval_step_session: true,
            state: &WorkflowExecutionState::Completed,
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
            is_prior_approval_step_session: false,
            state: &WorkflowExecutionState::Running,
        };
        assert!(matches!(
            validate_approval_chat_instruction(not_waiting, "ok").unwrap_err(),
            WorkflowError::InvalidState(_)
        ));

        let waiting = ApprovalChatInstructionContext {
            is_current_approval_session: true,
            is_prior_approval_step_session: false,
            state: &WorkflowExecutionState::WaitingApproval,
        };
        assert!(matches!(
            validate_approval_chat_instruction(waiting, "   ").unwrap_err(),
            WorkflowError::Validation(_)
        ));

        let valid = ApprovalChatInstructionContext {
            is_current_approval_session: true,
            is_prior_approval_step_session: false,
            state: &WorkflowExecutionState::WaitingApproval,
        };
        assert!(validate_approval_chat_instruction(valid, "please revise").is_ok());
    }

    #[test]
    fn test_can_reject_reject_ruleの有無を判定する() {
        assert!(!can_reject(&[]));
        assert!(can_reject(&[TransitionRule {
            r#match: "reject".to_string(),
            next: "fix".to_string(),
        }]));
    }

    #[test]
    fn validate_approval_rules_allows_only_one_reject_rule() {
        assert!(validate_approval_rules(&[]).is_ok());
        assert!(validate_approval_rules(&[TransitionRule {
            r#match: "reject".to_string(),
            next: "fix".to_string(),
        }])
        .is_ok());

        assert_eq!(
            validate_approval_rules(&[TransitionRule {
                r#match: "approve".to_string(),
                next: "done".to_string(),
            }])
            .unwrap_err()
            .to_string(),
            "match: reject 以外のruleは定義できません"
        );

        assert_eq!(
            validate_approval_rules(&[
                TransitionRule {
                    r#match: "reject".to_string(),
                    next: "fix".to_string(),
                },
                TransitionRule {
                    r#match: "reject".to_string(),
                    next: "retry".to_string(),
                },
            ])
            .unwrap_err()
            .to_string(),
            "match: reject ruleは最大1件です"
        );
    }

    #[test]
    fn reject_structured_output_redacts_sensitive_comment() {
        let structured = reject_structured_output(
            "Reject because password=secret123 and ghp_abcdefghijklmnopqrstuvwxyz1234567890",
            &[],
        );
        let comment = structured["comment"].as_str().unwrap();
        assert_eq!(structured["decision"].as_str(), Some("reject"));
        assert!(!comment.contains("secret123"));
        assert!(!comment.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(comment.contains("[REDACTED]"));
    }

    #[test]
    fn resolve_chat_session_requires_active_waiting_approval_node_with_session() {
        let snapshot = ApprovalChatSessionSnapshot {
            is_active: true,
            state: &WorkflowExecutionState::WaitingApproval,
            is_current_approval_session: true,
            current_session_id: Some("session-1"),
        };

        assert_eq!(
            resolve_chat_session_for_approval(snapshot).unwrap(),
            "session-1"
        );

        let inactive = ApprovalChatSessionSnapshot {
            is_active: false,
            state: &WorkflowExecutionState::WaitingApproval,
            is_current_approval_session: true,
            current_session_id: Some("session-1"),
        };
        assert_eq!(
            resolve_chat_session_for_approval(inactive)
                .unwrap_err()
                .to_string(),
            "invalid_state: workflow run is not active"
        );

        let no_session = ApprovalChatSessionSnapshot {
            is_active: true,
            state: &WorkflowExecutionState::WaitingApproval,
            is_current_approval_session: true,
            current_session_id: None,
        };
        assert_eq!(
            resolve_chat_session_for_approval(no_session)
                .unwrap_err()
                .to_string(),
            "invalid_state: workflow has no current step session for approval chat"
        );
    }

    #[test]
    fn resolve_approval_target_validates_run_and_step_identity() {
        let waiting = WorkflowExecutionState::WaitingApproval;
        let snapshot = ApprovalTargetSnapshot {
            execution_id: "run-1",
            state: &waiting,
            current_step_name: "review",
        };

        assert_eq!(
            resolve_approval_target(snapshot, Some("run-1"), Some("review")).unwrap(),
            "review"
        );

        let snapshot = ApprovalTargetSnapshot {
            execution_id: "run-1",
            state: &waiting,
            current_step_name: "review",
        };
        assert_eq!(
            resolve_approval_target(snapshot, Some("run-2"), Some("review"))
                .unwrap_err()
                .to_string(),
            "unauthorized_approval_target: execution_id does not match"
        );

        let snapshot = ApprovalTargetSnapshot {
            execution_id: "run-1",
            state: &waiting,
            current_step_name: "review",
        };
        assert_eq!(
            validate_approval_target(snapshot, Some("run-1"), None)
                .unwrap_err()
                .to_string(),
            "unauthorized_approval_target: step_name is required"
        );
    }

    #[test]
    fn is_approval_step_session_matches_current_or_history_approval_steps() {
        let approval_steps = HashSet::from(["review".to_string()]);
        let history = vec![StepHistoryEntry {
            step_name: "review".to_string(),
            completed_at: 1.0,
            result: None,
            session_id: Some("old-review".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: STEP_STATE_COMPLETED.to_string(),
        }];

        assert!(is_approval_step_session(
            "current-review",
            Some("current-review"),
            "review",
            &approval_steps,
            &history,
        ));
        assert!(is_approval_step_session(
            "old-review",
            None,
            "plan",
            &approval_steps,
            &history,
        ));
        assert!(!is_approval_step_session(
            "agent-session",
            Some("agent-session"),
            "plan",
            &approval_steps,
            &history,
        ));
    }
}
