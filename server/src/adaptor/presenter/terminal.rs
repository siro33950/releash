use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::application::TerminalSurfaceSnapshotDto;
use crate::usecase::terminal_surface::spawn_usecase::GetOrSpawnTerminalOutcome;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalProcessLaunchV1 {
    pub executable: String,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
}

impl TryFrom<TerminalProcessLaunchV1> for TerminalProcessLaunch {
    type Error = String;

    fn try_from(value: TerminalProcessLaunchV1) -> Result<Self, Self::Error> {
        TerminalProcessLaunch::new(value.executable, value.arguments, value.environment)
            .map_err(|error| format!("invalid Terminal process launch: {error:?}"))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalSurfaceCheckpointV1 {
    pub replay: String,
    pub sequence: u64,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalShellStateV1 {
    pub cwd: Option<String>,
    pub has_foreground_process: bool,
}
impl From<crate::domain::terminal_surface::TerminalShellState> for TerminalShellStateV1 {
    fn from(value: crate::domain::terminal_surface::TerminalShellState) -> Self {
        Self {
            cwd: value.cwd,
            has_foreground_process: value.has_foreground_process,
        }
    }
}

impl From<&crate::adaptor::presenter::client::TerminalStatus> for TerminalShellStateV1 {
    fn from(value: &crate::adaptor::presenter::client::TerminalStatus) -> Self {
        Self {
            cwd: value.cwd.clone(),
            has_foreground_process: value.has_foreground_process,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminalSurfaceV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_state: Option<TerminalShellStateV1>,
    pub terminal_surface: TerminalSurfaceCheckpointV1,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl From<TerminalSurfaceSnapshotDto> for TerminalSurfaceV1 {
    fn from(surface: TerminalSurfaceSnapshotDto) -> Self {
        Self {
            shell_state: surface.shell_state.map(Into::into),
            terminal_surface: TerminalSurfaceCheckpointV1 {
                replay: surface.replay,
                sequence: surface.sequence,
                cols: surface.cols,
                rows: surface.rows,
            },
            is_exited: surface.is_exited,
            exit_code: surface.exit_code,
            label: surface.label,
        }
    }
}

impl From<crate::domain::terminal_surface::entities::TerminalSurface> for TerminalSurfaceV1 {
    fn from(surface: crate::domain::terminal_surface::entities::TerminalSurface) -> Self {
        TerminalSurfaceSnapshotDto::from(surface).into()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GetOrSpawnTerminalV1 {}

impl From<GetOrSpawnTerminalOutcome> for GetOrSpawnTerminalV1 {
    fn from(_: GetOrSpawnTerminalOutcome) -> Self {
        Self {}
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalSurfaceStreamItemV1 {
    Status {
        state: TerminalShellStateV1,
        sequence: u64,
    },
    Snapshot {
        surface: TerminalSurfaceV1,
    },
    Output {
        data: Arc<str>,
        sequence: u64,
    },
    Resize {
        cols: u16,
        rows: u16,
        sequence: u64,
    },
    Exit {
        state: Option<TerminalShellStateV1>,
        exit_code: Option<i32>,
        sequence: u64,
    },
}

impl From<crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem>
    for TerminalSurfaceStreamItemV1
{
    fn from(
        item: crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem,
    ) -> Self {
        match item {
            crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem::Status {
                session_key: _,
                state,
                sequence,
            } => Self::Status {
                state: state.into(),
                sequence,
            },
            crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem::Snapshot(
                surface,
            ) => Self::Snapshot {
                surface: surface.into(),
            },
            crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem::Output {
                session_key: _,
                data,
                sequence,
            } => Self::Output { data, sequence },
            crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem::Resize {
                session_key: _,
                cols,
                rows,
                sequence,
            } => Self::Resize {
                cols,
                rows,
                sequence,
            },
            crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem::Exit {
                state,
                session_key: _,
                exit_code,
                sequence,
            } => Self::Exit {
                state: Some(state.into()),
                exit_code,
                sequence,
            },
        }
    }
}

impl TryFrom<&crate::adaptor::presenter::client::TerminalEvent> for TerminalSurfaceStreamItemV1 {
    type Error = String;

    fn try_from(
        value: &crate::adaptor::presenter::client::TerminalEvent,
    ) -> Result<Self, Self::Error> {
        use crate::adaptor::presenter::client::terminal_event::Item;
        match value.item.as_ref().ok_or("Missing terminal event")? {
            Item::Status(status) => Ok(Self::Status {
                state: status.into(),
                sequence: status.sequence,
            }),
            Item::Snapshot(snapshot) => Ok(Self::Snapshot {
                surface: TerminalSurfaceV1 {
                    shell_state: snapshot.status.as_ref().map(Into::into),
                    terminal_surface: TerminalSurfaceCheckpointV1 {
                        replay: snapshot.replay.clone(),
                        sequence: snapshot.sequence,
                        cols: snapshot
                            .cols
                            .try_into()
                            .map_err(|_| "Invalid terminal columns")?,
                        rows: snapshot
                            .rows
                            .try_into()
                            .map_err(|_| "Invalid terminal rows")?,
                    },
                    is_exited: snapshot.is_exited,
                    exit_code: snapshot.exit_code,
                    label: None,
                },
            }),
            Item::Output(output) => Ok(Self::Output {
                data: Arc::from(output.data.as_str()),
                sequence: output.sequence,
            }),
            Item::Resize(resize) => Ok(Self::Resize {
                cols: resize
                    .cols
                    .try_into()
                    .map_err(|_| "Invalid terminal columns")?,
                rows: resize
                    .rows
                    .try_into()
                    .map_err(|_| "Invalid terminal rows")?,
                sequence: resize.sequence,
            }),
            Item::Exit(exit) => Ok(Self::Exit {
                state: exit.status.as_ref().map(Into::into),
                exit_code: exit.exit_code,
                sequence: exit.sequence,
            }),
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
    WorkspaceTerminal {
        workspace_path: String,
        terminal_id: String,
    },
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
        match value {
            TerminalSurfaceOwnerV1::WorkspaceTerminal {
                workspace_path,
                terminal_id,
            } => TerminalSurfaceOwner::workspace_terminal(
                WorkspaceIdentity::new(workspace_path),
                terminal_id,
            ),
            TerminalSurfaceOwnerV1::Workspace { workspace_path } => {
                TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(workspace_path))
            }
            TerminalSurfaceOwnerV1::Session {
                workspace_path,
                session_id,
            } => TerminalSurfaceOwner::session(WorkspaceIdentity::new(workspace_path), session_id),
        }
        .map_err(|error| format!("invalid Terminal Surface owner: {error:?}"))
    }
}

#[cfg(test)]
#[path = "terminal_test.rs"]
mod terminal_tests;
