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

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub(crate) struct NodeStartupError {
    pub node_execution_id: Option<String>,
    pub error: WorkflowRuntimeError,
}

pub(crate) async fn retry_failed_nodes(
    gateway: &impl NodeStartupGateway,
    mut failed: Vec<String>,
) -> Result<(), NodeStartupError> {
    let mut restarts = 0;
    let mut first_error = None;
    while !failed.is_empty() {
        let Some(delay) = startup_restart_delay(restarts) else {
            break;
        };
        if !gateway.wait(delay).await {
            break;
        }
        let mut starts = Vec::new();
        for id in failed {
            match gateway.restart(&id).await {
                Ok(Some(start)) => starts.push(start),
                Ok(None) => {}
                Err(error) => {
                    first_error.get_or_insert(NodeStartupError {
                        node_execution_id: Some(id),
                        error,
                    });
                }
            }
        }
        if starts.is_empty() {
            break;
        }
        failed = gateway
            .start(starts)
            .await
            .map_err(|error| NodeStartupError {
                node_execution_id: None,
                error,
            })?;
        restarts += 1;
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
#[path = "node_startup_test.rs"]
mod node_startup_tests;
