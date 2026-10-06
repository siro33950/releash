use crate::adaptor::controller::command as commands;
use releash_lib::desktop_api::test_support as wire;
use std::sync::Arc;
use tauri::Manager;
#[test]
fn test_クライアントdispatch_proto全commandの登録と引数検証() {
    for command in commands::tests::registered_command_names() {
        if command != "set_menu_items_enabled"
            && !commands::client::COMMAND_NAMES.contains(&command)
            && !commands::desktop_lifecycle::COMMAND_NAMES.contains(&command)
        {
            assert!(wire::COMMAND_NAMES.contains(&command), "{command}");
        }
    }
    assert!(!commands::tests::registered_command_names().contains(&"get_terminal_stream_endpoint"));
    for command in [
        "menu",
        "set_menu_items_enabled",
        "get_terminal_stream_endpoint",
        "start_watching",
        "start_git_dir_watching",
        "stop_watching",
    ]
    .into_iter()
    .chain(commands::desktop_lifecycle::COMMAND_NAMES.iter().copied())
    {
        assert!(!wire::COMMAND_NAMES.contains(&command), "{command}");
    }
}

#[tokio::test(start_paused = true)]
async fn test_接続情報command_起動中の要求を拒否せず検証済みendpointを返す() {
    use crate::usecase::test_helpers::{tick, FakeDaemon};
    use std::sync::atomic::Ordering;
    // Given
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    app.manage(supervisor);
    for reconnect in [false, true] {
        if reconnect {
            gateway.ready.store(false, Ordering::SeqCst);
            tick(200).await;
        }
        // When
        let request = super::get_client_endpoint(app.state());
        tokio::pin!(request);
        assert!(futures_util::poll!(&mut request).is_pending());
        tick(200).await;
        assert!(futures_util::poll!(&mut request).is_pending());
        gateway.ready.store(true, Ordering::SeqCst);
        tick(200).await;
        // Then
        let endpoint = request.await.unwrap();
        assert_eq!(endpoint.endpoint.token, "client-only");
        assert_eq!(endpoint.launch_id, "launch");
    }
}

#[test]
fn test_通常要求_connect入口にはsupervisorの受付制御を登録しない() {
    // Given / When
    let commands = super::COMMAND_NAMES;
    // Then
    assert!(commands.contains(&"get_client_endpoint"));
    for removed in [
        "get_workspaces",
        "get_workspace_tree_selection_reconciliation",
        "get_workspace_node_detail",
        "get_agent_session",
        "get_workspace_session_node_id",
        "list_agent_session_history",
        "list_available_agent_session_providers",
        "list_branches",
        "get_branch_base",
        "list_branches_with_status_snapshot",
        "get_current_branch",
        "get_cached_issues",
        "list_worktrees",
        "get_main_repo_path",
        "get_cwd",
        "load_workspace_state",
        "get_cached_pr_status",
        "list_workspace_worktree_nodes",
        "list_workspace_workflow_history",
        "get_workflow_execution_state",
        "resolve_active_execution_by_worktree",
        "get_app_settings",
        "get_notion_config",
        "get_provider_availability",
        "get_external_editor",
        "detect_editors",
        "get_releash_base",
        "get_workflow_config",
        "get_performance_telemetry_enabled",
        "get_performance_real_app_mode",
        "get_terminal_performance_switches",
        "list_provider_hook_health_warnings",
        "get_application_startup_outcome",
        "fetch_pr_status",
        "admit_client_command",
        "attach_desktop_client",
        "send_desktop_client_frame",
        "detach_desktop_client",
    ] {
        assert!(!commands.contains(&removed));
    }
}
