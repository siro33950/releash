#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCarryState {
    Resumed,
    Reinjected,
    Failed,
}

impl ContextCarryState {
    pub fn is_failed(self) -> bool {
        self == Self::Failed
    }

    pub fn is_reinjected(self) -> bool {
        self == Self::Reinjected
    }
}
