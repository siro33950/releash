use crate::domain::failure::{BackgroundFailures, FailureKind, TargetFailures};

#[derive(Default)]
pub(crate) struct BackgroundFailureState(TargetFailures);

impl BackgroundFailures for BackgroundFailureState {
    fn observe(&mut self, target: &str, kind: FailureKind) -> bool {
        self.0.observe("repository_scan", target, kind)
    }
    fn clear(&mut self, target: &str) -> bool {
        self.0.clear("repository_scan", target)
    }
    fn requires_attention(&self, target: &str) -> bool {
        self.0.requires_attention("repository_scan", target)
    }
}

#[cfg(test)]
#[path = "background_failure_test.rs"]
mod background_failure_tests;
