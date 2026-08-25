use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::entities::{
    TerminalSurface, TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError,
    TerminalSurfaceSummary,
};
use super::{TerminalProcessLaunch, TerminalSurfaceCheckpoint};

pub struct TerminalRuntimeSpawnRequest {
    pub runtime_generation: u64,
    pub session_key: String,
    pub rows: u16,
    pub cols: u16,
    pub cwd: Option<String>,
    pub process: Option<TerminalProcessLaunch>,
    pub initial_terminal_surface: Option<TerminalSurfaceCheckpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceGatewayError {
    message: String,
}

impl TerminalSurfaceGatewayError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TerminalSurfaceGatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TerminalSurfaceGatewayError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalSurfaceInputUnavailableCause {
    StaleAttachment,
    PendingCapacityExceeded,
    RuntimeWriteFailed(String),
}

impl TerminalSurfaceInputUnavailableCause {
    pub fn internal_cause(&self) -> &str {
        match self {
            Self::StaleAttachment => "Terminal input attachment is no longer active",
            Self::PendingCapacityExceeded => "Terminal input reorder buffer is full",
            Self::RuntimeWriteFailed(cause) => cause,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalSurfaceEvent {
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
        runtime_generation: u64,
        exit_code: Option<i32>,
        sequence: u64,
    },
    InputUnavailable {
        session_key: String,
        cause: TerminalSurfaceInputUnavailableCause,
    },
}

impl TerminalSurfaceEvent {
    pub fn session_key(&self) -> &str {
        match self {
            Self::Output { session_key, .. }
            | Self::Resize { session_key, .. }
            | Self::Exit { session_key, .. }
            | Self::InputUnavailable { session_key, .. } => session_key,
        }
    }
}

pub trait TerminalSurfaceEventSink: Send + Sync {
    fn publish(&self, event: TerminalSurfaceEvent);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalSurfaceEventReceiveError {
    Lagged(u64),
    Closed,
}

pub trait TerminalSurfaceEventSubscription: Send {
    fn recv(
        &mut self,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TerminalSurfaceEvent, TerminalSurfaceEventReceiveError>>
                + Send
                + '_,
        >,
    >;
}

pub trait TerminalSurfaceEventCancellation: Send + Sync {
    fn cancel(&self);
}

pub struct TerminalSurfaceEventStream {
    pub subscription: Box<dyn TerminalSurfaceEventSubscription>,
    pub cancellation: Arc<dyn TerminalSurfaceEventCancellation>,
}

pub trait TerminalSurfaceEventSource: Send + Sync {
    fn subscribe(&self) -> TerminalSurfaceEventStream;

    fn subscribe_owner(
        &self,
        _session_key: &str,
        _attachment_id: &str,
    ) -> TerminalSurfaceEventStream {
        self.subscribe()
    }

    fn acknowledge_owner_output(&self, _session_key: &str, _attachment_id: &str, _sequence: u64) {}
}

pub trait TerminalSurfaceRepository {
    fn find_summary_by_session_key(&self, session_key: &str) -> Option<TerminalSurfaceSummary>;
    fn list_summaries(&self) -> Vec<TerminalSurfaceSummary>;
}

pub trait TerminalSurfaceGateway: TerminalSurfaceRepository {
    fn next_runtime_generation(&self) -> u64;
    fn load_terminal_checkpoint(
        &self,
        _session_key: &str,
    ) -> Result<Option<TerminalSurfaceCheckpoint>, TerminalSurfaceGatewayError> {
        Ok(None)
    }
    fn delete_terminal_checkpoint(
        &self,
        _session_key: &str,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }
    fn spawn_runtime(
        &self,
        request: TerminalRuntimeSpawnRequest,
    ) -> Result<(), TerminalSurfaceGatewayError>;
    fn insert_surface(&self, surface: TerminalSurface);
    fn start_output_reader(
        &self,
        runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError>;
    fn snapshot(&self, runtime_generation: u64) -> Option<TerminalSurface>;
    fn select_kill_targets_by_worktree(&self, worktree_path: &str) -> Vec<u64>;
    fn remove_surface(&self, runtime_generation: u64) -> Option<TerminalSurface>;
    fn reserve_spawn_slot(
        &self,
        session_key: &str,
        worktree_path: Option<&str>,
    ) -> Result<TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError>;
    fn wait_for_spawn_resolution(&self, session_key: &str) -> Option<TerminalSurfaceSummary> {
        self.find_summary_by_session_key(session_key)
    }
    fn complete_spawn_slot(&self, reservation: &TerminalSurfaceSpawnReservation);
    fn rollback_spawn_slot(&self, reservation: &TerminalSurfaceSpawnReservation);
    fn activate_input_attachment(&self, session_key: &str, attachment_id: &str);
    fn deactivate_input_attachment(&self, session_key: &str, attachment_id: &str);
    fn write_attached(
        &self,
        session_key: &str,
        attachment_id: &str,
        sequence: u64,
        data: &str,
    ) -> Result<(), TerminalSurfaceGatewayError>;
    fn write(&self, session_key: &str, data: &str) -> Result<(), TerminalSurfaceGatewayError>;
    fn resize(
        &self,
        session_key: &str,
        rows: u16,
        cols: u16,
    ) -> Result<(), TerminalSurfaceGatewayError>;
    fn request_runtime_stop(
        &self,
        runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError>;
    fn wait_runtime_output_drain(
        &self,
        _runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }
    fn remove_runtime(&self, runtime_generation: u64);
    fn flush_checkpoints(&self) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }
}
