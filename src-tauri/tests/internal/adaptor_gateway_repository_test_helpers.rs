use connectrpc::ErrorCode;
use releash_lib::test_support::integration::platform::Cancellation;
use releash_lib::test_support::integration::platform::OperationContext;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub fn assert_stops_at_each_checkpoint<
    T,
    E: releash_lib::test_support::integration::transport::ConnectFailure + std::fmt::Debug,
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
    let expired = releash_lib::test_support::integration::platform::sync_scope(
        OperationContext::default().with_deadline(
            releash_lib::test_support::integration::platform::Deadline::new(
                std::time::Instant::now(),
            ),
        ),
        &mut operation,
    );
    assert!(
        matches!(expired, Err(ref error) if error.connect_code() == ErrorCode::DeadlineExceeded)
    );
    let baseline = Arc::new(CancelAt {
        checks: AtomicUsize::new(0),
        stop_at: usize::MAX,
    });
    releash_lib::test_support::integration::platform::sync_scope(
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
        let result = releash_lib::test_support::integration::platform::sync_scope(
            OperationContext::new(None, cancellation.clone()),
            &mut operation,
        );
        assert!(
            matches!(result, Err(ref error) if error.connect_code() == ErrorCode::Canceled),
            "checkpoint {stop_at}"
        );
        assert_eq!(
            cancellation.checks.load(Ordering::SeqCst),
            stop_at + 1,
            "continued after checkpoint {stop_at}"
        );
    }
}
