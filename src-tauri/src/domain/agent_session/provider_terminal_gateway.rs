use crate::domain::agent_session::aggregates::ManagedPtyPresence;
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentTerminalGatewayError {
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderAgentTerminalSpawnError {
    PerWorktreeCap { worktree_path: String },
    TotalCap,
    OwnerConflict,
    PtySpawn { error: String },
    OtherSpawnFailure { error: String },
}

impl std::fmt::Display for ProviderAgentTerminalSpawnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PerWorktreeCap { worktree_path } => {
                write!(
                    formatter,
                    "kind=per_worktree_cap worktree_path={worktree_path}"
                )
            }
            Self::TotalCap => formatter.write_str("kind=total_cap"),
            Self::OwnerConflict => formatter.write_str("kind=owner_conflict"),
            Self::PtySpawn { error } => write!(formatter, "kind=pty_spawn error={error}"),
            Self::OtherSpawnFailure { error } => {
                write!(formatter, "kind=other_spawn_failure error={error}")
            }
        }
    }
}

impl std::error::Error for ProviderAgentTerminalSpawnError {}

pub(crate) trait ProviderAgentTerminalGateway: Send + Sync {
    fn spawn(
        &self,
        owner: TerminalSurfaceOwner,
        worktree_path: &str,
        process: TerminalProcessLaunch,
        rows: u16,
        cols: u16,
    ) -> Result<(), ProviderAgentTerminalSpawnError>;

    fn presence(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<ManagedPtyPresence, ProviderAgentTerminalGatewayError>;

    fn stop_preserving_checkpoint(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError>;

    fn delete(&self, owner: &TerminalSurfaceOwner)
        -> Result<(), ProviderAgentTerminalGatewayError>;

    fn is_current_runtime_generation(
        &self,
        owner: &TerminalSurfaceOwner,
        runtime_generation: u64,
    ) -> Result<bool, ProviderAgentTerminalGatewayError>;
}

pub(crate) trait ProviderAgentTerminalInputGateway: Send + Sync {
    fn write(
        &self,
        owner: &TerminalSurfaceOwner,
        input: &str,
    ) -> Result<(), ProviderAgentTerminalGatewayError>;
}

pub(crate) trait ProviderAgentTerminalObservationGateway: Send + Sync {
    fn owner_for_runtime_generation(
        &self,
        session_key: &str,
        runtime_generation: u64,
    ) -> Option<TerminalSurfaceOwner>;

    fn exited_session_owners(&self) -> Vec<(u64, TerminalSurfaceOwner, Option<i32>)>;

    /// owner の surface summary が保持する exit_code。surface 不在・実行中は None。
    fn session_exit_code(&self, owner: &TerminalSurfaceOwner) -> Option<i32>;
}
