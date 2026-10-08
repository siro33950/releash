pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn test_workflow状態_実行中と完了とabortの語彙と操作可否を保つ() {
        // Given / When / Then
        for (status, name, active) in [
            (ExecutionStatus::Running, "running", true),
            (ExecutionStatus::Completed, "completed", false),
            (ExecutionStatus::Aborted, "aborted", false),
        ] {
            assert_eq!(status.as_str(), name);
            assert_eq!(status.is_active(), active);
            assert_eq!(status.is_finished(), !active);
            assert_eq!(status.is_finished(), !active);
            assert_eq!(status.can_abort(), active);
        }
    }

    #[test]
    fn execution_origin_owns_the_public_vocabulary_and_rejects_unknown_values() {
        for (value, expected) in [
            ("desktop_ui", ExecutionOrigin::DesktopUi),
            ("desktop-ui", ExecutionOrigin::DesktopUi),
            ("cli", ExecutionOrigin::Cli),
            ("agent", ExecutionOrigin::Agent),
            ("api", ExecutionOrigin::Api),
        ] {
            assert_eq!(ExecutionOrigin::from_public_value(value).unwrap(), expected);
            assert_eq!(
                ExecutionOrigin::from_public_value(expected.as_public_value()).unwrap(),
                expected
            );
        }
        assert!(ExecutionOrigin::from_public_value("remote").is_err());
    }

    #[test]
    fn retryable_node_ids_only_include_the_latest_current_attempt() {
        let node = |id: &str, node_name: &str, attempt: u32| NodeExecution {
            worktree: None,
            id: id.to_string(),
            execution_id: "execution-1".to_string(),
            node_name: node_name.to_string(),
            kind: crate::domain::workflow::NodeKindName::Command,
            attempt,
            status: crate::domain::workflow::NodeExecutionStatus::Running,
            session_id: None,
            display_command: None,
            result_summary: None,
            artifact: None,
            token_usage: None,
            process_presence: crate::domain::workflow::NodeProcessPresence::ConfirmedAbsent,
            parent: None,
            completion_signals: crate::domain::workflow::NodeCompletionSignalState::Pending,
            started_at: 1.0,
            completed_at: None,
        };
        let execution = ExecutionTree {
            id: "execution-1".to_string(),
            workflow_name: "review".to_string(),
            status: ExecutionStatus::Running,
            current_node: Some("review".to_string()),
            created_from: ExecutionOrigin::Cli,
            worktree_path: "/repo".to_string(),
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            total_token_usage: TokenUsage::default(),
            node_executions: vec![
                node("review-1", "review", 1),
                node("review-2", "review", 2),
                node("other-1", "other", 1),
            ],
            artifacts: Vec::new(),
            fanouts: Vec::new(),
            approval_target: None,
        };

        assert_eq!(
            execution.retryable_node_execution_ids(),
            std::collections::HashSet::from(["review-2".to_string(), "other-1".to_string()])
        );
    }
}
