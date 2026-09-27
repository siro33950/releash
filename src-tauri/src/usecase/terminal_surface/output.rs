use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalSurfaceOutputSummary {
    pub session_key: String,
    pub owner: crate::domain::terminal_surface::TerminalSurfaceOwner,
    pub runtime_generation: u64,
    pub latest_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalSurfaceOutputEvent {
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
}

impl TerminalSurfaceOutputEvent {
    pub fn session_key(&self) -> &str {
        match self {
            Self::Output { session_key, .. }
            | Self::Resize { session_key, .. }
            | Self::Exit { session_key, .. } => session_key,
        }
    }
}

pub trait TerminalSurfaceStateSink: Send + Sync {
    fn initialize(&self, surface: &TerminalSurfaceOutputSummary);
    /// Returns whether a subscription still owns the input attachment.
    fn remove(&self, surface: &TerminalSurfaceOutputSummary) -> bool;
    fn publish(&self, event: TerminalSurfaceOutputEvent);
}

pub trait TerminalSurfaceEventSink: Send + Sync {
    fn initialize(&self, _surface: &TerminalSurfaceOutputSummary) {}
    /// Returns whether a subscription still owns the input attachment.
    fn remove(&self, _surface: &TerminalSurfaceOutputSummary) -> bool {
        false
    }
    fn wait_output(&self, _session_key: &str) {}
    fn release_output(&self, _session_key: &str) {}

    fn publish(&self, event: TerminalSurfaceOutputEvent);
}

pub trait TerminalSurfaceOutputControl: Send + Sync {
    fn set_state_sink(&self, sink: Arc<dyn TerminalSurfaceStateSink>);
    fn subscribe_output(&self, session_key: &str, client: &str, units: usize);
    fn unsubscribe_output(&self, session_key: &str, client: &str);
    fn processed_output(&self, session_key: &str, client: &str, units: usize);
}
