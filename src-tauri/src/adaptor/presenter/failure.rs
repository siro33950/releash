use crate::adaptor::gateway::failure_records::FailureRecordStore;
use crate::usecase::failure::{FailureKey, FailureOutput, WorkFailure};
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionOutputRef};
use std::sync::Arc;

pub(crate) struct FailurePresenter {
    store: Arc<FailureRecordStore>,
    output: Option<StateSubscriptionOutputRef>,
}

impl FailurePresenter {
    pub(crate) fn new(
        store: Arc<FailureRecordStore>,
        output: Option<StateSubscriptionOutputRef>,
    ) -> Self {
        Self { store, output }
    }

    #[cfg(test)]
    pub(crate) fn store(&self) -> &FailureRecordStore {
        &self.store
    }

    #[cfg(test)]
    pub(crate) fn records(&self, target: &str) -> Vec<crate::usecase::failure::FailureObservation> {
        self.store.records(target)
    }

    fn publish(&self, key: &FailureKey, attention_changed: bool) {
        let Some(output) = &self.output else {
            return;
        };
        if attention_changed && key.operation.starts_with("workflow_") {
            output.invalidate(StateChangeSource::WorkspaceList);
        }
        output.invalidate(StateChangeSource::Failures(key.target.clone()));
    }
}

impl FailureOutput for FailurePresenter {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn observed(&self, key: &FailureKey, failure: WorkFailure) {
        let changed = self.store.observe(key, failure);
        self.publish(key, changed);
    }

    fn resolved(&self, key: &FailureKey) {
        let changed = self.store.resolve(key);
        self.publish(key, changed);
    }
}

#[cfg(test)]
#[path = "failure_test.rs"]
mod failure_tests;
