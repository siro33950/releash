#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtyEvictReason {
    Idle,
    CapExceeded,
}
