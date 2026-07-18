use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;

const WIRE_RECORD_ENV: &str = "RELEASH_WIRE_RECORD";
const MAX_PENDING_RECORDS: usize = 256;
const MAX_PENDING_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireBackend {
    Claude,
    Codex,
}

impl WireBackend {
    fn file_name(self) -> &'static str {
        match self {
            Self::Claude => "claude.jsonl",
            Self::Codex => "codex.jsonl",
        }
    }
}

struct QueuedRecord {
    raw_line: Vec<u8>,
    reserved_bytes: usize,
}

pub(crate) struct WireRecorder {
    active: Option<ActiveRecorder>,
}

struct ActiveRecorder {
    sender: SyncSender<QueuedRecord>,
    pending_bytes: Arc<AtomicUsize>,
    max_pending_bytes: usize,
    writer: JoinHandle<()>,
}

impl WireRecorder {
    pub(crate) fn from_env(backend: WireBackend) -> Self {
        let Some(root) = std::env::var_os(WIRE_RECORD_ENV) else {
            return Self { active: None };
        };
        let root = PathBuf::from(root);
        if root.as_os_str().is_empty() {
            log::warn!("{WIRE_RECORD_ENV} is empty; wire recording is disabled");
            return Self { active: None };
        }
        Self::start(root, backend, MAX_PENDING_RECORDS, MAX_PENDING_BYTES)
    }

    fn start(
        root: PathBuf,
        backend: WireBackend,
        max_pending_records: usize,
        max_pending_bytes: usize,
    ) -> Self {
        Self::start_with_writer(
            backend,
            max_pending_records,
            max_pending_bytes,
            move |receiver, pending_bytes| {
                writer_loop(receiver, pending_bytes, &root, backend);
            },
        )
    }

    fn start_with_writer<F>(
        backend: WireBackend,
        max_pending_records: usize,
        max_pending_bytes: usize,
        writer_loop: F,
    ) -> Self
    where
        F: FnOnce(Receiver<QueuedRecord>, Arc<AtomicUsize>) + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(max_pending_records);
        let pending_bytes = Arc::new(AtomicUsize::new(0));
        let writer_pending_bytes = Arc::clone(&pending_bytes);
        let writer = match std::thread::Builder::new()
            .name(format!("wire-record-{}", backend.file_name()))
            .spawn(move || writer_loop(receiver, writer_pending_bytes))
        {
            Ok(writer) => writer,
            Err(error) => {
                log::warn!("failed to start wire record writer: {error}");
                return Self { active: None };
            }
        };
        Self {
            active: Some(ActiveRecorder {
                sender,
                pending_bytes,
                max_pending_bytes,
                writer,
            }),
        }
    }

    pub(crate) fn record(&self, raw_line: Vec<u8>) {
        let Some(active) = &self.active else {
            return;
        };
        active.try_record(raw_line);
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) async fn shutdown(&mut self) {
        let Some(active) = self.active.take() else {
            return;
        };
        match tokio::task::spawn_blocking(move || active.shutdown()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("failed to shut down wire record writer: {error}"),
            Err(error) => log::warn!("failed to join wire record shutdown task: {error}"),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(root: PathBuf, backend: WireBackend) -> Self {
        Self::start(root, backend, MAX_PENDING_RECORDS, MAX_PENDING_BYTES)
    }
}

impl ActiveRecorder {
    fn try_record(&self, mut raw_line: Vec<u8>) {
        let append_newline = !raw_line.ends_with(b"\n");
        let Some(reserved_bytes) = raw_line.len().checked_add(usize::from(append_newline)) else {
            log::warn!("wire record line size overflow; dropping line");
            return;
        };
        if self
            .pending_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(reserved_bytes)
                    .filter(|next| *next <= self.max_pending_bytes)
            })
            .is_err()
        {
            log::warn!("wire record queue byte limit reached; dropping {reserved_bytes}-byte line");
            return;
        }
        if append_newline {
            raw_line.push(b'\n');
        }

        let record = QueuedRecord {
            raw_line,
            reserved_bytes,
        };
        match self.sender.try_send(record) {
            Ok(()) => {}
            Err(TrySendError::Full(record)) => {
                self.release_bytes(record.reserved_bytes);
                log::warn!("wire record queue item limit reached; dropping line");
            }
            Err(TrySendError::Disconnected(record)) => {
                self.release_bytes(record.reserved_bytes);
                log::warn!("wire record writer is unavailable; dropping line");
            }
        }
    }

    fn release_bytes(&self, bytes: usize) {
        self.pending_bytes.fetch_sub(bytes, Ordering::AcqRel);
    }

    fn shutdown(self) -> Result<(), String> {
        let Self {
            sender,
            pending_bytes: _,
            max_pending_bytes: _,
            writer,
        } = self;
        drop(sender);
        writer
            .join()
            .map_err(|_| "wire record writer panicked".to_string())?;
        Ok(())
    }
}

