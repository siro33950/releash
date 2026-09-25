use super::*;
use crate::common::operation_context::{Deadline, OperationContext, OperationStopped};
use std::time::{Duration, Instant};

thread_local! {
    static GIT_PROGRAM: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn git_program() -> std::path::PathBuf {
    GIT_PROGRAM.with_borrow(|program| program.clone().unwrap_or_else(|| "git".into()))
}

#[cfg(unix)]
#[test]
fn test_hunk変更_実行中のprocessを期限と取消で回収し停止分類を返す() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            GIT_PROGRAM.set(None);
        }
    }
    // Given
    for reverse in [false, true] {
        for expire in [false, true] {
            let (dir, _repo) = crate::test_support::git::create_test_repo();
            let program = dir.path().join("fake-git");
            std::fs::write(
                &program,
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > args\necho $$ > pid\nexec sleep 30\n",
            )
            .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            GIT_PROGRAM.set(Some(program));
            let _reset = Reset;
            let token = tokio_util::sync::CancellationToken::new();
            let context = OperationContext::new(
                expire.then(|| Deadline::new(Instant::now() + Duration::from_secs(1))),
                Arc::new(token.clone()),
            );
            let pid_file = dir.path().join("pid");
            let ready = pid_file.clone();
            let signal = std::thread::spawn(move || {
                let start = Instant::now();
                while std::fs::read_to_string(&ready)
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    assert!(
                        start.elapsed() < Duration::from_secs(5),
                        "git did not start"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                if !expire {
                    token.cancel();
                }
            });
            // When
            let result = crate::common::operation_context::sync_scope(context, || {
                if reverse {
                    StagingGateway.unstage_hunk(dir.path().to_str().unwrap(), "patch")
                } else {
                    StagingGateway.stage_hunk(dir.path().to_str().unwrap(), "patch")
                }
            });
            signal.join().unwrap();
            // Then
            assert!(
                matches!(result, Err(CodeError::Technical(error)) if error == if expire { OperationStopped::Expired.into() } else { OperationStopped::Cancelled.into() })
            );
            let pid: i32 = std::fs::read_to_string(pid_file)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join("args")).unwrap(),
                if reverse {
                    "apply\n--cached\n--reverse\n"
                } else {
                    "apply\n--cached\n"
                }
            );
        }
    }
}
