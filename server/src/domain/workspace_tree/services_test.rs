pub(crate) mod tests {
    use super::super::*;
    use crate::domain::workflow::{AgentSessionActivity, NodeCompletionSignalState};
    use crate::domain::workspace_tree::{WorkspaceNodeStatus, WorkspaceNodeStatusClassification};

    fn node(
        id: &str,
        parent_id: Option<&str>,
        sibling_order: u64,
        kind: WorkspaceNodeKind,
        execution_id: Option<&str>,
    ) -> WorkspaceTreeNode {
        WorkspaceTreeNode {
            process_presence: Default::default(),
            can_resume_session: false,
            worktree: None,
            id: id.to_string(),
            parent_id: parent_id.map(str::to_string),
            sibling_order,
            kind,
            title: id.to_string(),
            status: WorkspaceNodeStatus::Running,
            status_classification: WorkspaceNodeStatusClassification::Active,
            delegate_waits_for_child: false,
            background_failure: false,
            activity: (kind == WorkspaceNodeKind::WorkflowSession)
                .then(AgentSessionActivity::default),
            error_reason: None,
            updated_at_bits: 0.0_f64.to_bits(),
            execution_id: execution_id.map(str::to_string),
            node_execution_id: None,
            node_name: None,
            attempt: None,
            execution_parent: None,
            retry_predecessor_id: None,
            past_attempt_ids: Vec::new(),
            is_retry_history: false,
            completion_signals: NodeCompletionSignalState::Pending,
            has_artifact: false,
            session_id: None,
            can_rename: false,
            can_approve: false,
            can_retry: false,
            can_abort: false,
            can_archive: false,
            display_command: None,
            command_result: None,
            dynamic_fanout: false,
        }
    }

    #[test]
    fn public_root_is_the_first_visible_direct_child_of_each_workflow_owner() {
        let mut retry = node(
            "retry-history",
            Some("owner-a"),
            0,
            WorkspaceNodeKind::WorkflowSession,
            Some("execution-a"),
        );
        retry.is_retry_history = true;
        let nodes = vec![
            node(
                "owner-a",
                None,
                0,
                WorkspaceNodeKind::Workflow,
                Some("execution-a"),
            ),
            retry,
            node(
                "second",
                Some("owner-a"),
                2,
                WorkspaceNodeKind::WorkflowCommand,
                Some("execution-a"),
            ),
            node(
                "first",
                Some("owner-a"),
                1,
                WorkspaceNodeKind::Sequence,
                Some("execution-a"),
            ),
            node(
                "owner-b",
                None,
                1,
                WorkspaceNodeKind::Workflow,
                Some("execution-b"),
            ),
            node(
                "session-root",
                Some("owner-b"),
                0,
                WorkspaceNodeKind::WorkflowSession,
                Some("execution-b"),
            ),
        ];

        let roots = WorkspacePublicRoot::all(&nodes);

        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].owner().id, "owner-a");
        assert_eq!(roots[0].node().id, "first");
        assert_eq!(roots[0].public_id(), "execution-a");
        assert_eq!(roots[1].node().id, "session-root");
        assert_eq!(roots[1].public_id(), "execution-b");
    }

    #[test]
    fn public_root_lookups_share_the_same_resolved_relationship() {
        let nodes = vec![
            node(
                "owner",
                None,
                0,
                WorkspaceNodeKind::Workflow,
                Some("execution"),
            ),
            node(
                "root",
                Some("owner"),
                0,
                WorkspaceNodeKind::Fanout,
                Some("execution"),
            ),
            node(
                "child",
                Some("owner"),
                1,
                WorkspaceNodeKind::WorkflowCommand,
                Some("execution"),
            ),
        ];

        let by_execution = WorkspacePublicRoot::for_execution(&nodes, "execution").unwrap();
        let by_node = WorkspacePublicRoot::for_node(&nodes, "root").unwrap();

        assert_eq!(by_execution, by_node);
        assert!(WorkspacePublicRoot::for_node(&nodes, "child").is_none());
    }

    #[test]
    fn test_public_root表示名_public_root_nodeではなくownerのtitleを返す() {
        // Given
        let mut owner = node(
            "owner",
            None,
            0,
            WorkspaceNodeKind::Workflow,
            Some("execution"),
        );
        owner.title = "01_author-spec".to_string();
        let mut public_root = node(
            "root",
            Some("owner"),
            0,
            WorkspaceNodeKind::Sequence,
            Some("execution"),
        );
        public_root.title = "main".to_string();
        let nodes = vec![owner, public_root];

        // When
        let public_root = WorkspacePublicRoot::for_execution(&nodes, "execution").unwrap();

        // Then
        assert_eq!(public_root.public_title(), "01_author-spec");
    }

    #[test]
    fn test_public_root表示名_単独session実行木はnode自身のtitleを返す() {
        let mut owner = node(
            "execution",
            None,
            0,
            WorkspaceNodeKind::Workflow,
            Some("execution"),
        );
        owner.title = "session".to_string();
        let mut session = node(
            "root",
            Some("execution"),
            0,
            WorkspaceNodeKind::WorkflowSession,
            Some("execution"),
        );
        session.node_execution_id = Some("execution".to_string());
        session.node_name = Some("session".to_string());
        session.attempt = Some(1);
        session.title = "Provider title".to_string();
        let nodes = vec![owner, session];

        let public_root = WorkspacePublicRoot::for_execution(&nodes, "execution").unwrap();

        assert_eq!(public_root.public_title(), "Provider title");
    }
}
