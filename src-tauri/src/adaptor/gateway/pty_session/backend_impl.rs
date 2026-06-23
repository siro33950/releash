use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use crate::adaptor::protocol::pty::{PtyExitMsg, PtyOutputMsg};
use crate::domain::pty_session::entities::{PtySession, PtySessionRegistry, PtySessionSnapshot};
use crate::domain::pty_session::gateway::{PtyBackend, PtyResizer, SpawnConfig};
use crate::domain::pty_session::services::{
    append_output_to_ring_buffer, decode_utf8_chunk, OUTPUT_BUFFER_CAPACITY,
};
use crate::protocol::WsMessage;
use crate::usecase::pty_session::dto::FoundPtySession;
use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::{PtyBackendSpawnRequest, PtySessionGateway};
use crate::ws_bridge::WsBroadcaster;

use super::direct::DirectPtyBackend;

pub(crate) struct PtyRuntime {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Arc<Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    resizer: Arc<Mutex<Box<dyn PtyResizer + Send>>>,
    output_buffer: Arc<Mutex<VecDeque<u8>>>,
    reader: Option<Box<dyn Read + Send>>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
}

pub struct PtySessionRuntimeGateway {
    registry: Mutex<PtySessionRegistry>,
    runtimes: Mutex<HashMap<u64, PtyRuntime>>,
    backend: Box<dyn PtyBackend>,
}

impl Default for PtySessionRuntimeGateway {
    fn default() -> Self {
        Self {
            registry: Mutex::new(PtySessionRegistry::default()),
            runtimes: Mutex::new(HashMap::new()),
            backend: Box::new(DirectPtyBackend::new()),
        }
    }
}

/// UTF-8 処理 + リングバッファ更新の純粋ロジック。
/// 戻り値: フィルタ済み出力文字列 (空ならイベント不要)
fn process_pty_output(
    raw_chunk: &[u8],
    pending: &mut Vec<u8>,
    output_buffer: &Mutex<VecDeque<u8>>,
) -> Option<String> {
    let raw = decode_utf8_chunk(raw_chunk, pending)?;
    let result = crate::infrastructure::pty_session::shell_integration::strip_osc_cmd_done(&raw);

    if result.filtered_output.is_empty() {
        return None;
    }

    {
        let mut ring = output_buffer.lock();
        append_output_to_ring_buffer(&mut ring, &result.filtered_output, OUTPUT_BUFFER_CAPACITY);
    }

    Some(result.filtered_output)
}

fn spawn_output_reader(
    app: AppHandle,
    pty_id: u64,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    output_buffer: Arc<Mutex<VecDeque<u8>>>,
) {
    let rt = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let ws = app.try_state::<Arc<WsBroadcaster>>();
        let mut buf = [0u8; 4096];
        let mut pending = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Some(filtered) =
                        process_pty_output(&buf[..n], &mut pending, &output_buffer)
                    {
                        let _ = app.emit(
                            "pty-output",
                            PtyOutput {
                                pty_id,
                                data: filtered.clone(),
                            },
                        );
                        if let Some(ws) = &ws {
                            ws.try_send(WsMessage::PtyOutput(PtyOutputMsg {
                                pty_id,
                                data: filtered,
                            }));
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let exit_code = child.wait().ok().map(|status| status.exit_code() as i32);
        if let Some(gateway) = app.try_state::<Arc<PtySessionRuntimeGateway>>() {
            gateway.mark_exited(pty_id, exit_code);
        }

        let _ = app.emit("pty-exit", PtyExit { pty_id, exit_code });
        if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
            ws.try_send(WsMessage::PtyExit(PtyExitMsg { pty_id, exit_code }));
        }

        // Delayed cleanup: remove exited session after 5 minutes.
        let app_cleanup = app.clone();
        rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            if let Some(gateway) = app_cleanup.try_state::<Arc<PtySessionRuntimeGateway>>() {
                crate::usecase::pty_session::lifecycle_usecase::remove_if_exited(
                    gateway.as_ref(),
                    pty_id,
                );
            }
        });
    });
}

impl PtySessionRuntimeGateway {
    #[allow(dead_code)]
    pub fn with_backend(backend: Box<dyn PtyBackend>) -> Self {
        Self {
            registry: Mutex::new(PtySessionRegistry::default()),
            runtimes: Mutex::new(HashMap::new()),
            backend,
        }
    }

