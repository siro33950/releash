use super::*;
use std::collections::VecDeque;

struct FakeChild {
    pid: Option<u32>,
    waits: VecDeque<Option<bool>>,
    calls: Vec<&'static str>,
    failure_id: String,
}

impl FakeChild {
    fn error(&self) -> std::io::Error {
        std::io::Error::other(self.failure_id.clone())
    }
}

impl ShutdownChild for FakeChild {
    fn id(&self) -> Option<u32> {
        self.pid
    }
    async fn wait(&mut self) -> std::io::Result<()> {
        self.calls.push("wait");
        match self.waits.pop_front().expect("unexpected wait") {
            Some(true) => Ok(()),
            Some(false) => Err(self.error()),
            None => std::future::pending().await,
        }
    }
    fn start_kill(&mut self) -> std::io::Result<()> {
        self.calls.push("kill");
        Err(self.error())
    }
    #[cfg(unix)]
    fn signal_group(&mut self, _pgid: i32, signal: i32) -> std::io::Result<()> {
        self.calls.push(if signal == libc::SIGTERM {
            "SIGTERM"
        } else {
            "SIGKILL"
        });
        Err(self.error())
    }
}

#[tokio::test]
async fn test_command停止_waitとsignalとkillとreapのos失敗をログへ残して続行する() {
    crate::test_support::install_capturing_logger();
    for pid in [Some(42), None] {
        // Given
        let failure_id = uuid::Uuid::new_v4().to_string();
        let mut child = FakeChild {
            pid,
            waits: [Some(false); 3].into(),
            calls: Vec::new(),
            failure_id: failure_id.clone(),
        };
        // When
        staged_shutdown_child(&mut child, "test command").await;
        // Then
        let messages: Vec<_> = crate::test_support::captured_error_messages()
            .into_iter()
            .filter(|message| message.contains(&failure_id))
            .collect();
        let mut expected = vec!["failed to wait for test command child".to_string()];
        #[cfg(unix)]
        if let Some(pid) = pid {
            expected.push(format!("failed to terminate command process group {pid}"));
        } else {
            expected.push("failed to kill command child".into());
        }
        #[cfg(not(unix))]
        expected.push("failed to kill command child".into());
        expected.push("failed to wait for test command child".into());
        #[cfg(unix)]
        if let Some(pid) = pid {
            expected.push(format!("failed to kill command process group {pid}"));
        }
        expected.push("failed to kill command child".into());
        expected.push("failed to reap test command during shutdown".into());
        assert_eq!(
            messages,
            expected
                .into_iter()
                .map(|message| format!("{message}: {failure_id}"))
                .collect::<Vec<_>>()
        );
        assert!(child.waits.is_empty());
        assert_eq!(child.calls.last(), Some(&"wait"));
    }
}

#[tokio::test(start_paused = true)]
async fn test_command停止_graceを超えたら次のsignalへ進み終了済みなら止める() {
    for (waits, seconds, wait_count) in [
        (vec![Some(true)], 0, 1),
        (vec![None, Some(true)], 5, 2),
        (vec![None, None, Some(true)], 10, 3),
    ] {
        // Given
        let mut child = FakeChild {
            pid: None,
            waits: waits.into(),
            calls: Vec::new(),
            failure_id: uuid::Uuid::new_v4().to_string(),
        };
        let started = tokio::time::Instant::now();
        // When
        staged_shutdown_child(&mut child, "test command").await;
        // Then
        assert_eq!(started.elapsed(), Duration::from_secs(seconds));
        assert_eq!(
            child.calls.iter().filter(|call| **call == "wait").count(),
            wait_count
        );
        assert_eq!(
            child.calls.iter().filter(|call| **call == "kill").count(),
            wait_count - 1
        );
    }
}

#[cfg(unix)]
#[test]
fn test_process_group_signal_危険なpgidを拒否する() {
    for pgid in [-1, 0, 1] {
        let error = signal_process_group(pgid, libc::SIGTERM).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}

#[cfg(unix)]
#[test]
fn test_process_group_signal_存在しない安全なpgidは成功する() {
    signal_process_group(i32::MAX, 0).unwrap();
}
