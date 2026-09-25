use std::cell::RefCell;
use std::future::Future;
use std::time::{Duration, Instant};

use crate::domain::operation_context::{Deadline, OperationContext, OperationStopped};

tokio::task_local! { static ASYNC_CONTEXT: OperationContext; }
thread_local! { static SYNC_CONTEXT: RefCell<OperationContext> = RefCell::default(); }

pub fn current() -> OperationContext {
    ASYNC_CONTEXT
        .try_with(Clone::clone)
        .unwrap_or_else(|_| SYNC_CONTEXT.with_borrow(Clone::clone))
}

pub async fn scope<T>(context: OperationContext, future: impl Future<Output = T>) -> T {
    ASYNC_CONTEXT.scope(context, future).await
}

pub fn sync_scope<T>(context: OperationContext, operation: impl FnOnce() -> T) -> T {
    struct Restore(Option<OperationContext>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SYNC_CONTEXT.set(self.0.take().unwrap());
        }
    }
    let _restore = Restore(Some(SYNC_CONTEXT.replace(context)));
    operation()
}

pub fn spawn_blocking<F, T>(operation: F) -> tokio::task::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let context = current();
    tokio::task::spawn_blocking(move || sync_scope(context, operation))
}

pub fn check() -> Result<(), OperationStopped> {
    current().check(Instant::now())
}

pub fn with_timeout(duration: Duration) -> OperationContext {
    current().with_deadline(Deadline::new(Instant::now() + duration))
}

pub async fn wait<T>(
    context: &OperationContext,
    future: impl Future<Output = T>,
) -> Result<T, OperationStopped> {
    tokio::pin!(future);
    loop {
        context.check(Instant::now())?;
        let interval = context
            .remaining(Instant::now())
            .unwrap_or(Duration::from_millis(10))
            .min(Duration::from_millis(10));
        tokio::select! {
            value = &mut future => { context.check(Instant::now())?; return Ok(value); }
            _ = tokio::time::sleep(interval) => {}
        }
    }
}

pub fn sleep(context: &OperationContext, duration: Duration) -> Result<(), OperationStopped> {
    let until = Instant::now() + duration;
    loop {
        context.check(Instant::now())?;
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        std::thread::sleep(
            remaining
                .min(context.remaining(Instant::now()).unwrap_or(remaining))
                .min(Duration::from_millis(10)),
        );
    }
}

#[cfg(test)]
#[path = "operation_context_test.rs"]
mod operation_context_tests;
