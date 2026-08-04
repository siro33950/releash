#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalProcessState {
    Running,
    Exited { exit_code: Option<i32> },
}

impl TerminalProcessState {
    pub fn is_exited(&self) -> bool {
        matches!(self, Self::Exited { .. })
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Self::Running => None,
            Self::Exited { exit_code } => *exit_code,
        }
    }
}
