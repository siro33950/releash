use super::*;
use crate::common::operation_context::{scope, Deadline, OperationContext};
use std::time::{Duration, Instant};

#[tokio::test]
async fn test_プロセス実行_期限切れでは起動しない() {
    let context = OperationContext::default().with_deadline(Deadline::new(Instant::now()));
    assert!(matches!(
        scope(context, async {
            output(tokio::process::Command::new("/nonexistent"), vec![]).await
        })
        .await,
        Err(ProcessError::Stopped(OperationStopped::Expired))
    ));
}

#[tokio::test]
async fn test_プロセス実行_入出力待ちを期限で止め子を回収する() {
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
    let result = scope(context, async {
        output(command, vec![0; 1024 * 1024]).await
    })
    .await;
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

#[tokio::test]
async fn test_プロセス実行_取消で実行中の子を終了し回収する() {
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
    let result = scope(context, async {
        output(command, vec![0; 1024 * 1024]).await
    })
    .await;
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

#[tokio::test]
async fn test_プロセス実行_外側waitの期限と取消でも子孫を終了する() {
    for cancelled in [false, true] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let parent_file = directory.path().join("parent");
        let descendant_file = directory.path().join("descendant");
        let mut command = tokio::process::Command::new("sh");
        command
            .arg("-c")
            .arg("echo $$ > \"$1\"; sleep 30 & echo $! > \"$2\"; wait")
            .arg("test")
            .arg(&parent_file)
            .arg(&descendant_file);
        let mut operation = Box::pin(scope(OperationContext::default(), output(command, vec![])));
        tokio::select! {
            result = &mut operation => panic!("process completed before stop: {}", result.is_ok()),
            _ = tokio::time::timeout(Duration::from_secs(5), async {
                while std::fs::read_to_string(&descendant_file).unwrap_or_default().trim().parse::<i32>().is_err() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }) => {}
        }
        let token = tokio_util::sync::CancellationToken::new();
        let context = if cancelled {
            OperationContext::new(None, std::sync::Arc::new(token.clone()))
        } else {
            OperationContext::default()
                .with_deadline(Deadline::new(Instant::now() + Duration::from_millis(30)))
        };
        // When
        let (result, _) = tokio::join!(
            crate::common::operation_context::wait(&context, operation),
            async {
                if cancelled {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    token.cancel();
                }
            }
        );
        // Then
        assert!(
            matches!(result, Err(stopped) if stopped == if cancelled { OperationStopped::Cancelled } else { OperationStopped::Expired })
        );
        for path in [parent_file, descendant_file] {
            let pid: i32 = std::fs::read_to_string(path)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                // SAFETY: signal zero checks the known test process without changing it.
                while unsafe { libc::kill(pid, 0) } == 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("process survived outer wait cancellation");
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
        }
    }
}

#[tokio::test]
async fn test_プロセス実行_正常完了でも出力を切り離した子孫を残さない() {
    let directory = tempfile::tempdir().unwrap();
    let pid_file = directory.path().join("descendant");
    let mut command = tokio::process::Command::new("sh");
    command
        .arg("-c")
        .arg("sleep 30 >/dev/null 2>&1 & echo $! > \"$1\"")
        .arg("test")
        .arg(&pid_file);
    assert!(output(command, vec![])
        .await
        .unwrap_or_else(|_| panic!("process failed"))
        .status
        .success());
    let pid: i32 = std::fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        // SAFETY: signal zero only checks the known descendant.
        while unsafe { libc::kill(pid, 0) } == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("descendant survived successful output");
}
