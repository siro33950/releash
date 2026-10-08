pub const WORKFLOW_NODE_TAB_OPERATION_FAILED_CODE: &str = "workflow_node_tab_operation_failed";

pub fn redacted_workflow_tab_error(code: &str) -> String {
    format!("{code}: workflow node tab operation failed")
}

pub fn workflow_node_tab_operation_failed() -> String {
    redacted_workflow_tab_error(WORKFLOW_NODE_TAB_OPERATION_FAILED_CODE)
}

#[cfg(test)]
#[path = "session_errors_test.rs"]
mod session_errors_tests;
