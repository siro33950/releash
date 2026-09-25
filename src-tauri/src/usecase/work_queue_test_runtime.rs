use crate::domain::failure::FailureKind;
use crate::usecase::work_queue::{Attempt, WorkFailure, WorkQueueRuntime};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct TestWorkQueueRuntime {
    origin: std::time::Instant,
    handle: tokio::runtime::Handle,
}
impl Default for TestWorkQueueRuntime {
    fn default() -> Self {
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
        Self {
            origin: std::time::Instant::now(),
            handle,
        }
    }
}
#[async_trait::async_trait]
impl WorkQueueRuntime for TestWorkQueueRuntime {
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
        1.0
    }
    fn spawn(&self, task: Pin<Box<dyn Future<Output = ()> + Send>>) {
        self.handle.spawn(task);
    }
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
    async fn attempt(&self, attempt: Attempt<'_>) -> Result<Option<Duration>, WorkFailure> {
        tokio::time::timeout(Duration::from_secs(20), attempt)
            .await
            .unwrap_or_else(|_| {
                Err(WorkFailure {
                    kind: FailureKind::Expired,
                    message: "試行の期限（20秒）を超えました".into(),
                })
            })
    }
}

pub(crate) fn runtime() -> std::sync::Arc<dyn WorkQueueRuntime> {
    std::sync::Arc::new(TestWorkQueueRuntime::default())
}
