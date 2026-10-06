use crate::common::operation_context::Cancellation;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct CancelAfter {
    pub remaining: AtomicUsize,
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        self.remaining.fetch_sub(1, Ordering::SeqCst) == 0
    }
}
