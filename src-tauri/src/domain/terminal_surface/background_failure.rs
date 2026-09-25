use crate::domain::failure::{BackgroundFailures, FailureKind, TargetFailures};

#[derive(Default)]
pub(crate) struct BackgroundFailureState(TargetFailures);

impl BackgroundFailures for BackgroundFailureState {
    fn observe(&mut self, target: &str, kind: FailureKind) -> bool {
        self.0.observe("terminal_checkpoint", target, kind)
    }
    fn clear(&mut self, target: &str) -> bool {
        self.0.clear("terminal_checkpoint", target)
    }
    fn requires_attention(&self, target: &str) -> bool {
        self.0.requires_attention("terminal_checkpoint", target)
    }
}

#[cfg(test)]
#[path = "background_failure_test.rs"]
mod background_failure_tests;
