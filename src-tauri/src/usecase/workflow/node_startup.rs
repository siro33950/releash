use super::runtime_error::WorkflowRuntimeError;
use crate::common::retry::RetryBackoff;
use crate::domain::failure::Failure;
use crate::domain::workflow::entities::workflow_execution::NodeStart;
use crate::usecase::work_queue::AttemptProgress;

#[derive(Debug, Clone)]
pub(crate) struct FailedNodeStart {
    pub id: String,
    pub kind: Failure,
}

#[cfg(test)]
impl From<&str> for FailedNodeStart {
    fn from(id: &str) -> Self {
        Self {
            id: id.into(),
            kind: Failure::Business(crate::domain::failure::BusinessFailure::VersionConflict),
        }
    }
}

#[async_trait::async_trait]
pub(crate) trait NodeStartupGateway: Send + Sync {
    async fn start(
        &self,
        starts: Vec<NodeStart>,
    ) -> Result<Vec<FailedNodeStart>, WorkflowRuntimeError>;
    async fn restart(
        &self,
        node_execution_id: &str,
        action: AttemptProgress,
    ) -> Result<Option<NodeStart>, WorkflowRuntimeError>;
    async fn wait(&self, duration: std::time::Duration) -> bool;
    async fn cancelled(&self);
}

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub(crate) struct NodeStartupError {
    pub node_execution_id: Option<String>,
    pub error: WorkflowRuntimeError,
}

pub(crate) async fn retry_failed_nodes_with_queue(
    gateway: &impl NodeStartupGateway,
    failed: Vec<FailedNodeStart>,
    queue: &std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
) -> Result<(), NodeStartupError> {
    use futures_util::StreamExt;
    let mut pending = futures_util::stream::FuturesUnordered::new();
    for failure in failed {
        pending.push(retry_node(gateway, failure, queue));
    }
    let mut first_error = None;
    while let Some(result) = pending.next().await {
        match result {
            Ok(failed) => {
                for failure in failed {
                    pending.push(retry_node(gateway, failure, queue));
                }
            }
            Err(error) => {
                if first_error.is_none() || error.node_execution_id.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn retry_node(
    gateway: &impl NodeStartupGateway,
    failure: FailedNodeStart,
    queue: &std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
) -> Result<Vec<FailedNodeStart>, NodeStartupError> {
    if crate::usecase::work_queue::next_attempt(failure.kind).is_none() {
        return Ok(Vec::new());
    }
    let key = crate::usecase::work_queue::WorkKey::new("workflow_node_start", &failure.id);
    let state = tokio::sync::Mutex::new((failure, true));
    let state = &state;
    let work = crate::usecase::work_queue::run_borrowed(
        queue,
        key,
        RetryBackoff::ITEM,
        |action| async move {
            let mut state = state.lock().await;
            let (failure, first) = &mut *state;
            if *first {
                *first = false;
                return Err(NodeStartupError {
                    node_execution_id: Some(failure.id.clone()),
                    error: WorkflowRuntimeError::storage(
                        crate::usecase::work_queue::WorkFailure {
                            kind: failure.kind,
                            message: "起動を再試行します".into(),
                        },
                        "起動を再試行します",
                    ),
                });
            }
            if !gateway.wait(std::time::Duration::ZERO).await {
                return Ok(Vec::new());
            }
            let attempted = queue
                .attempt(async {
                    match gateway.restart(&failure.id, action).await {
                        Ok(Some(start)) => {
                            failure.id = start.node_execution_id().to_string();
                            (gateway.start(vec![start]).await, None)
                        }
                        Ok(None) => (Ok(Vec::new()), None),
                        Err(error) => (Err(error), Some(failure.id.clone())),
                    }
                })
                .await;
            let (result, node_execution_id) = attempted.unwrap_or_else(|error| {
                (
                    Err({
                        let message = error.message.clone();
                        WorkflowRuntimeError::storage(error, message)
                    }),
                    Some(failure.id.clone()),
                )
            });
            match result {
                Ok(mut failed)
                    if failed.len() == 1
                        && crate::usecase::work_queue::next_attempt(failed[0].kind).is_some() =>
                {
                    *failure = failed.remove(0);
                    Err(NodeStartupError {
                        node_execution_id: Some(failure.id.clone()),
                        error: WorkflowRuntimeError::storage(
                            crate::usecase::work_queue::WorkFailure {
                                kind: failure.kind,
                                message: "起動を再試行します".into(),
                            },
                            "起動を再試行します",
                        ),
                    })
                }
                Ok(failed) => Ok(failed),
                Err(error) => {
                    queue
                        .observe(
                            &crate::usecase::work_queue::WorkKey::new(
                                "workflow_node_start",
                                &failure.id,
                            ),
                            &crate::usecase::work_queue::WorkFailure::from_error(&error),
                        )
                        .await;
                    Err(NodeStartupError {
                        node_execution_id,
                        error,
                    })
                }
            }
        },
        true,
        false,
    );
    tokio::select! {
        biased;
        _ = async {
            gateway.cancelled().await;
            let _attempt = state.lock().await;
        } => Ok(Vec::new()),
        result = work => result,
    }
}

#[cfg(test)]
#[path = "node_startup_test.rs"]
mod node_startup_tests;

impl From<&NodeStartupError> for Failure {
    fn from(error: &NodeStartupError) -> Self {
        (&error.error).into()
    }
}
