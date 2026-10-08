use releashd::test_support::client_api_acceptance::{
    ClientRecoveryAcceptanceHost, ClientRecoveryState,
};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn test_実クライアント復旧_無関係な完了後も確定済み設定と副作用を保持する() {
    // Given
    let host = ClientRecoveryAcceptanceHost::start().await;
    // When
    let output = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/helpers/client-recovery.mjs"
        ))
        .args(&host.urls)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(30), output)
        .await
        .expect("client recovery deadline")
        .expect("node client recovery");
    // Then
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        *host.state.lock().unwrap(),
        ClientRecoveryState {
            crash_reporting: false,
            mounted_xterms: 3,
            effects: vec!["crash:true".into(), "xterms:3".into(), "crash:false".into()],
        }
    );
}
