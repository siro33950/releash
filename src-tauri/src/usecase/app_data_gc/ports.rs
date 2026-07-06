use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use super::request::{
    GcWorktreePath, ProcessRecord, RuntimeProtection, WorkflowArchivePruneResult,
};
use crate::usecase::agent_session::session::SessionState;

pub(crate) trait GcFileSystem {
    fn exists(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, GcFileSystemError>;
    fn read_to_string(&self, path: &Path) -> Result<String, GcFileSystemError>;
    fn remove_path(&self, path: &Path) -> Result<bool, String>;
    fn recursive_size(&self, path: &Path) -> Result<u64, String>;
}

pub(crate) trait WorkflowArchivePruner {
    fn prune_workflow_archive_records(
        &self,
        app_data_dir: &Path,
        run_ids: &std::collections::HashSet<String>,
    ) -> Result<WorkflowArchivePruneResult, String>;
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CurrentSessionState {
    pub(crate) worktree_path: Option<GcWorktreePath>,
    pub(crate) state: Option<SessionState>,
    pub(crate) updated_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CurrentWorkflowRunState {
    pub(crate) worktree_path: GcWorktreePath,
    pub(crate) is_terminal: bool,
    pub(crate) manual_archived_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RevalidationRead<T> {
    Present(T),
    Missing,
    Unavailable(String),
}

pub(crate) trait GcRevalidationReader {
    fn runtime_protection(
        &self,
        app_data_dir: &Path,
        process_records: &[ProcessRecord],
    ) -> RuntimeProtection;

    fn session_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> RevalidationRead<CurrentSessionState>;

    fn workflow_run_state(
        &self,
        app_data_dir: &Path,
        run_id: &str,
    ) -> RevalidationRead<CurrentWorkflowRunState>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GcFileSystemErrorKind {
    NotFound,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GcFileSystemError {
    kind: GcFileSystemErrorKind,
    message: String,
}

impl GcFileSystemError {
    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: GcFileSystemErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub(crate) fn other(message: impl Into<String>) -> Self {
        Self {
            kind: GcFileSystemErrorKind::Other,
            message: message.into(),
        }
    }

    pub(crate) fn is_not_found(&self) -> bool {
        self.kind == GcFileSystemErrorKind::NotFound
    }
}

impl fmt::Display for GcFileSystemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl From<io::Error> for GcFileSystemError {
    fn from(error: io::Error) -> Self {
        gc_file_system_error(error)
    }
}

pub(crate) fn gc_file_system_error(error: io::Error) -> GcFileSystemError {
    if error.kind() == io::ErrorKind::NotFound {
        GcFileSystemError::not_found(error.to_string())
    } else {
        GcFileSystemError::other(error.to_string())
    }
}
