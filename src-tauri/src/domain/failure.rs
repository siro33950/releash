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
