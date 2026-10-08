use crate::domain::daemon::{
    DaemonInfo, DaemonRepository, DaemonRequest, StartupFailure, StopAcceptance, StopRequest,
};
use std::sync::Arc;
#[derive(Clone)]
pub struct DaemonUsecase {
    repository: Arc<dyn DaemonRepository>,
    subscriptions: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
}
impl DaemonUsecase {
    pub(crate) fn new(repository: Arc<dyn DaemonRepository>) -> Self {
        Self {
            repository,
            subscriptions: None,
        }
    }
    pub(crate) fn with_state_publisher(
        mut self,
        subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.subscriptions = Some(subscriptions);
        self
    }
    fn notify(&self) {
        if let Some(subscriptions) = &self.subscriptions {
            subscriptions.notify(crate::usecase::state_subscription::StateChangeSource::Daemon);
        }
    }

    #[cfg(feature = "test-support")]
    pub fn test_with_repository(repository: Arc<dyn DaemonRepository>) -> Self {
        Self::new(repository)
    }

    pub(crate) async fn info(&self) -> DaemonInfo {
        self.repository.info().await
    }
    pub async fn admits(&self, request: DaemonRequest) -> bool {
        self.repository.admits(request).await
    }
    pub(crate) async fn serve(&self) {
        self.repository.serve().await;
        self.notify();
    }
    pub(crate) async fn fail(&self, failure: StartupFailure) {
        self.repository.fail(failure).await;
        self.notify();
    }
    pub async fn stop(&self, request: StopRequest) -> StopAcceptance {
        let acceptance = self.repository.stop(request).await;
        if matches!(acceptance, StopAcceptance::Started { .. }) {
            self.notify();
        }
        acceptance
    }
    pub(crate) async fn stopped(&self) {
        self.repository.stopped().await;
        self.notify();
    }
}
#[cfg(test)]
#[path = "daemon_test.rs"]
pub(crate) mod daemon_tests;