fn writer_loop(
    receiver: Receiver<QueuedRecord>,
    pending_bytes: Arc<AtomicUsize>,
    root: &Path,
    backend: WireBackend,
) {
    let path = root.join(backend.file_name());
    let mut file = open_wire_file(root, &path);
    while let Ok(record) = receiver.recv() {
        if let Some(open_file) = file.as_mut() {
            if let Err(error) = open_file.write_all(&record.raw_line) {
                log::warn!(
                    "failed to append wire record file {}: {error}",
                    path.display()
                );
                file = None;
            }
        }
        pending_bytes.fetch_sub(record.reserved_bytes, Ordering::AcqRel);
    }
    if let Some(mut file) = file {
        if let Err(error) = file.flush() {
            log::warn!(
                "failed to flush wire record file {}: {error}",
                path.display()
            );
        }
    }
}

fn open_wire_file(root: &Path, path: &Path) -> Option<File> {
    if let Err(error) = fs::create_dir_all(root) {
        log::warn!(
            "failed to create wire record directory {}: {error}",
            root.display()
        );
        return None;
    }
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => Some(file),
        Err(error) => {
            log::warn!(
                "failed to open wire record file {}: {error}",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};

    struct EnvRestore(Option<OsString>);

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var(WIRE_RECORD_ENV, value),
                None => std::env::remove_var(WIRE_RECORD_ENV),
            }
        }
    }

    #[test]
    fn unset_environment_does_not_record() {
        let _lock = TEST_ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore(std::env::var_os(WIRE_RECORD_ENV));
        std::env::remove_var(WIRE_RECORD_ENV);
        let recorder = WireRecorder::from_env(WireBackend::Claude);

        recorder.record(br#"{"type":"result"}"#.to_vec());

        assert!(recorder.active.is_none());
    }

    #[test]
    fn empty_environment_does_not_record() {
        let _lock = TEST_ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::set_value(WIRE_RECORD_ENV, "");
        let recorder = WireRecorder::from_env(WireBackend::Claude);

        recorder.record(br#"{"type":"result"}"#.to_vec());

        assert!(!recorder.is_active());
    }

    #[tokio::test]
    async fn configured_environment_appends_one_message_per_line_in_order() {
        let env_lock = TEST_ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let env_guard = EnvVarGuard::set_path(WIRE_RECORD_ENV, dir.path());
        let mut claude = WireRecorder::from_env(WireBackend::Claude);
        let mut codex = WireRecorder::from_env(WireBackend::Codex);

        claude.record(br#"{"type":"system"}"#.to_vec());
        claude.record(b"{\"type\":\"result\"}\n".to_vec());
        codex.record(br#"{"method":"turn/completed"}"#.to_vec());
        drop(env_guard);
        drop(env_lock);
        claude.shutdown().await;
        codex.shutdown().await;

        assert_eq!(
            fs::read_to_string(dir.path().join("claude.jsonl")).unwrap(),
            "{\"type\":\"system\"}\n{\"type\":\"result\"}\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("codex.jsonl")).unwrap(),
            "{\"method\":\"turn/completed\"}\n"
        );
    }

    #[tokio::test]
    async fn io_failures_do_not_escape_the_tap() {
        let env_lock = TEST_ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let invalid_root = dir.path().join("not-a-directory");
        fs::write(&invalid_root, "occupied").unwrap();
        let env_guard = EnvVarGuard::set_path(WIRE_RECORD_ENV, &invalid_root);
        let mut recorder = WireRecorder::from_env(WireBackend::Claude);

        recorder.record(br#"{"type":"result"}"#.to_vec());
        drop(env_guard);
        drop(env_lock);
        recorder.shutdown().await;

        assert_eq!(fs::read_to_string(invalid_root).unwrap(), "occupied");
    }

    #[tokio::test]
    async fn queue_drops_records_beyond_item_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (mut recorder, start_writer) =
            paused_recorder(dir.path().to_path_buf(), WireBackend::Claude, 1, 1024);
        recorder.record(b"first\n".to_vec());
        recorder.record(b"second\n".to_vec());

        assert_eq!(pending_bytes(&recorder), b"first\n".len());
        start_writer.send(()).unwrap();
        recorder.shutdown().await;

        assert_eq!(
            fs::read(dir.path().join("claude.jsonl")).unwrap(),
            b"first\n"
        );
    }

    #[tokio::test]
    async fn queue_drops_records_beyond_byte_limit() {
        assert_line_with_byte_limit(b"five!", 4, 6, false).await;
    }

    #[tokio::test]
    async fn queue_byte_limit_boundary_with_existing_newline() {
        assert_line_with_byte_limit(b"abc\n", 4, 4, true).await;
        assert_line_with_byte_limit(b"abcd\n", 4, 5, false).await;
    }

    #[tokio::test]
    async fn queue_byte_limit_boundary_with_appended_newline() {
        assert_line_with_byte_limit(b"abc", 4, 4, true).await;
        assert_line_with_byte_limit(b"abcd", 4, 5, false).await;
    }

    fn paused_recorder(
        root: PathBuf,
        backend: WireBackend,
        max_pending_records: usize,
        max_pending_bytes: usize,
    ) -> (WireRecorder, mpsc::Sender<()>) {
        let (start_writer, wait_for_start) = mpsc::channel();
        let recorder = WireRecorder::start_with_writer(
            backend,
            max_pending_records,
            max_pending_bytes,
            move |receiver, pending_bytes| {
                let _ = wait_for_start.recv();
                writer_loop(receiver, pending_bytes, &root, backend);
            },
        );
        (recorder, start_writer)
    }

    fn pending_bytes(recorder: &WireRecorder) -> usize {
        recorder
            .active
            .as_ref()
            .expect("test recorder should be active")
            .pending_bytes
            .load(Ordering::Acquire)
    }

    async fn assert_line_with_byte_limit(
        raw_line: &[u8],
        max_pending_bytes: usize,
        expected_reserved_bytes: usize,
        accepted: bool,
    ) {
        let reserved_bytes = raw_line.len() + usize::from(!raw_line.ends_with(b"\n"));
        assert_eq!(reserved_bytes, expected_reserved_bytes);
        let dir = tempfile::tempdir().unwrap();
        let (mut recorder, start_writer) = paused_recorder(
            dir.path().to_path_buf(),
            WireBackend::Claude,
            1,
            max_pending_bytes,
        );

        recorder.record(raw_line.to_vec());

        assert_eq!(
            pending_bytes(&recorder),
            if accepted { reserved_bytes } else { 0 }
        );
        start_writer.send(()).unwrap();
        recorder.shutdown().await;

        let mut expected = raw_line.to_vec();
        if accepted && !expected.ends_with(b"\n") {
            expected.push(b'\n');
        }
        assert_eq!(
            fs::read(dir.path().join("claude.jsonl")).unwrap(),
            if accepted { expected } else { Vec::new() }
        );
    }
}
