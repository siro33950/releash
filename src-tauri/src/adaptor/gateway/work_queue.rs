use crate::domain::failure::FailureKind;
use crate::usecase::work_queue::{Attempt, WorkFailure, WorkQueueRuntime};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct TokioWorkQueueRuntime {
    origin: std::time::Instant,
    handle: tokio::runtime::Handle,
}
impl Default for TokioWorkQueueRuntime {
    fn default() -> Self {
        #[cfg(test)]
        let handle = {
            static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> =
                std::sync::OnceLock::new();
            RUNTIME
                .get_or_init(|| {
                    tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .enable_all()
                        .build()
                        .expect("test work queue runtime")
                })
                .handle()
                .clone()
        };
        #[cfg(not(test))]
        let handle = tokio::runtime::Handle::current();
        Self {
            origin: std::time::Instant::now(),
            handle,
        }
    }
}
#[async_trait::async_trait]
impl WorkQueueRuntime for TokioWorkQueueRuntime {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
    fn timestamp_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
    fn jitter(&self) -> f64 {
        let random = uuid::Uuid::new_v4().as_u128() as u32;
        1.0 + (random as f64 / u32::MAX as f64 - 0.5) * 0.4
    }
    fn spawn(&self, task: Pin<Box<dyn Future<Output = ()> + Send>>) {
        self.handle.spawn(task);
    }
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
    async fn attempt(&self, attempt: Attempt<'_>) -> Result<Option<Duration>, WorkFailure> {
        crate::infrastructure::process::attempt::timed(Duration::from_secs(20), attempt)
            .await
            .unwrap_or_else(|error| {
                let message = if error.cleanup_errors.is_empty() {
                    "試行の期限（20秒）を超えました".to_string()
                } else {
                    format!("試行の期限（20秒）を超えました: {error}")
                };
                Err(WorkFailure {
                    kind: FailureKind::Expired,
                    message,
                })
            })
    }
}

#[cfg(test)]
#[path = "work_queue_test.rs"]
mod work_queue_tests;
