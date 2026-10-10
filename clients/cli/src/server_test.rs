use super::*;

#[test]
fn test_起動成功_jsonは起動の有無を返し平文は維持する() {
    // Given / When / Then
    for (started, human) in [
        (true, "server started\n"),
        (false, "server is already running\n"),
    ] {
        let machine: serde_json::Value =
            serde_json::from_str(&start_output(started, true)).unwrap();
        assert_eq!(machine, json!({"started": started}));
        assert_eq!(start_output(started, false), human);
    }
}

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
    assert_eq!(value.get("server"), Some(&serde_json::Value::Null));
    assert_eq!(value.get("connection"), Some(&serde_json::Value::Null));
    assert_eq!(value.get("uptime_seconds"), Some(&serde_json::Value::Null));
    assert_eq!(value.get("compatibility"), Some(&serde_json::Value::Null));
    assert_eq!(value.get("guidance"), Some(&serde_json::Value::Null));
    assert_eq!(value["startup_guidance"], startup_guidance());
    assert_eq!(
        value["discovery_file"],
        discovery::discovery_file(dir.path()).to_str().unwrap()
    );
    assert!(!output.contains("token"));
    assert!(status(dir.path(), false)
        .await
        .unwrap()
        .contains("not running"));
    assert!(!discovery::discovery_file(dir.path()).exists());
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

#[tokio::test]
async fn test_引数なし起動_独自data_dirではアプリ起動を要求せず理由と状態を表示する() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("Releash.app/Contents/MacOS/releash");
    // When
    let output = launch_started(dir.path(), &executable, |app| {
        panic!("アプリ起動を要求してはいけない: {}", app.display());
    })
    .await
    .unwrap();
    // Then
    assert_eq!(
        output,
        format!(
            "app was not opened: data dir differs from the desktop default\n{}",
            status(dir.path(), false).await.unwrap(),
        ),
    );
}
