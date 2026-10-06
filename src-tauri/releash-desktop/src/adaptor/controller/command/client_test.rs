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
    for command in [
        "menu",
        "set_menu_items_enabled",
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
