use std::path::Path;

/// Filesystem operations performed by a production app-data collaborator.
///
/// Acceptance compositions inject one observer through every collaborator so
/// B-070 can detect access outside the fixed SQLite and explicitly retained
/// nonlegacy families. Production uses the no-op implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppDataPathOperation {
    Open,
    Metadata,
    ReadDir,
    Read,
    Write,
    Rename,
    Remove,
    Sync,
}

pub trait AppDataPathObserver: Send + Sync {
    fn observe(&self, operation: AppDataPathOperation, path: &Path);
}

#[derive(Debug, Default)]
pub struct NoopAppDataPathObserver;

impl AppDataPathObserver for NoopAppDataPathObserver {
    fn observe(&self, _operation: AppDataPathOperation, _path: &Path) {}
}
