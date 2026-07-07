use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::runtime::ports::{
    AgentTaskSpawner, WorkflowStallNotifier, WorkflowTurnCompleteNotifier,
};
use crate::usecase::workflow::ports::{
    WorkflowStallClearedNotification, WorkflowStallObservedNotification,
    WorkflowTurnCompleteNotification,
};
use crate::usecase::workflow::WorkflowRuntimeUsecase;

pub(crate) struct TokioAgentTaskSpawner;

impl AgentTaskSpawner for TokioAgentTaskSpawner {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        tokio::spawn(future);
    }
}

pub(crate) struct WorkflowRuntimeAgentSessionNotifier {
    workflow_runtime: Arc<WorkflowRuntimeUsecase>,
}

impl WorkflowRuntimeAgentSessionNotifier {
    pub(crate) fn new(workflow_runtime: Arc<WorkflowRuntimeUsecase>) -> Self {
        Self { workflow_runtime }
    }
}

#[async_trait::async_trait]
impl WorkflowTurnCompleteNotifier for WorkflowRuntimeAgentSessionNotifier {
    async fn turn_completed(&self, notification: WorkflowTurnCompleteNotification) {
        if let Err(error) = self.workflow_runtime.complete_turn(notification).await {
            log::warn!("workflow turn-complete notification failed: {error}");
        }
    }
}

#[async_trait::async_trait]
impl WorkflowStallNotifier for WorkflowRuntimeAgentSessionNotifier {
    async fn stall_observed(&self, notification: WorkflowStallObservedNotification) {
        if let Err(error) = self.workflow_runtime.observe_stall(notification).await {
            log::warn!("workflow stall-observed notification failed: {error}");
        }
    }

    async fn stall_cleared(
        &self,
        notification: WorkflowStallClearedNotification,
    ) -> Result<(), WorkflowError> {
        self.workflow_runtime.clear_stall(notification).await
    }
}
