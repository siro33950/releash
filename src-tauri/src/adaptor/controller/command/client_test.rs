use super::*;

#[tokio::test]
async fn test_クライアントdispatch_startup失敗時はusecase実行前に拒否する() {
    // Given
    let dispatch = ClientCommandDispatch::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(ApplicationStartupAuthority::failed_kind(
            crate::usecase::application_startup::StartupFailureKind::StoreValidationFailed,
        )),
    );
    // When
    let error = dispatch
        .dispatch(
            "get_current_branch",
            serde_json::json!({"repoPath":"/missing"}),
        )
        .await
        .unwrap_err();
    // Then
    assert_eq!(
        serde_json::to_value(error).unwrap()["code"],
        "APPLICATION_UNAVAILABLE"
    );
}

#[test]
fn test_クライアント接続情報_terminalと同じ非master_tokenを返す() {
    // Given
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    assert!(client_endpoint(app.handle()).is_none());
    app.manage(crate::adaptor::controller::state::TerminalStreamEndpoint {
        port: 12345,
        token: Arc::from("client-only"),
    });
    // When
    let endpoint = client_endpoint(app.handle()).unwrap();
    // Then
    assert_eq!(endpoint.url, "ws://127.0.0.1:12345/v1/client");
    assert_eq!(endpoint.auth_subprotocol, "releash-bearer.client-only");
    assert_eq!(
        serde_json::to_value(endpoint).unwrap(),
        serde_json::json!({
            "url": "ws://127.0.0.1:12345/v1/client",
            "authSubprotocol": "releash-bearer.client-only"
        })
    );
}
