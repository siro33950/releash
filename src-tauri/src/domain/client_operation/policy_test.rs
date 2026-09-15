use super::*;

#[test]
fn test_通信方針_副作用と操作別の期限を分類する() {
    // Given / When / Then
    assert_eq!(recovery("get_current_branch"), Recovery::Read);
    assert_eq!(recovery("save_workspace_state"), Recovery::Idempotent);
    assert_eq!(recovery("create_agent_session"), Recovery::CallerAttempt);
    assert_eq!(recovery("start_workflow"), Recovery::AtMostOnce);
    assert_eq!(
        recovery("get_or_spawn_terminal_surface"),
        Recovery::AtMostOnce
    );
    assert_eq!(recovery("unknown_command"), Recovery::AtMostOnce);
    assert_eq!(deadline_ms("get_current_branch"), 10_000);
    assert_eq!(deadline_ms("create_worktree"), 120_000);
    assert_eq!(deadline_ms("write_terminal_surface"), 3_000);
    assert_eq!(deadline_ms("append_review_comment"), 30_000);
    assert!(replay_after_restart("request_application_quit"));
    for command in [
        "create_agent_session",
        "restore_agent_session",
        "open_agent_session",
        "start_workflow",
        "stop_watching",
        "save_workspace_state",
        "add_repo_path",
    ] {
        assert!(!replay_after_restart(command));
    }
    assert!(HEARTBEAT_TIMEOUT_MS < HEARTBEAT_INTERVAL_MS);
}

#[test]
fn test_通信方針_更新順序の対象をドメインの種類で識別する() {
    // Given / When / Then
    assert_eq!(
        ordering_scope("add_repo_path"),
        Some(OrderingScope::RepositoryMembership)
    );
    assert_eq!(
        ordering_scope("remove_repo_path"),
        ordering_scope("add_repo_path")
    );
    assert_eq!(
        ordering_scope("save_workspace_state"),
        Some(OrderingScope::WorkspaceState)
    );
    assert_eq!(
        ordering_scope("save_notion_config"),
        ordering_scope("delete_notion_config")
    );
    assert_eq!(ordering_scope("unknown_command"), None);
    assert!(!may_replay("start_watching", false, true));
    assert!(!may_replay("create_agent_session", true, true));
    assert!(!may_replay("request_application_quit", false, true));
    assert!(
        OPERATION_RETENTION_MS
            > LONG_COMMANDS
                .iter()
                .map(|command| deadline_ms(command))
                .max()
                .unwrap()
    );
    assert!(!may_replay("get_repo_paths", true, true));
    assert!(!may_replay("start_workflow", true, false));
}
