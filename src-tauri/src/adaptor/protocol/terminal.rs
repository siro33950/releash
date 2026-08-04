use serde::{Deserialize, Serialize};

use crate::domain::terminal_surface::entities::{TerminalSurface, TerminalSurfaceSummary};
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
use crate::usecase::terminal_surface::spawn_usecase::GetOrSpawnTerminalOutcome;

#[derive(Clone, Debug, Serialize)]
pub struct TerminalSurfaceCheckpointV1 {
    pub replay: String,
    pub sequence: u64,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalSurfaceV1 {
    pub session_key: String,
    pub terminal_surface: TerminalSurfaceCheckpointV1,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl From<TerminalSurface> for TerminalSurfaceV1 {
    fn from(surface: TerminalSurface) -> Self {
        Self {
            session_key: surface.session_key,
            terminal_surface: TerminalSurfaceCheckpointV1 {
                replay: surface.checkpoint.replay,
                sequence: surface.checkpoint.sequence,
                cols: surface.checkpoint.cols,
                rows: surface.checkpoint.rows,
            },
            is_exited: surface.process_state.is_exited(),
            exit_code: surface.process_state.exit_code(),
            label: surface.label,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GetOrSpawnTerminalV1 {
    pub session_key: String,
    pub restored_from_checkpoint: bool,
    pub is_new: bool,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
}

impl From<GetOrSpawnTerminalOutcome> for GetOrSpawnTerminalV1 {
    fn from(outcome: GetOrSpawnTerminalOutcome) -> Self {
        let surface = outcome.surface;
        Self {
            session_key: surface.session_key,
            restored_from_checkpoint: outcome.restored_from_checkpoint,
            is_new: outcome.is_new,
            is_exited: surface.process_state.is_exited(),
            exit_code: surface.process_state.exit_code(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalSurfaceInfoV1 {
    pub session_key: String,
    pub worktree_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub is_exited: bool,
}

impl From<TerminalSurfaceSummary> for TerminalSurfaceInfoV1 {
    fn from(surface: TerminalSurfaceSummary) -> Self {
        Self {
            session_key: surface.session_key,
            worktree_path: surface.worktree_path,
            label: surface.label,
            is_exited: surface.process_state.is_exited(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalSurfaceAvailabilityV1 {
    pub unavailable_session_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalSurfaceStreamItemV1 {
    Snapshot {
        surface: TerminalSurfaceV1,
    },
    Output {
        session_key: String,
        data: String,
        sequence: u64,
    },
    Resize {
        session_key: String,
        cols: u16,
        rows: u16,
        sequence: u64,
    },
    Exit {
        session_key: String,
        exit_code: Option<i32>,
        sequence: u64,
    },
}

impl From<TerminalSurfaceStreamItem> for TerminalSurfaceStreamItemV1 {
    fn from(item: TerminalSurfaceStreamItem) -> Self {
        match item {
            TerminalSurfaceStreamItem::Snapshot(surface) => Self::Snapshot {
                surface: surface.into(),
            },
            TerminalSurfaceStreamItem::Output {
                session_key,
                data,
                sequence,
            } => Self::Output {
                session_key,
                data,
                sequence,
            },
            TerminalSurfaceStreamItem::Resize {
                session_key,
                cols,
                rows,
                sequence,
            } => Self::Resize {
                session_key,
                cols,
                rows,
                sequence,
            },
            TerminalSurfaceStreamItem::Exit {
                session_key,
                exit_code,
                sequence,
            } => Self::Exit {
                session_key,
                exit_code,
                sequence,
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TerminalSurfaceOwnerV1 {
    Workspace {
        workspace_path: String,
    },
    Session {
        workspace_path: String,
        session_id: String,
    },
}

impl TryFrom<TerminalSurfaceOwnerV1> for TerminalSurfaceOwner {
    type Error = String;

    fn try_from(value: TerminalSurfaceOwnerV1) -> Result<Self, Self::Error> {
        let (workspace_path, owner_id) = match &value {
            TerminalSurfaceOwnerV1::Workspace { workspace_path } => (workspace_path, None),
            TerminalSurfaceOwnerV1::Session {
                workspace_path,
                session_id,
            } => (workspace_path, Some(("sessionId", session_id))),
        };
        if workspace_path.trim().is_empty() {
            return Err("Terminal Surface workspacePath must not be empty".to_string());
        }
        if let Some((field, value)) = owner_id {
            if value.trim().is_empty() {
                return Err(format!("Terminal Surface {field} must not be empty"));
            }
        }

        Ok(match value {
            TerminalSurfaceOwnerV1::Workspace { workspace_path } => {
                TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(workspace_path))
            }
            TerminalSurfaceOwnerV1::Session {
                workspace_path,
                session_id,
            } => TerminalSurfaceOwner::session(WorkspaceIdentity::new(workspace_path), session_id),
        })
    }
}

#[cfg(test)]
#[path = "terminal_test.rs"]
mod terminal_tests;
