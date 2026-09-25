use crate::domain::operation_context::Cancellation;

impl Cancellation for tokio_util::sync::CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }
}
