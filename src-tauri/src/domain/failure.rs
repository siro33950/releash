#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Temporary,
    RestartRequired,
    StateRequired,
    InvalidInput,
    Expired,
    Missing,
    AlreadyPresent,
    Permission,
    Capacity,
    Unsupported,
    Internal,
    Corrupt,
    Cancelled,
    Unknown,
    OutsideRange,
    AuthenticationRequired,
}

pub trait ClassifiedFailure {
    fn failure_kind(&self) -> FailureKind;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAction {
    Stop,
    Retry,
    Restart,
}

impl FailureKind {
    pub fn retry_action(self) -> RetryAction {
        match self {
            Self::Temporary => RetryAction::Retry,
            Self::RestartRequired => RetryAction::Restart,
            _ => RetryAction::Stop,
        }
    }

    pub fn requires_attention(self) -> bool {
        self.retry_action() == RetryAction::Stop && self != Self::Cancelled
    }
}

#[cfg(test)]
#[path = "failure_test.rs"]
mod failure_tests;

#[derive(Default)]
pub struct TargetFailures(std::collections::HashMap<(String, String), FailureKind>);
impl TargetFailures {
    pub fn observe(&mut self, operation: &str, target: &str, kind: FailureKind) -> bool {
        if kind.requires_attention() {
            self.0.insert((operation.into(), target.into()), kind) != Some(kind)
        } else {
            self.clear(operation, target)
        }
    }
    pub fn clear(&mut self, operation: &str, target: &str) -> bool {
        self.0.remove(&(operation.into(), target.into())).is_some()
    }
    pub fn requires_attention(&self, operation: &str, target: &str) -> bool {
        self.0.contains_key(&(operation.into(), target.into()))
    }
}

pub trait BackgroundFailures: Send {
    fn observe(&mut self, target: &str, kind: FailureKind) -> bool;
    fn clear(&mut self, target: &str) -> bool;
    fn requires_attention(&self, target: &str) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TechnicalFailure {
    pub kind: FailureKind,
    pub message: String,
}
impl std::fmt::Display for TechnicalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl ClassifiedFailure for TechnicalFailure {
    fn failure_kind(&self) -> FailureKind {
        self.kind
    }
}
