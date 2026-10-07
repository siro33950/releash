pub(crate) mod retry {
    pub use releashd::desktop_api::{RetryBackoff, RetryLimiter};
    pub const DESKTOP_POLL: RetryBackoff = RetryBackoff::new(
        std::time::Duration::from_millis(20),
        1.0,
        std::time::Duration::from_millis(20),
    );
}
