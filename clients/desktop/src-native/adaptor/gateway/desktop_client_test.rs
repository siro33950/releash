use super::*;

#[test]
fn test_desktop設定の受信_全項目をwatchへ送りfalseも保持する() {
    for enabled in [false, true] {
        // Given
        let (sender, mut receiver) = tokio::sync::watch::channel(None);
        let payload = wire::StatePayload {
            value: Some(wire::state_payload::Value::DesktopSettings(
                wire::DesktopSettings {
                    start_minimized: Some(enabled),
                    crash_reporting: Some(!enabled),
                    performance_telemetry: Some(enabled),
                    auto_launch: Some(!enabled),
                },
            )),
        };

        // When
        apply_settings(payload, &sender);

        // Then
        assert!(receiver.has_changed().unwrap());
        assert_eq!(
            *receiver.borrow_and_update(),
            Some(DesktopSettingsDto {
                start_minimized: enabled,
                crash_reporting: !enabled,
                performance_telemetry: enabled,
                auto_launch: !enabled,
            })
        );
    }
}

#[test]
fn test_desktop設定の受信_必須項目が欠けた通知と別の通知は既存値を変えない() {
    // Given
    let original = DesktopSettingsDto {
        start_minimized: false,
        crash_reporting: false,
        performance_telemetry: false,
        auto_launch: false,
    };
    let (sender, receiver) = tokio::sync::watch::channel(Some(original));
    for field in 0..4 {
        let mut value = wire::DesktopSettings {
            start_minimized: Some(true),
            crash_reporting: Some(true),
            performance_telemetry: Some(true),
            auto_launch: Some(true),
        };
        [
            &mut value.start_minimized,
            &mut value.crash_reporting,
            &mut value.performance_telemetry,
            &mut value.auto_launch,
        ][field]
            .take();

        // When
        apply_settings(
            wire::StatePayload {
                value: Some(wire::state_payload::Value::DesktopSettings(value)),
            },
            &sender,
        );

        // Then
        assert!(!receiver.has_changed().unwrap());
        assert_eq!(*receiver.borrow(), Some(original));
    }
    // When
    apply_settings(wire::StatePayload::default(), &sender);
    apply_settings(
        wire::StatePayload {
            value: Some(wire::state_payload::Value::TerminalStartupCommand(
                Default::default(),
            )),
        },
        &sender,
    );

    // Then
    assert!(!receiver.has_changed().unwrap());
    assert_eq!(*receiver.borrow(), Some(original));
}

#[test]
fn test_購読開始_重複したreadyで開始要求が増えない() {
    // Given
    let client = client(&DaemonEndpoint {
        url: "http://127.0.0.1:1".into(),
        token: "client".into(),
    })
    .unwrap();
    let mut subscription = SettingsSubscription::default();
    subscription.request_if_needed(&client, "client-id");
    let requested = subscription.has_pending();
    subscription.pending = None;
    // When
    subscription.request_if_needed(&client, "client-id");
    // Then
    assert!(requested);
    assert!(!subscription.has_pending());
}

#[test]
fn test_生存確認_connectの失敗を技術的な失敗の性質へ写す() {
    use connectrpc::ErrorCode;
    // Given / When / Then
    for (code, nature) in [
        (
            ErrorCode::DeadlineExceeded,
            TechnicalFailureNature::TimedOut,
        ),
        (ErrorCode::Unavailable, TechnicalFailureNature::Transient),
        (
            ErrorCode::ResourceExhausted,
            TechnicalFailureNature::Transient,
        ),
        (ErrorCode::Aborted, TechnicalFailureNature::Other),
        (ErrorCode::Unauthenticated, TechnicalFailureNature::Other),
        (ErrorCode::Canceled, TechnicalFailureNature::Cancelled),
        (ErrorCode::Internal, TechnicalFailureNature::Other),
    ] {
        let error = connectrpc::ConnectError::new(code, "reason");
        let message = error.to_string();
        let failure = connection_failure(error);
        assert_eq!(failure.nature, nature);
        assert_eq!(failure.message, message);
    }
}

#[test]
fn test_ネイティブエラー_detailが無いか不正な場合は元の診断を保持する() {
    // Given / When / Then
    let error = connectrpc::ConnectError::unavailable("connection refused");
    let expected = error.to_string();
    assert_eq!(error_message(error.clone()), expected);
    for (type_url, value) in [
        ("releash.client.v1.CommandError", Some("invalid-base64")),
        ("releash.client.v1.CommandError", Some("AA")),
        ("releash.client.v1.CommandError", None),
        ("other.Error", Some("AA")),
    ] {
        assert_eq!(
            error_message(error.clone().with_detail(connectrpc::ErrorDetail {
                type_url: type_url.into(),
                value: value.map(Into::into),
                debug: None,
            })),
            expected
        );
    }
}
