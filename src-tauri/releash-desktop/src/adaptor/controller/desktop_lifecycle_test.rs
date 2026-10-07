use super::*;
use crate::domain::daemon_connection::DaemonConnectionFailure;
#[tokio::test]
async fn test_停止確認_却下と確認失敗では停止せず承諾時だけ停止する() {
    // Given
    for (confirmed, expected, count) in [
        (Ok(false), Ok(()), 0),
        (Ok(true), Ok(()), 1),
        (
            Err("confirmation closed".into()),
            Err(DaemonConnectionFailure::TechnicalFailure(
                "confirmation closed".into(),
            )),
            0,
        ),
    ] {
        let stopped = std::sync::atomic::AtomicUsize::new(0);
        // When
        let result = stop_after_confirmation(confirmed, || async {
            stopped.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await;
        // Then
        assert_eq!(result, expected);
        assert_eq!(stopped.load(std::sync::atomic::Ordering::SeqCst), count);
    }
    // When / Then
    let failure = DaemonConnectionFailure::TechnicalFailure("stop refused".into());
    assert_eq!(
        stop_after_confirmation(Ok(true), || async { Err(failure.clone()) }).await,
        Err(failure)
    );
}
