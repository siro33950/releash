use super::*;
#[test]
fn test_起動結果_終了状態とstderrおよび確認期限切れを状態へ変換する() {
    use std::os::unix::process::ExitStatusExt;
    // Given
    let status = std::process::ExitStatus::from_raw(7 << 8);
    // When / Then
    assert_eq!(
        daemon_failure(daemon::DaemonError::Exited {
            status,
            stderr: "failure tail".into()
        }),
        DaemonConnectionFailure::StartupFailed {
            status: Some(status.to_string()),
            stderr: "failure tail".into()
        }
    );
    assert_eq!(
        daemon_failure(daemon::DaemonError::StartupTimeout {
            stderr: "waiting tail".into()
        }),
        DaemonConnectionFailure::StartupFailed {
            status: None,
            stderr: "waiting tail".into()
        }
    );
    let error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing executable");
    assert_eq!(
        daemon_failure(daemon::DaemonError::Io(error)),
        DaemonConnectionFailure::TechnicalFailure("missing executable".into())
    );
}
