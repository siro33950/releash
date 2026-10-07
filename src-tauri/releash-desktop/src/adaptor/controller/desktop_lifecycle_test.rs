use super::*;
use crate::domain::daemon_connection::DaemonConnectionState;
#[tokio::test]
async fn test_停止確認_却下と確認失敗では停止せず承諾時だけ停止する() {
    // Given
    let stopped = std::sync::atomic::AtomicUsize::new(0);
    for (confirmed, expected) in [
        (Ok(false), Ok(())),
        (Ok(true), Ok(())),
        (
            Err("confirmation closed".into()),
            Err(DaemonConnectionState::TechnicalFailure(
                "confirmation closed".into(),
            )),
        ),
    ] {
        // When
        let result = stop_after_confirmation(confirmed, || async {
            stopped.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await;
        // Then
        assert_eq!(result, expected);
    }
    assert_eq!(stopped.load(std::sync::atomic::Ordering::SeqCst), 1);
    // When / Then
    let failure = DaemonConnectionState::TechnicalFailure("stop refused".into());
    assert_eq!(
        stop_after_confirmation(Ok(true), || async { Err(failure.clone()) }).await,
        Err(failure)
    );
}
