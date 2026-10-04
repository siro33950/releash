use crate::common::operation_context::{self, OperationStopped};
use std::future::Future;
use std::time::Duration;

const JITTER_SPREAD: f64 = 0.2;

#[derive(Debug, Clone, Copy)]
pub struct RetryBackoff {
    initial: Duration,
    multiplier: f64,
    maximum: Duration,
}

impl RetryBackoff {
    pub const ITEM: Self = Self::new(Duration::from_millis(5), 2.0, Duration::from_secs(1000));
    pub const RECOVERY: Self = Self::new(Duration::from_millis(800), 2.0, Duration::from_secs(30));
    pub const CONFLICT: Self = Self::new(Duration::from_millis(10), 5.0, Duration::from_secs(1));
    pub const POLL: Self = Self::new(Duration::from_millis(10), 1.0, Duration::from_millis(10));
    #[cfg(feature = "desktop")]
    pub const DESKTOP_POLL: Self =
        Self::new(Duration::from_millis(20), 1.0, Duration::from_millis(20));

    pub const fn new(initial: Duration, multiplier: f64, maximum: Duration) -> Self {
        assert!(!initial.is_zero());
        assert!(multiplier >= 1.0 && multiplier < f64::INFINITY);
        assert!(!maximum.is_zero());
        Self {
            initial,
            multiplier,
            maximum,
        }
    }

    pub fn delay(self, failure_count: u64, jitter: f64) -> Duration {
        assert!(jitter.is_finite() && jitter >= 0.0);
        let exponent = failure_count.saturating_sub(1).min(i32::MAX as u64) as i32;
        Duration::from_secs_f64(
            (self.initial.as_secs_f64() * self.multiplier.powi(exponent))
                .min(self.maximum.as_secs_f64())
                * jitter,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetryBucket {
    tokens: f64,
    updated_at: Duration,
}

impl RetryBucket {
    pub fn new(now: Duration) -> Self {
        Self {
            tokens: 100.0,
            updated_at: now,
        }
    }

    pub fn acquire(&mut self, now: Duration) -> Duration {
        self.tokens =
            (self.tokens + now.saturating_sub(self.updated_at).as_secs_f64() * 10.0).min(100.0);
        self.updated_at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Duration::ZERO
        } else {
            Duration::from_secs_f64((1.0 - self.tokens) / 10.0)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptProgress {
    Continue,
    Reload,
}

pub struct RetryLimiter {
    bucket: std::sync::Mutex<RetryBucket>,
    origin: tokio::time::Instant,
    jitter: fn() -> f64,
}

impl RetryLimiter {
    fn backoff_delay(&self, policy: RetryBackoff, failures: u64, spread: f64) -> Duration {
        policy.delay(failures, 1.0 + (self.jitter)() * spread)
    }

    fn token_wait(&self) -> Duration {
        self.bucket
            .lock()
            .expect("retry bucket")
            .acquire(self.origin.elapsed())
    }

    fn token_waits(&self) -> impl Iterator<Item = Duration> + '_ {
        std::iter::from_fn(|| {
            let delay = self.token_wait();
            (!delay.is_zero()).then_some(delay)
        })
    }

    pub fn try_acquire(&self) -> bool {
        self.token_wait().is_zero()
    }

    pub fn wait_sync(
        &self,
        context: &operation_context::OperationContext,
        policy: RetryBackoff,
        failures: u64,
    ) -> Result<(), OperationStopped> {
        operation_context::sleep(context, self.backoff_delay(policy, failures, JITTER_SPREAD))?;
        for delay in self.token_waits() {
            operation_context::sleep(context, delay)?;
        }
        context.check(std::time::Instant::now())
    }

    pub fn new() -> Self {
        Self::with_jitter(jitter_fraction)
    }

    #[cfg(test)]
    pub fn deterministic() -> Self {
        Self::with_jitter(|| 0.0)
    }

    fn with_jitter(jitter: fn() -> f64) -> Self {
        Self {
            bucket: std::sync::Mutex::new(RetryBucket::new(Duration::ZERO)),
            origin: tokio::time::Instant::now(),
            jitter,
        }
    }

    pub async fn wait(&self, policy: RetryBackoff, failures: u64) -> Result<(), OperationStopped> {
        self.wait_with_spread(policy, failures, JITTER_SPREAD).await
    }

    pub async fn wait_with_spread(
        &self,
        policy: RetryBackoff,
        failures: u64,
        spread: f64,
    ) -> Result<(), OperationStopped> {
        self.pause(self.backoff_delay(policy, failures, spread))
            .await?;
        self.acquire().await
    }

    async fn pause(&self, delay: Duration) -> Result<(), OperationStopped> {
        operation_context::wait(&operation_context::current(), tokio::time::sleep(delay)).await
    }

    pub async fn acquire(&self) -> Result<(), OperationStopped> {
        operation_context::check()?;
        for delay in self.token_waits() {
            self.pause(delay).await?;
        }
        operation_context::check()
    }
}

impl Default for RetryLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
fn jitter() -> f64 {
    1.0 + jitter_fraction() * JITTER_SPREAD
}

fn jitter_fraction() -> f64 {
    let random = uuid::Uuid::new_v4().as_u128() as u32;
    (random as f64 / u32::MAX as f64 - 0.5) * 2.0
}

pub async fn attempts<T, E, F, Fut>(
    policy: RetryBackoff,
    limiter: &RetryLimiter,
    decide: impl Fn(&E) -> Option<AttemptProgress>,
    operation: F,
) -> Result<T, E>
where
    F: FnMut(AttemptProgress) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    attempts_with_pushback(
        policy,
        limiter,
        |error| decide(error).map(|progress| (progress, None)),
        operation,
    )
    .await
}

pub async fn attempts_with_pushback<T, E, F, Fut>(
    policy: RetryBackoff,
    limiter: &RetryLimiter,
    decide: impl Fn(&E) -> Option<(AttemptProgress, Option<Duration>)>,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut(AttemptProgress) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut progress = AttemptProgress::Continue;
    let mut failures = 0u64;
    loop {
        let error = match operation(progress).await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let Some((next, pushback)) = decide(&error) else {
            return Err(error);
        };
        failures = failures.saturating_add(1);
        let policy = if next == AttemptProgress::Reload {
            RetryBackoff::CONFLICT
        } else {
            policy
        };
        if let Some(delay) = pushback {
            if operation_context::current()
                .remaining(std::time::Instant::now())
                .is_some_and(|remaining| delay >= remaining)
                || !limiter.try_acquire()
                || limiter.pause(delay).await.is_err()
            {
                return Err(error);
            }
        } else if limiter.wait(policy, failures).await.is_err() {
            return Err(error);
        }
        progress = next;
    }
}

pub async fn bounded<T, E>(
    limit: Duration,
    expired: impl FnOnce() -> E,
    operation: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    match tokio::time::timeout(limit, operation).await {
        Ok(result) => result,
        Err(_) => Err(expired()),
    }
}

#[cfg(test)]
#[path = "retry_test.rs"]
mod retry_tests;
