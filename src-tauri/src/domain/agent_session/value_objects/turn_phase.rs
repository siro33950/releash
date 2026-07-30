/// Canonical phase of the current turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPhase {
    Idle,
    Streaming,
    WaitingPermission,
}

impl TurnPhase {
    pub fn is_idle(self) -> bool {
        self == Self::Idle
    }

    pub fn is_streaming(self) -> bool {
        self == Self::Streaming
    }

    pub fn has_pending_permission(self) -> bool {
        self == Self::WaitingPermission
    }

    pub fn workflow_execution_is_running(self) -> bool {
        matches!(self, Self::Streaming | Self::WaitingPermission)
    }

    #[cfg(test)]
    pub(crate) fn has_active_turn(self) -> bool {
        !self.is_idle()
    }

    pub fn is_watchdog_live(self) -> bool {
        self.workflow_execution_is_running()
    }
}
