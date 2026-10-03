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

    pub fn with_classifier(&self, classify: fn(&str) -> Option<&'static str>) -> Self {
        Self::new(self.limits.clone(), classify, self.events.clone())
    }

    pub async fn run<I, T, E, F, Fut>(
        &self,
        request: I,
        path_of: impl for<'a> FnOnce(&'a I) -> &'a str,
        deadline: Option<Instant>,
        next: F,
        reject: impl FnOnce(Rejection) -> E,
    ) -> Result<T, E>
    where
        F: FnOnce(I) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let path = path_of(&request);
        let Some(level) = (self.classify)(path) else {
            return next(request).await;
        };
        let _seat = match self.limits.admit(level, deadline).await {
            Ok(seat) => seat,
            Err(rejection) => {
                self.events.rejected(path, &rejection);
                return Err(reject(rejection));
            }
        };
        self.events.admitted();
        next(request).await
    }

    #[cfg(test)]
    pub fn limits(&self) -> &PriorityLimits {
        &self.limits
    }
}

#[cfg(test)]
#[path = "priority_test.rs"]
mod priority_tests;
