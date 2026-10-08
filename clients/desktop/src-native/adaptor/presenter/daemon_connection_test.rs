use super::*;
#[test]
fn test_接続失敗表示_初回設定を受け取れない理由と古い側を画面へ渡す() {
    // Given / When / Then
    let settings = failure(DaemonConnectionFailure::InitialSettingsUnavailable {
        detail: Some("settings denied".into()),
    });
    assert!(settings
        .message
        .contains("サーバの初回設定を受信できませんでした"));
    assert!(settings.message.contains("settings denied"));
    let timeout = failure(DaemonConnectionFailure::InitialSettingsUnavailable { detail: None });
    assert_eq!(timeout.message, "サーバの初回設定を受信できませんでした");
    for server_older in [true, false] {
        let value = failure(DaemonConnectionFailure::Incompatible {
            server_older,
            server_release: "server".into(),
            client_release: "desktop".into(),
        });
        assert_eq!(value.server_older, server_older);
        assert_eq!(value.client_older, !server_older);
        assert!(value.message.contains(if server_older {
            "サーバが古い"
        } else {
            "画面が古い"
        }));
        assert!(value.message.contains("server"));
        assert!(value.message.contains("desktop"));
    }
}

#[test]
fn test_接続先の返却_サーバの接続先を転送型へ変換する() {
    // Given
    let connection = DaemonEndpoint {
        url: "http://127.0.0.1:1234".into(),
        token: "client-token".into(),
    };
    // When
    let value = endpoint(connection);
    // Then
    assert_eq!(value.url, "http://127.0.0.1:1234");
    assert_eq!(value.token, "client-token");
    assert_eq!(
        serde_json::to_value(value).unwrap(),
        serde_json::json!({"url": "http://127.0.0.1:1234", "token": "client-token"})
    );
}
