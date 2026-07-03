use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::usecase::agent_session::runtime::ports::{
    AgentTaskSpawner, WorkflowTurnCompleteNotifier,
};
use crate::usecase::workflow::ports::WorkflowTurnCompleteNotification;
use crate::usecase::workflow::WorkflowRuntimeUsecase;

pub(crate) struct TokioAgentTaskSpawner;

impl AgentTaskSpawner for TokioAgentTaskSpawner {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        tokio::spawn(future);
    }
}

pub(crate) struct WorkflowRuntimeTurnCompleteNotifier {
    workflow_runtime: Arc<WorkflowRuntimeUsecase>,
}

impl WorkflowRuntimeTurnCompleteNotifier {
    pub(crate) fn new(workflow_runtime: Arc<WorkflowRuntimeUsecase>) -> Self {
        Self { workflow_runtime }
    }
}

#[async_trait::async_trait]
impl WorkflowTurnCompleteNotifier for WorkflowRuntimeTurnCompleteNotifier {
    async fn turn_completed(&self, notification: WorkflowTurnCompleteNotification) {
        if let Err(error) = self.workflow_runtime.complete_turn(notification).await {
            log::warn!("workflow turn-complete notification failed: {error}");
        }
    }
}
