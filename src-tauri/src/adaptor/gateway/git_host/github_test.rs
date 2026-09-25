use super::*;
use crate::common::operation_context::sync_scope;
use crate::common::operation_context::{Deadline, OperationContext, OperationStopped};
use std::time::Instant;

#[cfg(unix)]
#[test]
fn test_gh実runner_呼出期限と資源期限の早い方で停止する() {
    // Given
    let runner = SystemGhCommandRunner {
        program: Some("/bin/sh".into()),
    };
    for deadline in [Some(Duration::from_millis(50)), Some(GH_TIMEOUT * 3), None] {
        let start = Instant::now();
        let context = OperationContext::default();
        let context = deadline.map_or(context.clone(), |duration| {
            context.with_deadline(Deadline::new(start + duration))
        });
        // When
        let output = sync_scope(context, || runner.output(&["-c", "exec sleep 30"], "/"));
        // Then
        assert_eq!(output, GhCommandOutput::Timeout);
        let expected = deadline.unwrap_or(GH_TIMEOUT).min(GH_TIMEOUT);
        assert!(start.elapsed() >= expected);
        assert!(start.elapsed() < expected + Duration::from_secs(3));
    }
}

#[cfg(unix)]
#[test]
fn test_gh実runner_実行中の取消を保持する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let started = directory.path().join("started");
    let token = tokio_util::sync::CancellationToken::new();
    let context = OperationContext::new(None, Arc::new(token.clone()));
    let runner = SystemGhCommandRunner {
        program: Some("/bin/sh".into()),
    };
    let task = std::thread::spawn(move || {
        sync_scope(context, || {
            runner.output(
                &["-c", "touch started; exec sleep 30"],
                directory.path().to_str().unwrap(),
            )
        })
    });
    let start = Instant::now();
    while !started.exists() {
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(5));
    }
    // When
    token.cancel();
    // Then
    assert_eq!(
        task.join().unwrap(),
        GhCommandOutput::Stopped(OperationStopped::Cancelled)
    );
}

#[cfg(unix)]
#[test]
fn test_gh実runner_出力と起動失敗の変換を保つ() {
    // Given
    let runner = SystemGhCommandRunner {
        program: Some("/bin/sh".into()),
    };
    // When / Then
    assert_eq!(
        runner.output(&["-c", "printf ok"], "/"),
        GhCommandOutput::Success("ok".into())
    );
    assert!(
        matches!(runner.output(&["-c", "printf error >&2; exit 7"], "/"), GhCommandOutput::NonZero { stderr, .. } if stderr == "error")
    );
    assert_eq!(
        runner.output(&["-c", "printf '\\377'"], "/"),
        GhCommandOutput::InvalidUtf8
    );
    let missing = tempfile::tempdir().unwrap();
    let runner = SystemGhCommandRunner {
        program: Some(missing.path().join("missing-gh")),
    };
    assert!(matches!(
        runner.output(&[], "/"),
        GhCommandOutput::SpawnFailed(_)
    ));
}
