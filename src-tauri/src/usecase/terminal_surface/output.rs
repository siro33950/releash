use std::sync::Arc;

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
    fn initialize(
        &self,
        session_key: &str,
        workspace_path: &str,
        session_id: Option<&str>,
        runtime_generation: u64,
        latest_sequence: u64,
    );
    /// Returns whether a subscription still owns the input attachment.
    fn remove(&self, session_key: &str, runtime_generation: u64) -> bool;
    fn publish(&self, event: TerminalSurfaceOutputEvent);
}

pub trait TerminalSurfaceEventSink: Send + Sync {
    /// Returns whether a subscription still owns the input attachment.
    fn remove(&self, _runtime_generation: u64) -> bool {
        false
    }
    fn wait_output(&self, _session_key: &str) {}
    fn release_output(&self, _session_key: &str) {}

    fn publish(&self, event: TerminalSurfaceOutputEvent);
}

pub trait TerminalSurfaceOutputControl: Send + Sync {
    fn set_state_sink(&self, sink: Arc<dyn TerminalSurfaceStateSink>);
    fn initialize(
        &self,
        session_key: &str,
        workspace_path: &str,
        session_id: Option<&str>,
        runtime_generation: u64,
        latest_sequence: u64,
    );
    fn subscribe_output(&self, session_key: &str, client: &str, units: usize);
    fn unsubscribe_output(&self, session_key: &str, client: &str);
    fn processed_output(&self, session_key: &str, client: &str, units: usize);
}
