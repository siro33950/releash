use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline(Instant);

impl Deadline {
    pub fn new(at: Instant) -> Self {
        Self(at)
    }
    pub fn minimum(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }
    pub fn is_expired(self, now: Instant) -> bool {
        now >= self.0
    }
    pub fn remaining(self, now: Instant) -> Duration {
        self.0.saturating_duration_since(now)
    }
}

pub trait Cancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

#[derive(Clone, Default)]
pub struct OperationContext {
    deadline: Option<Deadline>,
    cancellation: Option<Arc<dyn Cancellation>>,
}

impl OperationContext {
    pub fn new(deadline: Option<Deadline>, cancellation: Arc<dyn Cancellation>) -> Self {
        Self {
            deadline,
            cancellation: Some(cancellation),
        }
    }
    pub fn with_deadline(&self, deadline: Deadline) -> Self {
        Self {
            deadline: Some(
                self.deadline
                    .map_or(deadline, |parent| parent.minimum(deadline)),
            ),
            cancellation: self.cancellation.clone(),
        }
    }
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.deadline.map(|deadline| deadline.remaining(now))
    }
    pub fn check(&self, now: Instant) -> Result<(), OperationStopped> {
        if self
            .deadline
            .is_some_and(|deadline| deadline.is_expired(now))
        {
            Err(OperationStopped::Expired)
        } else if self
            .cancellation
            .as_ref()
            .is_some_and(|cancellation| cancellation.is_cancelled())
        {
            Err(OperationStopped::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationStopped {
    Expired,
    Cancelled,
}

impl std::fmt::Display for OperationStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Expired => "Operation deadline exceeded",
            Self::Cancelled => "Operation cancelled",
        })
    }
}
impl std::error::Error for OperationStopped {}

use std::cell::RefCell;
use std::future::Future;

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

pub async fn ingress<T>(
    deadline: Option<Instant>,
    operation: impl Future<Output = T>,
) -> Result<T, OperationStopped> {
    let cancellation = tokio_util::sync::CancellationToken::new();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let context = OperationContext::new(deadline.map(Deadline::new), Arc::new(cancellation));
    let result = scope(context.clone(), operation).await;
    context.check(Instant::now())?;
    Ok(result)
}

impl Cancellation for tokio_util::sync::CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }
}

pub async fn spawned<T: Send + 'static>(
    operation: impl Future<Output = T> + Send + 'static,
) -> Result<Result<T, OperationStopped>, tokio::task::JoinError> {
    let cancellation = tokio_util::sync::CancellationToken::new();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    tokio::spawn(scope(current(), async move {
        cancellation
            .run_until_cancelled(operation)
            .await
            .ok_or(OperationStopped::Cancelled)
    }))
    .await
}

pub fn before<T>(operation: impl FnOnce() -> T) -> Result<T, OperationStopped> {
    check()?;
    Ok(operation())
}

pub fn checked<T>(operation: impl FnOnce() -> T) -> Result<T, OperationStopped> {
    let result = before(operation)?;
    check()?;
    Ok(result)
}

pub fn poll<T>(mut attempt: impl FnMut() -> Option<T>) -> Result<T, OperationStopped> {
    loop {
        if let Some(value) = before(&mut attempt)? {
            return Ok(value);
        }
    }
}

pub fn timeout_sync<T>(duration: Duration, operation: impl FnOnce() -> T) -> T {
    sync_scope(with_timeout(duration), operation)
}

pub async fn timeout<T>(
    duration: Duration,
    operation: impl Future<Output = T>,
) -> Result<T, OperationStopped> {
    let context = with_timeout(duration);
    scope(context.clone(), wait(&context, operation)).await
}

#[cfg(test)]
#[path = "operation_context_test.rs"]
mod operation_context_tests;
