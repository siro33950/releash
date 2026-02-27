use std::io::{Read, Write};
use std::sync::Arc;

use parking_lot::Mutex;

pub struct SpawnConfig {
    pub rows: u16,
    pub cols: u16,
    pub cwd: Option<String>,
    pub shell: String,
    pub integration_dir: Option<std::path::PathBuf>,
    pub pty_id: u64,
}

pub struct BackendSession {
    pub reader: Box<dyn Read + Send>,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub resizer: Arc<Mutex<Box<dyn PtyResizer + Send>>>,
}

pub trait PtyResizer {
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String>;
    fn get_size(&self) -> Result<(u16, u16), String>;
}

pub trait PtyBackend: Send + Sync {
    fn spawn(&self, config: SpawnConfig) -> Result<BackendSession, String>;
    fn backend_name(&self) -> &'static str;
}
