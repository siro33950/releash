use super::runtime_error::WorkflowRuntimeError;
use crate::domain::workflow::entities::workflow_execution::NodeStart;
use crate::domain::workflow::startup_restart_delay;

#[async_trait::async_trait]
pub(crate) trait NodeStartupGateway: Send + Sync {
    async fn start(&self, starts: Vec<NodeStart>) -> Result<Vec<String>, WorkflowRuntimeError>;
    async fn restart(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<NodeStart>, WorkflowRuntimeError>;
    async fn wait(&self, duration: std::time::Duration) -> bool;
}

pub(crate) async fn retry_failed_nodes(
    gateway: &impl NodeStartupGateway,
    mut failed: Vec<String>,
) -> Result<(), WorkflowRuntimeError> {
    let mut restarts = 0;
    while !failed.is_empty() {
        let Some(delay) = startup_restart_delay(restarts) else {
            return Ok(());
        };
        if !gateway.wait(delay).await {
            return Ok(());
        }
        let mut starts = Vec::new();
        for id in failed {
            if let Some(start) = gateway.restart(&id).await? {
                starts.push(start);
            }
        }
        if starts.is_empty() {
            return Ok(());
        }
        failed = gateway.start(starts).await?;
        restarts += 1;
    }
    Ok(())
}

#[cfg(test)]
#[path = "node_startup_test.rs"]
mod node_startup_tests;
