use crate::domain::daemon::{
    DaemonInfo, DaemonRepository, DaemonRequest, StartupFailure, StopAcceptance, StopRequest,
};
use std::sync::Arc;
#[derive(Clone)]
pub(crate) struct DaemonUsecase(pub(crate) Arc<dyn DaemonRepository>);
impl DaemonUsecase {
    pub(crate) async fn info(&self) -> DaemonInfo {
        self.0.info().await
    }
    pub(crate) async fn admits(&self, request: DaemonRequest) -> bool {
        self.0.admits(request).await
    }
    pub(crate) async fn serve(&self) {
        self.0.serve().await;
    }
    pub(crate) async fn fail(&self, failure: StartupFailure) {
        self.0.fail(failure).await;
    }
    pub(crate) async fn stop(&self, request: StopRequest) -> StopAcceptance {
        self.0.stop(request).await
    }
    pub(crate) async fn stopped(&self) {
        self.0.stopped().await;
    }
}
#[cfg(test)]
#[path = "daemon_test.rs"]
mod daemon_tests;
