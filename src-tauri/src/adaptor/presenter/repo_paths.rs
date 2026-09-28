use crate::usecase::repo_paths_usecase::RepoPathsNotifier;
use crate::usecase::state_subscription::StateSubscriptionOutputRef;
use crate::usecase::state_subscription::StateValue;

pub struct RepoPathsNotifyGateway {
    publisher: StateSubscriptionOutputRef,
}

impl RepoPathsNotifyGateway {
    pub(crate) fn new(publisher: StateSubscriptionOutputRef) -> Self {
        Self { publisher }
    }
}

impl RepoPathsNotifier for RepoPathsNotifyGateway {
    fn notify_changed(&self, paths: Vec<String>) {
        self.publisher
            .invalidate(crate::usecase::state_subscription::StateChangeSource::Repositories);
        self.publisher
            .publish(
                &crate::usecase::state_subscription::SubscriptionTarget::RepositoryPaths,
                StateValue::RepositoryPaths(paths),
                None,
            )
            .expect("registered target and available version");
    }
}

#[cfg(test)]
#[path = "repo_paths_test.rs"]
mod notify_tests;
