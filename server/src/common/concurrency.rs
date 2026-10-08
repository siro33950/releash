use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const WAIT_LIMIT: Duration = Duration::from_secs(60);

pub struct PriorityLevel {
    name: &'static str,
    seats: Arc<Semaphore>,
    queue: Arc<Semaphore>,
}

pub struct PriorityLimits {
    levels: Vec<PriorityLevel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    QueueFull,
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub level: &'static str,
    pub reason: RejectReason,
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::QueueFull => "queue_full",
            Self::TimedOut => "time-out",
        })
    }
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} requests rejected: {}", self.level, self.reason)
    }
}
impl std::error::Error for Rejection {}

impl PriorityLimits {
    pub fn new(total: usize, shares: &[(&'static str, u32)], queue_length: usize) -> Self {
        let sum: u32 = shares.iter().map(|(_, share)| share).sum();
        Self {
            levels: shares
                .iter()
                .map(|(name, share)| PriorityLevel {
                    name,
                    seats: Arc::new(Semaphore::new(
                        (total * *share as usize).div_ceil(sum as usize).max(1),
                    )),
                    queue: Arc::new(Semaphore::new(queue_length)),
                })
                .collect(),
        }
    }

    fn level(&self, name: &str) -> &PriorityLevel {
        self.levels
            .iter()
            .find(|level| level.name == name)
            .unwrap_or_else(|| panic!("unknown priority level {name}"))
    }

    pub async fn admit(
        &self,
        name: &str,
        deadline: Option<Instant>,
    ) -> Result<OwnedSemaphorePermit, Rejection> {
        let level = self.level(name);
        if let Ok(seat) = level.seats.clone().try_acquire_owned() {
            return Ok(seat);
        }
        let Ok(_queued) = level.queue.clone().try_acquire_owned() else {
            return Err(Rejection {
                level: level.name,
                reason: RejectReason::QueueFull,
            });
        };
        let wait = deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()) / 4)
            .unwrap_or(WAIT_LIMIT)
            .min(WAIT_LIMIT);
        tokio::time::timeout(wait, level.seats.clone().acquire_owned())
            .await
            .map_err(|_| Rejection {
                level: level.name,
                reason: RejectReason::TimedOut,
            })
            .map(|seat| seat.expect("priority semaphore is never closed"))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fill(&self, name: &str) -> Vec<OwnedSemaphorePermit> {
        let level = self.level(name);
        [&level.seats, &level.queue]
            .into_iter()
            .map(|semaphore| {
                semaphore
                    .clone()
                    .try_acquire_many_owned(semaphore.available_permits() as u32)
                    .expect("fill an idle level")
            })
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn available(&self, name: &str) -> usize {
        self.level(name).seats.available_permits()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn seats(&self, name: &str) -> Arc<Semaphore> {
        self.level(name).seats.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn queue_length(&self, name: &str) -> usize {
        self.level(name).queue.available_permits()
    }
}

#[cfg(test)]
#[path = "concurrency_test.rs"]
mod concurrency_tests;
