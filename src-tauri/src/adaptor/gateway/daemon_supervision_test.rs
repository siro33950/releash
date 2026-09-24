use super::*;
use std::{cell::Cell, time::Duration};

#[tokio::test]
async fn test_daemon接続_認証と検証が完了した呼び出しで接続情報を返す() {
    use crate::infrastructure::local_api::{
        process_start_time, LocalApiDiscovery, LocalApiDiscoveryFile,
    };
    use prost::Message;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let gateway = DaemonProcessGateway::new(PathBuf::new(), directory.path().into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let discovery = LocalApiDiscovery {
        port: listener.local_addr().unwrap().port(),
        token: "master-token".into(),
        instance_id: "instance".into(),
        pid: std::process::id(),
        process_started_at: process_start_time(std::process::id()).unwrap(),
    };
    LocalApiDiscoveryFile::create(directory.path(), discovery.clone()).unwrap();
    LocalApiDiscoveryFile::create_client(
        directory.path(),
        LocalApiDiscovery {
            token: "client-token".into(),
            ..discovery
        },
    )
    .unwrap();
    let router = axum::Router::new().route(
        "/releash.client.v1.ClientService/GetServerInfo",
        axum::routing::post(|headers: axum::http::HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer client-token");
            assert_eq!(headers["origin"], "tauri://localhost");
            let info = wire::ServerInfo {
                launch_id: "launch".into(),
                release: env!("CARGO_PKG_VERSION").into(),
                desktop_settings: Some(Default::default()),
            };
            (
                [("content-type", "application/proto")],
                info.encode_to_vec(),
            )
        }),
    );
    let peer = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    // When
    let started_at_ms = gateway.monotonic_ms();
    let connection = tokio::time::timeout(Duration::from_secs(2), gateway.connect_client("launch"))
        .await
        .unwrap();
    // Then
    let connection = connection
        .unwrap()
        .expect("connection must be reported immediately");
    assert_eq!(connection.launch_id, "launch");
    assert_eq!(connection.release, env!("CARGO_PKG_VERSION"));
    assert!((started_at_ms..=gateway.monotonic_ms()).contains(&connection.connected_at_ms));
    assert!(gateway.connected());
    let cached = gateway.connection().await.unwrap().unwrap();
    assert_eq!(cached.launch_id, connection.launch_id);
    assert_eq!(cached.connected_at_ms, connection.connected_at_ms);
    peer.abort();
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_自然終了なら強制終了しない() {
    // Given
    let start = tokio::time::Instant::now();
    // When
    wait_for_termination(
        || std::future::ready(Ok(start.elapsed() >= Duration::from_secs(1))),
        || panic!("a stopped daemon must not be killed"),
    )
    .await
    .unwrap();
    // Then
    assert_eq!(start.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_五秒後に一度強制終了して終了確認まで待つ() {
    // Given
    let start = tokio::time::Instant::now();
    let killed = Cell::new(false);
    // When
    wait_for_termination(
        || std::future::ready(Ok(start.elapsed() >= Duration::from_secs(6))),
        || {
            assert_eq!(start.elapsed(), Duration::from_secs(5));
            assert!(!killed.replace(true));
            Ok(())
        },
    )
    .await
    .unwrap();
    // Then
    assert!(killed.get());
    assert_eq!(start.elapsed(), Duration::from_secs(6));
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_強制終了後も終了未確認なら十秒で失敗する() {
    // Given
    let start = tokio::time::Instant::now();
    let kills = Cell::new(0);
    // When
    let error = wait_for_termination(
        || std::future::ready(Ok(false)),
        || {
            kills.set(kills.get() + 1);
            Ok(())
        },
    )
    .await
    .unwrap_err();
    // Then
    assert_eq!(
        error,
        "Daemon exit could not be confirmed after termination."
    );
    assert_eq!(start.elapsed(), Duration::from_secs(10));
    assert_eq!(kills.get(), 1);
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_終了観測とkillのエラーを伝播する() {
    // Given / When / Then
    assert_eq!(
        wait_for_termination(
            || std::future::ready(Err("wait failed".into())),
            || panic!("must not kill after an observation error"),
        )
        .await
        .unwrap_err(),
        "wait failed"
    );
    assert_eq!(
        wait_for_termination(
            || std::future::ready(Ok(false)),
            || Err("kill failed".into()),
        )
        .await
        .unwrap_err(),
        "kill failed"
    );
}

#[tokio::test]
async fn test_daemon停止_子がない場合は即座に完了する() {
    // Given
    let gateway = DaemonProcessGateway::new(PathBuf::new(), PathBuf::new());
    // When / Then
    gateway.terminate_and_wait().await.unwrap();
}

#[test]
fn test_停止応答_acceptedを受理し異なる応答を拒否する() {
    // Given
    use wire::{application_quit_outcome_dto_v1 as outcome, command_result::Command};
    let response = |variant| {
        Command::RequestApplicationQuit(wire::ApplicationQuitOutcomeDtoV1 {
            variant: Some(variant),
        })
    };
    // When / Then
    shutdown_response(response(outcome::Variant::Accepted(Default::default()))).unwrap();
    assert!(shutdown_response(Command::RequestApplicationQuit(
        wire::ApplicationQuitOutcomeDtoV1 { variant: None },
    ))
    .is_err());
    assert_eq!(
        shutdown_response(Command::GetExternalEditor(Default::default())).unwrap_err(),
        "Unexpected shutdown response."
    );
}
