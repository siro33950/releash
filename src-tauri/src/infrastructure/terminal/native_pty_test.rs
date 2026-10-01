use std::ffi::OsStr;
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use portable_pty::CommandBuilder;

use super::{configure_terminal_environment, NativePtyResizer, NativePtyRuntime, NativePtySystem};

struct BlockingWriter {
    started: Arc<(Mutex<bool>, Condvar)>,
    release: Arc<(Mutex<bool>, Condvar)>,
    written: Arc<(Mutex<Vec<u8>>, Condvar)>,
}

impl Write for BlockingWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let (started, changed) = &*self.started;
        *started.lock().unwrap() = true;
        changed.notify_all();

        let (released, changed) = &*self.release;
        let _guard = changed
            .wait_while(released.lock().unwrap(), |released| !*released)
            .unwrap();

        let (written, changed) = &*self.written;
        written.lock().unwrap().extend_from_slice(buffer);
        changed.notify_all();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct NoopKiller;

impl portable_pty::ChildKiller for NoopKiller {
    fn kill(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(Self)
    }
}

struct NoopResizer;

impl NativePtyResizer for NoopResizer {
    fn resize(
        &mut self,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), crate::infrastructure::terminal::native_pty::NativePtyError> {
        Ok(())
    }
}

#[test]
fn test_実pty基盤_プロセスを起動せず構築できる() {
    let _system = NativePtySystem;
}

#[test]
fn test_実pty基盤_provider_tuiから親processの色抑止環境を除去する() {
    let mut command = CommandBuilder::new("provider");
    command.env("NO_COLOR", "1");

    configure_terminal_environment(&mut command, true);

    assert_eq!(command.get_env("NO_COLOR"), None);
    #[cfg(not(target_os = "windows"))]
    {
        assert_eq!(command.get_env("TERM"), Some(OsStr::new("xterm-256color")));
        assert_eq!(command.get_env("COLORTERM"), Some(OsStr::new("truecolor")));
    }
}

#[test]
fn test_実pty基盤_通常terminalでは利用者の色抑止環境を維持する() {
    let mut command = CommandBuilder::new("shell");
    command.env("NO_COLOR", "1");

    configure_terminal_environment(&mut command, false);

    assert_eq!(command.get_env("NO_COLOR"), Some(OsStr::new("1")));
}

#[test]
fn test_実pty基盤_入力はwriter完了を待たず順序キューへ受け付ける() {
    let started = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let written = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
    let runtime = NativePtyRuntime::from_parts(
        Box::new(BlockingWriter {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            written: Arc::clone(&written),
        }),
        Box::new(NoopKiller),
        Box::new(NoopResizer),
    );

    let first = std::thread::spawn({
        let runtime = runtime.clone();
        move || runtime.write(b"first")
    });
    let (has_started, changed) = &*started;
    let guard = changed
        .wait_while(has_started.lock().unwrap(), |has_started| !*has_started)
        .unwrap();
    drop(guard);

    let (completed_tx, completed_rx) = std::sync::mpsc::channel();
    let second = std::thread::spawn(move || {
        completed_tx.send(runtime.write(b"second")).unwrap();
    });
    let second_completed_while_writer_blocked = completed_rx
        .recv_timeout(Duration::from_millis(100))
        .is_ok();

    let (released, changed) = &*release;
    *released.lock().unwrap() = true;
    changed.notify_all();
    first.join().unwrap().unwrap();
    second.join().unwrap();

    let (captured, changed) = &*written;
    let captured = changed
        .wait_timeout_while(
            captured.lock().unwrap(),
            Duration::from_secs(1),
            |captured| captured.len() < b"firstsecond".len(),
        )
        .unwrap()
        .0
        .clone();

    assert!(second_completed_while_writer_blocked);
    assert_eq!(captured, b"firstsecond");
}

#[test]
fn test_pty失敗_io種類と文脈を保持する() {
    use std::io::{Error, ErrorKind};
    for kind in [
        ErrorKind::Interrupted,
        ErrorKind::WouldBlock,
        ErrorKind::ConnectionReset,
        ErrorKind::TimedOut,
        ErrorKind::PermissionDenied,
    ] {
        // Given
        let error = Error::new(kind, "source failure");
        // When
        let failure = super::NativePtyError::io(error, "PTY operation");
        // Then
        assert_eq!(failure.kind, kind);
        assert_eq!(failure.message, "PTY operation: source failure");
    }
}

#[test]
fn test_pty外部失敗_source内のio種類と元のメッセージを保持する() {
    #[derive(Debug, thiserror::Error)]
    #[error("PTY wrapper: {0}")]
    struct Wrapped(#[source] std::io::Error);

    // Given
    let error: Box<dyn std::error::Error + Send + Sync> = Box::new(Wrapped(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "source failure",
    )));
    // When
    let failure = super::NativePtyError::external(error, "PTY operation");
    // Then
    assert_eq!(failure.kind, std::io::ErrorKind::TimedOut);
    assert_eq!(
        failure.message,
        "PTY operation: PTY wrapper: source failure"
    );
}

#[test]
fn test_pty外部失敗_io以外はotherと元のメッセージを保持する() {
    // Given
    let error: Box<dyn std::error::Error + Send + Sync> = Box::new(std::fmt::Error);
    let message = error.to_string();
    // When
    let failure = super::NativePtyError::external(error, "PTY operation");
    // Then
    assert_eq!(failure.kind, std::io::ErrorKind::Other);
    assert_eq!(failure.message, format!("PTY operation: {message}"));
}
