#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubscriptionError {
    InvalidId,
    AlreadyExists,
    StreamEnded,
    UnknownTarget,
    VersionExhausted,
    SnapshotRequired,
    EncodingFailed,
}

impl std::fmt::Display for SubscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SubscriptionError {}
