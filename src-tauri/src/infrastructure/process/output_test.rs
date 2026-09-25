use super::*;
use crate::common::operation_context::{sync_scope, Deadline, OperationContext};
use std::time::{Duration, Instant};

#[test]
fn test_プロセス実行_期限切れでは起動しない() {
    let context = OperationContext::default().with_deadline(Deadline::new(Instant::now()));
    assert!(matches!(
        sync_scope(context, || output(
            tokio::process::Command::new("/nonexistent"),
            vec![]
        )),
        Err(ProcessError::Stopped(OperationStopped::Expired))
    ));
}

#[test]
fn test_プロセス実行_入出力待ちを期限で止め子を回収する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("pid");
    let mut command = tokio::process::Command::new("sh");
    command
        .arg("-c")
        .arg("echo $$ > \"$1\"; sleep 30")
        .arg("test")
        .arg(&pid_file);
    let context = OperationContext::default()
        .with_deadline(Deadline::new(Instant::now() + Duration::from_millis(100)));
    // When
    let result = sync_scope(context, || output(command, vec![0; 1024 * 1024]));
    // Then
    assert!(matches!(
        result,
        Err(ProcessError::Stopped(OperationStopped::Expired))
    ));
    let pid: i32 = std::fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: signal 0 only checks whether the known child is still present.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
}

#[test]
fn test_プロセス実行_取消で実行中の子を終了し回収する() {
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("pid");
    let mut command = tokio::process::Command::new("sh");
    command
        .arg("-c")
        .arg("echo $$ > \"$1\"; sleep 30")
        .arg("test")
        .arg(&pid_file);
    let token = tokio_util::sync::CancellationToken::new();
    let context = OperationContext::new(None, std::sync::Arc::new(token.clone()));
    let ready = pid_file.clone();
    let cancel = std::thread::spawn(move || {
        let limit = Instant::now() + Duration::from_secs(5);
        while std::fs::read_to_string(&ready)
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            assert!(Instant::now() < limit, "child did not start");
            std::thread::sleep(Duration::from_millis(1));
        }
        token.cancel();
    });
    let result = sync_scope(context, || output(command, vec![0; 1024 * 1024]));
    cancel.join().unwrap();
    assert!(matches!(
        result,
        Err(ProcessError::Stopped(OperationStopped::Cancelled))
    ));
    let pid: i32 = std::fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: signal 0 only checks whether the known child is still present.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
}