    #[allow(dead_code)]
    pub fn backend_name(&self) -> &'static str {
        self.backend.backend_name()
    }

    fn mark_exited(&self, pty_id: u64, exit_code: Option<i32>) {
        self.registry.lock().mark_exited(pty_id, exit_code);
    }

    fn buffered_output(&self, pty_id: u64) -> String {
        let Some(output_buffer) = self
            .runtimes
            .lock()
            .get(&pty_id)
            .map(|runtime| Arc::clone(&runtime.output_buffer))
        else {
            return String::new();
        };
        let ring = output_buffer.lock();
        let (a, b) = ring.as_slices();
        let mut bytes = Vec::with_capacity(a.len() + b.len());
        bytes.extend_from_slice(a);
        bytes.extend_from_slice(b);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl PtySessionGateway for PtySessionRuntimeGateway {
    type AppContext = AppHandle;
    type Runtime = PtyRuntime;

    fn next_pty_id(&self) -> u64 {
        self.registry.lock().next_pty_id()
    }

    fn spawn_backend(
        &self,
        app: &Self::AppContext,
        request: PtyBackendSpawnRequest,
    ) -> Result<Self::Runtime, UsecaseError> {
        let integration_dir = if request.exec_command.is_some() {
            None
        } else {
            app.path().app_data_dir().ok().and_then(|data_dir| {
                crate::infrastructure::pty_session::shell_integration::create_shell_integration_files(
                    &data_dir,
                )
                .ok()
            })
        };
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        let mut extra_env = Vec::new();
        match crate::path_aliases::prepare_child_env(app.path().app_data_dir().ok()) {
            Ok(env) => extra_env.extend(env),
            Err(e) => {
                return Err(UsecaseError::Gateway(format!(
                    "failed to prepare alias child env for PTY spawn: {e}"
                )));
            }
        }
        let backend_session = self
            .backend
            .spawn(SpawnConfig {
                rows: request.rows,
                cols: request.cols,
                cwd: request.cwd,
                shell,
                integration_dir,
                pty_id: request.pty_id,
                extra_env,
                exec_command: request.exec_command,
            })
            .map_err(UsecaseError::from)?;

        let killer = backend_session.child.clone_killer();
        Ok(PtyRuntime {
            writer: backend_session.writer,
            killer: Arc::new(Mutex::new(killer)),
            resizer: backend_session.resizer,
            output_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(OUTPUT_BUFFER_CAPACITY))),
            reader: Some(backend_session.reader),
            child: Some(backend_session.child),
        })
    }

    fn insert_session(&self, session: PtySession, runtime: Self::Runtime) {
        let pty_id = session.pty_id;
        let active_count = {
            let mut registry = self.registry.lock();
            registry.insert(session);
            registry.len()
        };
        crate::other::telemetry::set_active_pty_count(active_count as u64);
        self.runtimes.lock().insert(pty_id, runtime);
    }

    fn start_output_reader(&self, app: &Self::AppContext, pty_id: u64) -> Result<(), UsecaseError> {
        let (reader, child, output_buffer) = {
            let mut runtimes = self.runtimes.lock();
            let runtime = runtimes
                .get_mut(&pty_id)
                .ok_or_else(|| UsecaseError::Gateway(format!("PTY {} not found", pty_id)))?;
            let reader = runtime.reader.take().ok_or_else(|| {
                UsecaseError::Gateway(format!("PTY {} output reader already started", pty_id))
            })?;
            let child = runtime.child.take().ok_or_else(|| {
                UsecaseError::Gateway(format!(
                    "PTY {} child already moved to output reader",
                    pty_id
                ))
            })?;
            (reader, child, Arc::clone(&runtime.output_buffer))
        };
        spawn_output_reader(app.clone(), pty_id, reader, child, output_buffer);
        Ok(())
    }

    fn find_by_session_key(&self, session_key: &str) -> Option<FoundPtySession> {
        let snapshot = self
            .registry
            .lock()
            .find_by_session_key(session_key)
            .map(PtySession::snapshot)?;
        let buffered_output = self.buffered_output(snapshot.pty_id);
        Some(FoundPtySession {
            snapshot,
            buffered_output,
        })
    }

    fn snapshot(&self, pty_id: u64) -> Option<PtySessionSnapshot> {
        self.registry.lock().get(pty_id).map(PtySession::snapshot)
    }

    fn list_snapshots(&self) -> Vec<PtySessionSnapshot> {
        self.registry.lock().list_snapshots()
    }

    fn select_kill_targets_by_worktree(&self, worktree_path: &str) -> Vec<u64> {
        self.registry
            .lock()
            .select_kill_targets_by_worktree(worktree_path)
    }

    fn select_gc_targets(&self, worktree_path: &str, keep_session_keys: &[String]) -> Vec<u64> {
        self.registry
            .lock()
            .select_gc_targets(worktree_path, keep_session_keys)
    }

    fn remove_session(&self, pty_id: u64) -> Option<PtySessionSnapshot> {
        self.runtimes.lock().remove(&pty_id);
        let (removed, active_count) = {
            let mut registry = self.registry.lock();
            let removed = registry.remove(pty_id);
            (removed, registry.len())
        };
        crate::other::telemetry::set_active_pty_count(active_count as u64);
        removed.map(|session| session.snapshot())
    }

    fn write(&self, pty_id: u64, data: &str) -> Result<(), UsecaseError> {
        let writer = {
            let runtimes = self.runtimes.lock();
            let runtime = runtimes
                .get(&pty_id)
                .ok_or_else(|| UsecaseError::Gateway(format!("PTY {} not found", pty_id)))?;
            Arc::clone(&runtime.writer)
        };
        let mut writer = writer.lock();
        writer
            .write_all(data.as_bytes())
            .map_err(|e| UsecaseError::Gateway(format!("Failed to write to PTY: {}", e)))?;
        writer
            .flush()
            .map_err(|e| UsecaseError::Gateway(format!("Failed to flush: {}", e)))?;
        Ok(())
    }

    fn resize(&self, pty_id: u64, rows: u16, cols: u16) -> Result<(), UsecaseError> {
        if rows == 0 || cols == 0 {
            return Ok(());
        }
        let resizer = {
            let runtimes = self.runtimes.lock();
            let runtime = runtimes
                .get(&pty_id)
                .ok_or_else(|| UsecaseError::Gateway(format!("PTY {} not found", pty_id)))?;
            Arc::clone(&runtime.resizer)
        };
        let result = resizer.lock().resize(rows, cols);
        result.map_err(UsecaseError::from)
    }

    fn get_pty_size(&self, pty_id: u64) -> Result<(u16, u16), UsecaseError> {
        let resizer = {
            let runtimes = self.runtimes.lock();
            let runtime = runtimes
                .get(&pty_id)
                .ok_or_else(|| UsecaseError::Gateway(format!("PTY {} not found", pty_id)))?;
            Arc::clone(&runtime.resizer)
        };
        let result = resizer.lock().get_size();
        result.map_err(UsecaseError::from)
    }

    fn kill_runtime(&self, pty_id: u64) -> Result<(), UsecaseError> {
        let killer = {
            let runtimes = self.runtimes.lock();
            let runtime = runtimes
                .get(&pty_id)
                .ok_or_else(|| UsecaseError::Gateway(format!("PTY {} not found", pty_id)))?;
            Arc::clone(&runtime.killer)
        };
        let result = killer
            .lock()
            .kill()
            .map_err(|e| UsecaseError::Gateway(format!("Failed to kill PTY {}: {}", pty_id, e)));
        result
    }

    fn remove_if_exited(&self, pty_id: u64) {
        if self
            .snapshot(pty_id)
            .is_some_and(|snapshot| snapshot.exited)
        {
            self.remove_session(pty_id);
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PtyOutput {
    pub pty_id: u64,
    pub data: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PtyExit {
    pub pty_id: u64,
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pty_session::services::MAX_PENDING_BYTES;
    use crate::domain::pty_session::PtyKind;

    #[test]
    fn runtime_gateway_default_has_no_sessions() {
        let gateway = PtySessionRuntimeGateway::default();
        assert!(gateway.list_snapshots().is_empty());
    }

    #[test]
    fn backend_name_uses_direct_backend_by_default() {
        let gateway = PtySessionRuntimeGateway::default();
        assert_eq!(gateway.backend_name(), "direct");
    }

    #[test]
    fn write_nonexistent_pty_returns_error() {
        let gateway = PtySessionRuntimeGateway::default();
        let result = gateway.write(99999, "hello");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn resize_nonexistent_pty_returns_error() {
        let gateway = PtySessionRuntimeGateway::default();
        let result = gateway.resize(99999, 24, 80);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn resize_zero_values_are_ignored() {
        let gateway = PtySessionRuntimeGateway::default();
        assert!(gateway.resize(99999, 0, 80).is_ok());
        assert!(gateway.resize(99999, 24, 0).is_ok());
        assert!(gateway.resize(99999, 0, 0).is_ok());
    }

    #[test]
    fn get_pty_size_nonexistent_returns_error() {
        let gateway = PtySessionRuntimeGateway::default();
        let result = gateway.get_pty_size(99999);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn output_buffer_capacity_value() {
        assert_eq!(OUTPUT_BUFFER_CAPACITY, 64 * 1024);
    }

    #[test]
    fn process_pty_output_valid_utf8() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        let result = process_pty_output(b"hello world", &mut pending, &output_buffer);
        assert_eq!(result.as_deref(), Some("hello world"));
        assert!(pending.is_empty());
        assert_eq!(output_buffer.lock().len(), 11);
    }

    #[test]
    fn process_pty_output_incomplete_utf8_pending() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        assert!(process_pty_output(&[0xE3, 0x81], &mut pending, &output_buffer).is_none());
        assert_eq!(pending.len(), 2);
        let result = process_pty_output(&[0x82], &mut pending, &output_buffer);
        assert_eq!(result.as_deref(), Some("あ"));
        assert!(pending.is_empty());
    }

    #[test]
    fn process_pty_output_max_pending_drop() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        let invalid_bytes = vec![0xFF; MAX_PENDING_BYTES + 1];
        assert!(process_pty_output(&invalid_bytes, &mut pending, &output_buffer).is_none());
        assert!(pending.is_empty());
    }

    #[test]
    fn process_pty_output_invalid_below_max_pending_retained() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        let invalid_bytes = vec![0xFF; 10];
        assert!(process_pty_output(&invalid_bytes, &mut pending, &output_buffer).is_none());
        assert_eq!(pending.len(), 10);
    }

    #[test]
    fn process_pty_output_ring_buffer_overflow() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        let data = "x".repeat(OUTPUT_BUFFER_CAPACITY - 10);
        process_pty_output(data.as_bytes(), &mut pending, &output_buffer);
        assert_eq!(output_buffer.lock().len(), OUTPUT_BUFFER_CAPACITY - 10);
        let data2 = "y".repeat(20);
        process_pty_output(data2.as_bytes(), &mut pending, &output_buffer);
        assert_eq!(output_buffer.lock().len(), OUTPUT_BUFFER_CAPACITY);
    }

    #[test]
    fn process_pty_output_exceeds_capacity() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        let data = "z".repeat(OUTPUT_BUFFER_CAPACITY + 100);
        process_pty_output(data.as_bytes(), &mut pending, &output_buffer);
        assert_eq!(output_buffer.lock().len(), OUTPUT_BUFFER_CAPACITY);
    }

    #[test]
    fn process_pty_output_empty_input() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        assert!(process_pty_output(b"", &mut pending, &output_buffer).is_none());
    }

    struct MockWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct MockKiller {
        killed: Arc<std::sync::atomic::AtomicBool>,
    }

    impl portable_pty::ChildKiller for MockKiller {
        fn kill(&mut self) -> Result<(), std::io::Error> {
            self.killed.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(MockKiller {
                killed: Arc::clone(&self.killed),
            })
        }
    }

    struct MockResizer {
        rows: u16,
        cols: u16,
    }

    impl PtyResizer for MockResizer {
        fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
            self.rows = rows;
            self.cols = cols;
            Ok(())
        }

        fn get_size(&self) -> Result<(u16, u16), String> {
            Ok((self.cols, self.rows))
        }
    }

    fn insert_test_session(
        gateway: &PtySessionRuntimeGateway,
        pty_id: u64,
        session_key: &str,
        worktree_path: Option<&str>,
        label: Option<&str>,
        kind: PtyKind,
    ) -> Arc<std::sync::atomic::AtomicBool> {
        let written = Arc::new(Mutex::new(Vec::<u8>::new()));
        let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let output_buffer = Arc::new(Mutex::new(VecDeque::new()));
        output_buffer.lock().extend(b"buffered data");
        gateway.insert_session(
            PtySession::new(
                pty_id,
                session_key.to_string(),
                worktree_path.map(str::to_string),
                label.map(str::to_string),
                kind,
            ),
            PtyRuntime {
                writer: Arc::new(Mutex::new(Box::new(MockWriter(written)))),
                killer: Arc::new(Mutex::new(Box::new(MockKiller {
                    killed: Arc::clone(&killed),
                }))),
                resizer: Arc::new(Mutex::new(Box::new(MockResizer { rows: 24, cols: 80 }))),
                output_buffer,
                reader: None,
                child: None,
            },
        );
        killed
    }

    #[test]
    fn write_success() {
        let gateway = PtySessionRuntimeGateway::default();
        insert_test_session(&gateway, 1, "key", Some("/repo"), None, PtyKind::Terminal);
        assert!(gateway.write(1, "hello").is_ok());
    }

    #[test]
    fn resize_success() {
        let gateway = PtySessionRuntimeGateway::default();
        insert_test_session(&gateway, 1, "key", Some("/repo"), None, PtyKind::Terminal);
        assert!(gateway.resize(1, 30, 100).is_ok());
        assert_eq!(gateway.get_pty_size(1).unwrap(), (100, 30));
    }

    #[test]
    fn find_session_includes_buffered_output() {
        let gateway = PtySessionRuntimeGateway::default();
        insert_test_session(
            &gateway,
            1,
            "key",
            Some("/repo"),
            Some("dev"),
            PtyKind::Terminal,
        );
        let found = gateway.find_by_session_key("key").unwrap();
        assert_eq!(found.snapshot.pty_id, 1);
        assert_eq!(found.snapshot.label.as_deref(), Some("dev"));
        assert_eq!(found.buffered_output, "buffered data");
    }

    #[test]
    fn lifecycle_usecase_kill_removes_registry_and_runtime() {
        let gateway = PtySessionRuntimeGateway::default();
        let killed =
            insert_test_session(&gateway, 1, "key", Some("/repo"), None, PtyKind::Terminal);

        crate::usecase::pty_session::lifecycle_usecase::kill(&gateway, 1).unwrap();

        assert!(killed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(gateway.snapshot(1).is_none());
        assert!(gateway.runtimes.lock().get(&1).is_none());
    }

    #[test]
    fn lifecycle_usecase_kill_by_worktree_uses_registry_selection() {
        let gateway = PtySessionRuntimeGateway::default();
        insert_test_session(
            &gateway,
            1,
            "key-1",
            Some("/repo"),
            Some("dev"),
            PtyKind::Terminal,
        );
        insert_test_session(
            &gateway,
            2,
            "key-2",
            Some("/repo"),
            Some("test"),
            PtyKind::Terminal,
        );
        insert_test_session(
            &gateway,
            3,
            "key-3",
            Some("/other"),
            None,
            PtyKind::Terminal,
        );

        let mut killed =
            crate::usecase::pty_session::lifecycle_usecase::kill_by_worktree(&gateway, "/repo");
        killed.sort_unstable();

        assert_eq!(killed, vec![1, 2]);
        assert!(gateway.snapshot(1).is_none());
        assert!(gateway.snapshot(2).is_none());
        assert!(gateway.snapshot(3).is_some());
    }

    #[test]
    fn lifecycle_usecase_gc_keeps_listed_session_keys() {
        let gateway = PtySessionRuntimeGateway::default();
        insert_test_session(
            &gateway,
            1,
            "key-1",
            Some("/repo"),
            Some("dev"),
            PtyKind::Terminal,
        );
        insert_test_session(
            &gateway,
            2,
            "key-2",
            Some("/repo"),
            Some("test"),
            PtyKind::Terminal,
        );
        insert_test_session(
            &gateway,
            3,
            "key-3",
            Some("/other"),
            None,
            PtyKind::Terminal,
        );

        let killed = crate::usecase::pty_session::lifecycle_usecase::gc_by_worktree(
            &gateway,
            "/repo",
            &[String::from("key-1")],
        );

        assert_eq!(killed, vec![2]);
        assert!(gateway.snapshot(1).is_some());
        assert!(gateway.snapshot(2).is_none());
        assert!(gateway.snapshot(3).is_some());
    }

    #[test]
    fn remove_if_exited_only_removes_exited_session() {
        let gateway = PtySessionRuntimeGateway::default();
        insert_test_session(&gateway, 1, "key-1", Some("/repo"), None, PtyKind::Terminal);
        insert_test_session(&gateway, 2, "key-2", Some("/repo"), None, PtyKind::Terminal);
        gateway.mark_exited(1, Some(0));

        crate::usecase::pty_session::lifecycle_usecase::remove_if_exited(&gateway, 1);
        crate::usecase::pty_session::lifecycle_usecase::remove_if_exited(&gateway, 2);

        assert!(gateway.snapshot(1).is_none());
        assert!(gateway.snapshot(2).is_some());
    }
}
