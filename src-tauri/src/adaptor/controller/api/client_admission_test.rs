use super::*;
#[tokio::test]
async fn test_connect受付_停止後は情報と停止以外を同じ分類で拒否する() {
    // Given
    let daemon = crate::usecase::daemon::DaemonUsecase(crate::adaptor::gateway::daemon::serving());
    let admission = DaemonAdmission(daemon.clone());
    let paths = [
        "GetServerInfo",
        "StopDaemon",
        "OpenStateStream",
        "StartStateSubscription",
        "StopStateSubscription",
        "ReportTerminalProcessed",
        "GetLanguageFromPath",
    ];
    for method in paths {
        admission
            .admit(&format!("/releash.client.v1.ClientService/{method}"))
            .await
            .unwrap();
    }
    // When
    daemon
        .stop(crate::domain::daemon::StopRequest::Exit { code: 0 })
        .await;
    // Then
    for method in paths {
        let result = admission
            .admit(&format!("/releash.client.v1.ClientService/{method}"))
            .await;
        if matches!(method, "GetServerInfo" | "StopDaemon") {
            result.unwrap();
        } else {
            assert_eq!(
                result.unwrap_err().code,
                connectrpc::ErrorCode::FailedPrecondition
            );
        }
    }
}
