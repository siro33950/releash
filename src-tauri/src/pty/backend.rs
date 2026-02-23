use std::io::{Read, Write};
use std::sync::Arc;

use parking_lot::Mutex;
use portable_pty::ChildKiller;

pub struct SpawnConfig {
    pub rows: u16,
    pub cols: u16,
    pub cwd: Option<String>,
    pub worktree_path: Option<String>,
    pub label: Option<String>,
    pub shell: String,
    pub integration_dir: Option<std::path::PathBuf>,
}

pub struct BackendSession {
    pub reader: Box<dyn Read + Send>,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub killer: Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>,
    pub resizer: Arc<Mutex<Box<dyn PtyResizer + Send>>>,
}

pub trait PtyResizer {
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String>;
    fn get_size(&self) -> Result<(u16, u16), String>;
}

pub struct ExistingSession {
    pub session_id: String,
    pub worktree_path: Option<String>,
    pub label: Option<String>,
}

#[allow(dead_code)]
pub trait PtyBackend: Send + Sync {
    fn spawn(&self, config: SpawnConfig) -> Result<BackendSession, String>;
    fn attach(&self, session_id: &str) -> Result<BackendSession, String>;
    fn list_existing(&self) -> Result<Vec<ExistingSession>, String>;
    fn backend_name(&self) -> &'static str;
}
