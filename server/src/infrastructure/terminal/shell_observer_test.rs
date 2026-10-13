use super::*;

#[test]
fn test_shell観測駆動_exec前は待ち本当の失敗だけを警告する() {
    // Given
    let _guard = crate::test_support::lock_crash_telemetry();
    crate::test_support::install_capturing_logger();
    let message = format!("shell-observer-{}", uuid::Uuid::new_v4());
    let mut ticks = 0;
    let mut calls = 0;
    // When
    run(
        || {
            calls += 1;
            if calls == 1 {
                Ok(None)
            } else {
                Err(NativePtyError {
                    kind: std::io::ErrorKind::PermissionDenied,
                    message: message.clone(),
                })
            }
        },
        || {
            ticks += 1;
            ticks <= 3
        },
        |_| panic!("no observation available"),
    );
    // Then
    assert_eq!(calls, 3);
    let warnings = crate::test_support::captured_warning_messages();
    assert_eq!(
        warnings
            .iter()
            .filter(|warning| warning.contains(&message))
            .count(),
        1
    );
    assert!(!warnings
        .iter()
        .any(|warning| warning.contains("Shell has not executed yet")));
}
