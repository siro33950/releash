pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn runtime_execution_state_active_is_derived() {
        assert!(RuntimeExecutionState::Running.is_active());
        assert!(!RuntimeExecutionState::Aborted.is_active());
        assert!(!RuntimeExecutionState::Completed.is_active());
    }
}
