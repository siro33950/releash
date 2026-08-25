use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::terminal_surface::entities::{TerminalSurface, TerminalSurfaceSummary};
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
use crate::usecase::terminal_surface::spawn_usecase::GetOrSpawnTerminalOutcome;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalLaunchPerformanceSampleV1 {
    pub phase: String,
    pub duration_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalPerformanceSwitchesV1 {
    pub disable_output_flow_control: bool,
    pub disable_terminal_journal: bool,
    pub disable_renderer_write_serialization: bool,
    pub disable_webgl_renderer: bool,
    pub disable_terminal_websocket: bool,
}

impl From<crate::other::performance_switches::TerminalPerformanceSwitches>
    for TerminalPerformanceSwitchesV1
{
    fn from(switches: crate::other::performance_switches::TerminalPerformanceSwitches) -> Self {
        Self {
            disable_output_flow_control: switches.disable_output_flow_control,
            disable_terminal_journal: switches.disable_terminal_journal,
            disable_renderer_write_serialization: switches.disable_renderer_write_serialization,
            disable_webgl_renderer: switches.disable_webgl_renderer,
            disable_terminal_websocket: switches.disable_terminal_websocket,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInputPerformanceSampleV1 {
    pub sequence: u64,
    pub on_data_to_command_ingress_ms: f64,
    pub command_ingress_to_admission_ms: f64,
    pub admission_to_writer_enqueue_ms: f64,
    pub writer_enqueue_to_output_read_ms: f64,
    pub output_read_to_model_apply_ms: f64,
    pub model_apply_to_event_publish_ms: f64,
    pub event_published_at_unix_ms: f64,
}

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

/// terminal WebSocket認証に使うsubprotocolのprefix。クライアントは
/// `{prefix}{bearer_token}` を Sec-WebSocket-Protocol として送る。
pub const TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX: &str = "releash-bearer.";

/// terminal WebSocket transportのroute path。
pub const TERMINAL_WS_PATH: &str = "/v1/terminal";

/// frontendがterminal streamをWebSocket購読するための接続情報。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStreamEndpointV1 {
    pub url: String,
    pub auth_subprotocol: String,
}

/// replay全量を含まないTerminal Surfaceの読み取り応答。
/// frontendのattach前照会はsession identityと生存状態だけを必要とする。
#[derive(Clone, Debug, Serialize)]
pub struct TerminalSurfaceSummaryV1 {
    pub session_key: String,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
}

impl From<TerminalSurfaceSummary> for TerminalSurfaceSummaryV1 {
    fn from(surface: TerminalSurfaceSummary) -> Self {
        Self {
            session_key: surface.session_key,
            is_exited: surface.process_state.is_exited(),
            exit_code: surface.process_state.exit_code(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalSurfaceStreamItemV1 {
    Snapshot {
        surface: TerminalSurfaceV1,
    },
    Output {
        session_key: String,
        data: Arc<str>,
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
    InputUnavailable {
        session_key: String,
        message: String,
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
            TerminalSurfaceStreamItem::InputUnavailable { session_key, cause } => {
                log::error!(
                    "Terminal input unavailable: operation=write_terminal_surface code=PTY_ERROR cause={}",
                    cause.internal_cause()
                );
                Self::InputUnavailable {
                    session_key,
                    message: "Terminal input could not be sent. Try again.".to_string(),
                }
            }
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
        match value {
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalWsRequestV1 {
    AttachSurface {
        id: String,
        owner: TerminalSurfaceOwnerV1,
        #[serde(default)]
        attachment_id: Option<String>,
    },
}

/// attach確立後にクライアントから届く要求。write/ackはterminalのhot pathで、
/// Tauri invokeを介さないことがWS transportの目的そのもの。
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalWsAttachedRequestV1 {
    Write {
        owner: TerminalSurfaceOwnerV1,
        attachment_id: String,
        sequence: u64,
        data: String,
        #[serde(default)]
        client_started_at_unix_ms: Option<f64>,
    },
    Ack {
        attachment_id: String,
        sequence: u64,
    },
    Resize {
        owner: TerminalSurfaceOwnerV1,
        rows: u16,
        cols: u16,
    },
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TerminalWsResponseV1 {
    Attached {
        id: String,
    },
    Error {
        id: String,
        error: TerminalWsErrorV1,
    },
    Event {
        id: String,
        item: TerminalSurfaceStreamItemV1,
    },
}

#[derive(Serialize)]
pub struct TerminalWsErrorV1 {
    pub code: &'static str,
    pub message: String,
}

#[cfg(test)]
#[path = "terminal_test.rs"]
mod terminal_tests;
