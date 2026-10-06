use super::*;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::infrastructure::terminal::native_pty::NativePtyResizer;
pub struct MockWriter(pub Arc<Mutex<Vec<u8>>>);
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
pub struct MockKiller {
    pub killed: Arc<std::sync::atomic::AtomicBool>,
}
pub struct MockResizer {
    pub rows: u16,
    pub cols: u16,
}
impl NativePtyResizer for MockResizer {
    fn resize(
        &mut self,
        rows: u16,
        cols: u16,
    ) -> Result<(), crate::infrastructure::terminal::native_pty::NativePtyError> {
        self.rows = rows;
        self.cols = cols;
        Ok(())
    }
}
pub fn insert_test_session_with_resizer(
    gateway: &TerminalSurfaceRuntimeGatewayFor,
    runtime_generation: u64,
    session_key: &str,
    worktree_path: Option<&str>,
    label: Option<&str>,
    resizer: Box<dyn NativePtyResizer + Send>,
) -> (Arc<std::sync::atomic::AtomicBool>, Arc<Mutex<Vec<u8>>>) {
    let written = Arc::new(Mutex::new(Vec::<u8>::new()));
    let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let terminal_surface = Arc::new(Mutex::new(NativeTerminalEmulator::new(
        80,
        24,
        TERMINAL_SURFACE_SCROLLBACK_ROWS,
    )));
    terminal_surface.lock().apply("buffered data");
    let workspace = WorkspaceIdentity::new(worktree_path.unwrap_or("/"));
    let mut session = TerminalSurface::new_with_session_key(
        runtime_generation,
        session_key.to_string(),
        TerminalSurfaceOwner::session(workspace, session_key).unwrap(),
        label.map(str::to_string),
    );
    session.worktree_path = worktree_path.map(str::to_string);
    let checkpoint = terminal_surface.lock().snapshot(1);
    assert!(session.apply_checkpoint(runtime_generation, to_domain_checkpoint(&checkpoint)));
    gateway.runtimes.lock().insert(
        runtime_generation,
        AttachedTerminalRuntime {
            native_pty: NativePtyRuntime::from_parts(
                Box::new(MockWriter(Arc::clone(&written))),
                Box::new(MockKiller {
                    killed: Arc::clone(&killed),
                }),
                resizer,
            ),
            output: None,
            event_order: Arc::new(TerminalSurfaceEventOrder::default()),
            terminal_surface,
            checkpoint_scheduler: None,
            session_key: session_key.to_string(),
            output_drained: Arc::new((Mutex::new(true), parking_lot::Condvar::new())),
            checkpoint_journal: None,
            checkpoint_store: None,
            checkpoint_io: None,
        },
    );
    gateway.insert_surface(session);
    (killed, written)
}
pub fn insert_test_session(
    gateway: &TerminalSurfaceRuntimeGatewayFor,
    runtime_generation: u64,
    session_key: &str,
    worktree_path: Option<&str>,
    label: Option<&str>,
) -> Arc<std::sync::atomic::AtomicBool> {
    insert_test_session_with_resizer(
        gateway,
        runtime_generation,
        session_key,
        worktree_path,
        label,
        Box::new(MockResizer { rows: 24, cols: 80 }),
    )
    .0
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
impl portable_pty::Child for MockKiller {
    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        Ok(Some(portable_pty::ExitStatus::with_exit_code(0)))
    }

    fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
        Ok(portable_pty::ExitStatus::with_exit_code(0))
    }

    fn process_id(&self) -> Option<u32> {
        None
    }

    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}
