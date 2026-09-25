use crate::domain::failure::{BackgroundFailures, FailureKind, TargetFailures};

#[derive(Default)]
pub(crate) struct BackgroundFailureState(TargetFailures);

impl BackgroundFailures for BackgroundFailureState {
    fn observe(&mut self, target: &str, kind: FailureKind) -> bool {
        self.0.observe("provider_session_title", target, kind)
    }
    fn clear(&mut self, target: &str) -> bool {
        self.0.clear("provider_session_title", target)
    }
    fn requires_attention(&self, target: &str) -> bool {
        self.0.requires_attention("provider_session_title", target)
    }
}

#[cfg(test)]
#[path = "background_failure_test.rs"]
mod background_failure_tests;
