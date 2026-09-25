use std::sync::Arc;
use std::time::{Duration, Instant};

use super::failure::{ClassifiedFailure, FailureKind};

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
impl ClassifiedFailure for OperationStopped {
    fn failure_kind(&self) -> FailureKind {
        match self {
            Self::Expired => FailureKind::Expired,
            Self::Cancelled => FailureKind::Cancelled,
        }
    }
}

#[cfg(test)]
#[path = "operation_context_test.rs"]
mod operation_context_tests;
