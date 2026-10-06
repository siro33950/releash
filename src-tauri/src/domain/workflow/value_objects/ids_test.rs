mod ids_tests {
    use super::super::*;

    #[test]
    fn test_実行木id_workflowと単独sessionの既存形式だけを受理する() {
        assert!(ExecutionTreeId::new("00000000-0000-4000-8000-000000000001").is_ok());
        let session_id =
            crate::domain::agent_session::launch_resource_id("agent-session", "archive-test")
                .unwrap();
        assert!(ExecutionTreeId::new(session_id.clone()).is_ok());
        assert!(WorkflowExecutionId::new(session_id).is_err());
        for invalid in [
            "agent-session-",
            "agent-session-123",
            "agent-session-0000000000000000000000000000000/",
            "agent-session-0000000000000000000000000000000g",
            "provider-slot-00000000000000000000000000000000",
        ] {
            assert!(ExecutionTreeId::new(invalid).is_err());
        }
        assert!(ExecutionTreeId::new("../bad").is_err());
        assert!(ExecutionTreeId::new("not-a-uuid").is_err());
    }

    #[test]
    fn test_workflow_definition_name_path要素を拒否する() {
        assert!(WorkflowDefinitionName::new("review").is_ok());
        assert!(WorkflowDefinitionName::new("../review").is_err());
        assert!(WorkflowDefinitionName::new("foo/bar").is_err());
        assert!(WorkflowDefinitionName::new("bad name!").is_err());
        assert!(WorkflowDefinitionName::new("_bad").is_err());
    }

    #[test]
    fn test_workspace_worktree_path末尾スラッシュを正規化する() {
        let path = WorkspaceWorktreePath::new("/tmp/repo/").unwrap();
        assert_eq!(path.to_string(), "/tmp/repo");
    }
}
