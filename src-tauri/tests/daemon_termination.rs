#![cfg(all(debug_assertions, feature = "desktop", unix))]

use std::{os::unix::fs::PermissionsExt, time::Duration};

#[tokio::test]
async fn test_daemon停止_親pipe閉鎖と強制終了で実プロセスと子孫の終了を確認する() {
    for ignores_parent_pipe in [false, true] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("daemon");
        let body = if ignores_parent_pipe {
            "/bin/sleep 30 &\nprintf '%s' $! > \"$2/descendant\"\ntouch \"$2/ready\"\nwait\n"
        } else {
            "touch \"$2/ready\"\nread -r line\nexit 0\n"
        };
        std::fs::write(
            &executable,
            format!("#!/bin/sh\nprintf '%s' $$ > \"$2/pid\"\n{body}"),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        // When
        let elapsed = releash_lib::client_api_acceptance::terminate_daemon_for_acceptance(
            executable,
            directory.path().to_path_buf(),
        )
        .await
        .unwrap();
        // Then
        if ignores_parent_pipe {
            assert!(elapsed >= Duration::from_secs(5));
        } else {
            assert!(
                elapsed < Duration::from_secs(5),
                "parent pipe closure must stop a cooperative daemon"
            );
        }
        assert!(elapsed < Duration::from_secs(10));
        for file in if ignores_parent_pipe {
            vec!["pid", "descendant"]
        } else {
            vec!["pid"]
        } {
            let pid = std::fs::read_to_string(directory.path().join(file))
                .unwrap()
                .parse::<i32>()
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while unsafe { libc::kill(pid, 0) } == 0 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "{file} {pid} survived daemon termination"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
    }
}
