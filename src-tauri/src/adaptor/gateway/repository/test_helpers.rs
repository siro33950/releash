use crate::domain::failure::FailureKind;
use crate::domain::operation_context::{Cancellation, OperationContext};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub(crate) fn assert_stops_at_each_checkpoint<
    T,
    E: crate::domain::failure::ClassifiedFailure + std::fmt::Debug,
>(
    mut operation: impl FnMut() -> Result<T, E>,
) {
    struct CancelAt {
        checks: AtomicUsize,
        stop_at: usize,
    }
    impl Cancellation for CancelAt {
        fn is_cancelled(&self) -> bool {
            self.checks.fetch_add(1, Ordering::SeqCst) >= self.stop_at
        }
    }
    let expired = crate::other::operation_context::sync_scope(
        OperationContext::default().with_deadline(crate::domain::operation_context::Deadline::new(
            std::time::Instant::now(),
        )),
        &mut operation,
    );
    assert!(matches!(expired, Err(ref error) if error.failure_kind() == FailureKind::Expired));
    let baseline = Arc::new(CancelAt {
        checks: AtomicUsize::new(0),
        stop_at: usize::MAX,
    });
    crate::other::operation_context::sync_scope(
        OperationContext::new(None, baseline.clone()),
        &mut operation,
    )
    .unwrap();
    let checkpoints = baseline.checks.load(Ordering::SeqCst);
    assert!(checkpoints > 0);
    for stop_at in 0..checkpoints {
        let cancellation = Arc::new(CancelAt {
            checks: AtomicUsize::new(0),
            stop_at,
        });
        let result = crate::other::operation_context::sync_scope(
            OperationContext::new(None, cancellation.clone()),
            &mut operation,
        );
        assert!(
            matches!(result, Err(ref error) if error.failure_kind() == FailureKind::Cancelled),
            "checkpoint {stop_at}"
        );
        assert_eq!(
            cancellation.checks.load(Ordering::SeqCst),
            stop_at + 1,
            "continued after checkpoint {stop_at}"
        );
    }
}
