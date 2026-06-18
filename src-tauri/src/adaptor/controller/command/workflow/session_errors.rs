pub const WORKFLOW_STEP_TAB_OPERATION_FAILED_CODE: &str = "workflow_step_tab_operation_failed";

pub fn redacted_workflow_tab_error(code: &str) -> String {
    format!("{code}: workflow step tab operation failed")
}

pub fn workflow_step_tab_operation_failed() -> String {
    redacted_workflow_tab_error(WORKFLOW_STEP_TAB_OPERATION_FAILED_CODE)
}

#[cfg(test)]
mod tests {
    use super::{
        redacted_workflow_tab_error, workflow_step_tab_operation_failed,
        WORKFLOW_STEP_TAB_OPERATION_FAILED_CODE,
    };

    #[test]
    fn workflow_step_tab_error_is_redacted() {
        let err = workflow_step_tab_operation_failed();

        assert_eq!(
            err,
            redacted_workflow_tab_error(WORKFLOW_STEP_TAB_OPERATION_FAILED_CODE)
        );
        assert_eq!(
            err,
            "workflow_step_tab_operation_failed: workflow step tab operation failed"
        );
        assert!(!err.contains("/repo"));
        assert!(!err.contains("agent-session-secret"));
        assert!(!err.contains("message body"));
    }

    #[test]
    fn redacted_workflow_tab_error_includes_code_without_sensitive_values() {
        let err = redacted_workflow_tab_error("workflow_step_session_rejected");

        assert_eq!(
            err,
            "workflow_step_session_rejected: workflow step tab operation failed"
        );
        assert!(!err.contains("/repo"));
        assert!(!err.contains("agent-session-secret"));
        assert!(!err.contains("message body"));
        assert!(!err.contains("worktree"));
    }
}
