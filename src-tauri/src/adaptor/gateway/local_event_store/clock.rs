//! Store clock abstraction so tests can drive deadlines deterministically.

#[cfg(test)]
use std::sync::atomic::{AtomicI64, Ordering};
#[cfg(test)]
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub trait StoreClock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64;
}

#[derive(Debug, Default)]
pub struct SystemStoreClock;

impl StoreClock for SystemStoreClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// Deterministic fake clock for tests and fault harnesses.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub struct FakeStoreClock {
    now_ms: Arc<AtomicI64>,
}

#[cfg(test)]
impl FakeStoreClock {
    pub fn at(now_ms: i64) -> Self {
        Self {
            now_ms: Arc::new(AtomicI64::new(now_ms)),
        }
    }

    pub fn advance_ms(&self, delta_ms: i64) {
        self.now_ms.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

#[cfg(test)]
impl StoreClock for FakeStoreClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}
