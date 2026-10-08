use super::*;

#[test]
fn test_画面判定_実行ファイルの祖先にあるappを選ぶ() {
    // Given / When / Then
    assert_eq!(
        app_bundle(Path::new(
            "/Applications/Releash.app/Contents/MacOS/releash"
        )),
        Some(Path::new("/Applications/Releash.app"))
    );
    assert_eq!(app_bundle(Path::new("/tmp/releash")), None);
    assert_eq!(app_bundle(Path::new("/tmp/releash.app")), None);
    assert_eq!(app_bundle(Path::new("/tmp/not.app-dir/releash")), None);
}

#[tokio::test]
async fn test_状態表示_未起動でも成功しtokenを出さない() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    // When
    let output = status(dir.path(), true).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    // Then
    assert_eq!(value["running"], false);
    assert_eq!(value["client"]["release"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["client"]["protocol"], descriptor::protocol());
    assert_eq!(value["data_dir"], dir.path().to_str().unwrap());
    assert!(value["server"].is_null());
    assert!(value["connection"].is_null());
    assert!(!output.contains("token"));
    assert!(status(dir.path(), false)
        .await
        .unwrap()
        .contains("not running"));
    assert!(!dir.path().join("client-api.json").exists());
}

#[tokio::test]
async fn test_停止_未起動なら失敗する() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    // When / Then
    assert_eq!(
        run(dir.path(), ServerSubcommand::Stop)
            .await
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::Unavailable
    );
}
