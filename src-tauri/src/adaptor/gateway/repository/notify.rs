use crate::domain::repository::RepoPathsNotifier;
use crate::domain::state_subscription::StateValue;
use crate::usecase::state_subscription::{StateSubscriptionPublisher, REPO_PATHS};

pub struct RepoPathsNotifyGateway {
    publisher: StateSubscriptionPublisher,
}

impl RepoPathsNotifyGateway {
    pub(crate) fn new(publisher: StateSubscriptionPublisher) -> Self {
        Self { publisher }
    }
}

impl RepoPathsNotifier for RepoPathsNotifyGateway {
    fn notify_changed(&self, paths: Vec<String>) {
        self.publisher
            .publish(REPO_PATHS, StateValue::RepositoryPaths(paths), None)
            .expect("registered target and available version");
    }
}

#[cfg(test)]
#[path = "notify_test.rs"]
mod notify_tests;
