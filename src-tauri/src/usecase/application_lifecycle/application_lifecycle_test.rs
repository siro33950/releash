use super::test_helpers::{FakeShutdown, STAGES};
use super::*;

#[test]
fn test_終了要求_intentと失敗をそのまま伝える() {
    struct QuitPort;
    impl ApplicationQuitIntentPort for QuitPort {
        fn execute(&self, intent: ApplicationQuitIntent) -> Result<(), ApplicationLifecycleError> {
            assert_eq!(intent, ApplicationQuitIntent::Restart { code: 23 });
            Err(ApplicationLifecycleError("receiver unavailable".into()))
        }
    }
    // Given / When
    let error = request_quit(&QuitPort, ApplicationQuitIntent::Restart { code: 23 }).unwrap_err();
    // Then
    assert_eq!(error.to_string(), "receiver unavailable");
}

#[tokio::test(start_paused = true)]
async fn test_終了処理_各段階の失敗後も順序を守って残りを行う() {
    crate::test_support::install_capturing_logger();
    for failed in std::iter::once(None).chain(STAGES.map(Some)) {
        // Given
        let gateway = FakeShutdown {
            failed,
            failure_id: uuid::Uuid::new_v4().to_string(),
            ..Default::default()
        };
        // When
        shutdown(&gateway).await;
        // Then
        let messages = crate::test_support::captured_error_messages()
            .into_iter()
            .filter(|message| message.contains(&gateway.failure_id))
            .collect::<Vec<_>>();
        let expected = failed
            .map(|name| {
                let stage = match name {
                    "commands" => "command stop",
                    "observer" => "provider exit observer stop",
                    "terminals" => "terminal state save",
                    "api" => "local API stop",
                    "telemetry" => "telemetry shutdown",
                    _ => unreachable!(),
                };
                format!(
                    "application shutdown: {stage} failed: {name} failed {}",
                    gateway.failure_id
                )
            })
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(messages, expected);
        assert_eq!(*gateway.calls.lock().unwrap(), STAGES);
    }
}

#[tokio::test(start_paused = true)]
async fn test_終了処理_どの段階が停止しても全体で15秒以内に打ち切る() {
    for (index, blocked) in STAGES.into_iter().enumerate() {
        // Given
        let gateway = FakeShutdown {
            blocked: Some(blocked),
            delay: std::time::Duration::from_secs(2),
            ..Default::default()
        };
        let started = tokio::time::Instant::now();
        // When
        shutdown(&gateway).await;
        // Then
        assert_eq!(started.elapsed(), std::time::Duration::from_secs(15));
        assert_eq!(*gateway.calls.lock().unwrap(), STAGES[..=index]);
    }
}

#[tokio::test(start_paused = true)]
async fn test_終了処理_正常時は期限を待たず終了へ進む() {
    // Given
    let gateway = FakeShutdown::default();
    let started = tokio::time::Instant::now();
    // When
    shutdown(&gateway).await;
    // Then
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(*gateway.calls.lock().unwrap(), STAGES);
}
