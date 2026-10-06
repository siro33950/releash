use crate::domain::daemon::{
    DaemonInfo, DaemonRepository, DaemonRequest, StartupFailure, StopAcceptance, StopRequest,
};
use std::sync::Arc;
#[derive(Clone)]
pub struct DaemonUsecase(pub(crate) Arc<dyn DaemonRepository>);
impl DaemonUsecase {
    #[cfg(feature = "test-support")]
    pub fn test_with_repository(repository: Arc<dyn DaemonRepository>) -> Self {
        Self(repository)
    }

    pub(crate) async fn info(&self) -> DaemonInfo {
        self.0.info().await
    }
    pub async fn admits(&self, request: DaemonRequest) -> bool {
        self.0.admits(request).await
    }
    pub(crate) async fn serve(&self) {
        self.0.serve().await;
    }
    pub(crate) async fn fail(&self, failure: StartupFailure) {
        self.0.fail(failure).await;
    }
    pub async fn stop(&self, request: StopRequest) -> StopAcceptance {
        self.0.stop(request).await
    }
    pub(crate) async fn stopped(&self) {
        self.0.stopped().await;
    }
}
#[cfg(test)]
#[path = "daemon_test.rs"]
pub(crate) mod daemon_tests;
