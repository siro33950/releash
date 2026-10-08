mod application_lifecycle_tests {
    use super::super::test_helpers::{FakeShutdown, STAGES};
    use super::super::*;

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
}
