#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ApplicationLifecycleError(pub String);

#[async_trait::async_trait]
pub(crate) trait ApplicationShutdownGateway: Send + Sync {
    async fn stop_commands(&self) -> Result<(), ApplicationLifecycleError>;
    async fn stop_provider_exit_observer(&self) -> Result<(), ApplicationLifecycleError>;
    async fn save_terminals(&self) -> Result<(), ApplicationLifecycleError>;
    async fn stop_local_api(&self) -> Result<(), ApplicationLifecycleError>;
    async fn shutdown_telemetry(&self) -> Result<(), ApplicationLifecycleError>;
}
