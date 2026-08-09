use crate::domain::agent_session::aggregates::ManagedPtyPresence;
use crate::domain::terminal_surface::{
    TerminalActivity, TerminalProcessLaunch, TerminalSurfaceOwner,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentTerminalGatewayError {
    Unavailable,
}

pub(crate) trait ProviderAgentTerminalGateway: Send + Sync {
    fn spawn(
        &self,
        owner: TerminalSurfaceOwner,
        worktree_path: &str,
        process: TerminalProcessLaunch,
        rows: u16,
        cols: u16,
    ) -> Result<(), ProviderAgentTerminalGatewayError>;

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

    /// owner の surface の出力recencyに基づく実行状態分類。
    /// surface 不在・終了済みは Idle。
    fn session_activity(&self, owner: &TerminalSurfaceOwner) -> TerminalActivity;

    /// session_key が Session 所有の surface を指す場合のみ worktree path を返す。
    fn session_worktree_path(&self, session_key: &str) -> Option<String>;
}
