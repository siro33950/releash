#![cfg(debug_assertions)]

#[test]
fn test_サーバ受け入れテスト_tauriとシェルハーネスを参照しない() {
    // Given
    let sources = [
        include_str!("../src/acceptance_test_support.rs"),
        include_str!("../src/agent_session_tui_acceptance.rs"),
        include_str!("../src/client_api_acceptance.rs"),
        include_str!("../src/provider_lifecycle_acceptance.rs"),
        include_str!("../src/terminal_subscription_acceptance.rs"),
        include_str!("../src/workflow_control_plane_acceptance.rs"),
        include_str!("../src/workflow_delegate_acceptance.rs"),
        include_str!("../src/workflow_diagnostics_acceptance.rs"),
        include_str!("../src/adaptor/controller/api/client_test.rs"),
        include_str!("../src/adaptor/controller/client/workflow/mod.rs"),
        include_str!("../src/adaptor/controller/client/app_config/shared_test.rs"),
        include_str!("../src/adaptor/controller/client/workspace_tree_shared_test.rs"),
        include_str!("agent_session_tui_acceptance.rs"),
        include_str!("client_api/mod.rs"),
        include_str!("state_subscription/flow_test.rs"),
        include_str!("workflow_control_plane_acceptance_test.rs"),
    ];
    // When / Then
    for source in sources {
        assert!(!source.contains("tauri::"));
        assert!(!source.contains("use tauri"));
        assert!(!source.contains("desktop_test_support"));
        assert!(!source.contains("gateway::desktop_client"));
    }
}
