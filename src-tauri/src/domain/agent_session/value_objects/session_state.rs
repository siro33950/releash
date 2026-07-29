/// Canonical lifecycle projection for an agent session.
///
/// `Done` and `Error` describe the most recently observed terminal turn. They
/// do not close the session and therefore are never sufficient, on their own,
/// to decide whether new work is admissible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Active,
    Idle,
    Done,
    Error,
    Closed,
    Archived,
}

impl SessionState {
    pub fn is_closed(self) -> bool {
        matches!(self, Self::Closed | Self::Archived)
    }

    pub fn is_open(self) -> bool {
        !self.is_closed()
    }

    pub fn is_archived(self) -> bool {
        self == Self::Archived
    }

    #[cfg(test)]
    pub fn is_closed_history(self) -> bool {
        self == Self::Closed
    }

    pub fn permits_legacy_queue_start(self) -> bool {
        matches!(self, Self::Idle | Self::Done | Self::Error)
    }

    pub fn fences_context_restore(self) -> bool {
        matches!(self, Self::Error | Self::Closed | Self::Archived)
    }

    pub fn normalizes_turn_phase_to_idle(self) -> bool {
        matches!(self, Self::Idle | Self::Closed | Self::Archived)
    }

    pub fn is_error(self) -> bool {
        self == Self::Error
    }

    pub fn retains_error_reason(self) -> bool {
        self.is_error()
    }
}
