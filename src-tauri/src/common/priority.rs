use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use super::concurrency::{PriorityLimits, Rejection};

pub trait PriorityEvents: Send + Sync {
    fn rejected(&self, path: &str, rejection: &Rejection);
    fn admitted(&self);
}

pub struct PriorityGate {
    limits: Arc<PriorityLimits>,
    classify: fn(&str) -> Option<&'static str>,
    events: Arc<dyn PriorityEvents>,
}

impl PriorityGate {
    pub fn new(
        limits: Arc<PriorityLimits>,
        classify: fn(&str) -> Option<&'static str>,
        events: Arc<dyn PriorityEvents>,
    ) -> Self {
        Self {
            limits,
            classify,
            events,
        }
    }

    pub async fn run<T, E, F, Fut>(
        &self,
        path: &str,
        deadline: Option<Instant>,
        next: F,
        reject: impl FnOnce(Rejection) -> E,
    ) -> Result<T, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let Some(level) = (self.classify)(path) else {
            return next().await;
        };
        let _seat = match self.limits.admit(level, deadline).await {
            Ok(seat) => seat,
            Err(rejection) => {
                self.events.rejected(path, &rejection);
                return Err(reject(rejection));
            }
        };
        self.events.admitted();
        next().await
    }

    #[cfg(test)]
    pub fn limits(&self) -> &PriorityLimits {
        &self.limits
    }
}
