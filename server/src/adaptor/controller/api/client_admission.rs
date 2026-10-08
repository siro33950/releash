use crate::domain::daemon::DaemonRequest;
#[derive(Clone)]
pub(crate) struct DaemonAdmission(pub(crate) crate::usecase::daemon::DaemonUsecase);
impl DaemonAdmission {
    async fn admit(&self, path: &str) -> Result<(), connectrpc::ConnectError> {
        let request = match path.rsplit('/').next().unwrap_or_default() {
            "GetServerInfo" => DaemonRequest::Status,
            "StopDaemon" => DaemonRequest::Stop,
            _ => DaemonRequest::Operation,
        };
        if self.0.admits(request).await {
            return Ok(());
        }
        Err(crate::adaptor::presenter::connect::command_error(
            crate::adaptor::presenter::error::AppError::invalid_state("Application is unavailable")
                .with_code("APPLICATION_UNAVAILABLE")
                .into(),
        ))
    }
}
#[connectrpc::async_trait]
impl connectrpc::Interceptor for DaemonAdmission {
    async fn intercept_unary(
        &self,
        req: connectrpc::interceptor::UnaryRequest,
        next: connectrpc::Next<'_>,
    ) -> Result<connectrpc::interceptor::UnaryResponse, connectrpc::ConnectError> {
        self.admit(req.ctx.path().expect("RPC path")).await?;
        next.run(req).await
    }
    async fn intercept_streaming(
        &self,
        req: connectrpc::interceptor::StreamRequest,
        inbound: connectrpc::interceptor::PayloadStream,
        next: connectrpc::NextStream<'_>,
    ) -> Result<connectrpc::interceptor::StreamResponse, connectrpc::ConnectError> {
        self.admit(req.ctx.path().expect("RPC path")).await?;
        next.run(req, inbound).await
    }
}
#[cfg(test)]
#[path = "client_admission_test.rs"]
mod client_admission_tests;
