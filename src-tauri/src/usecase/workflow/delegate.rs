use super::control_plane::WorkflowControlPlaneCommit;
use super::runtime_error::WorkflowRuntimeError;
use super::runtime_snapshot::RuntimeCommitSnapshot;
use crate::domain::workflow::entities::workflow_execution::{
    DelegateInjection, TransitionOutcome, WorkflowExecution,
};
use crate::domain::workflow::services::prompt_composition::delegate_continuation_instruction;
use crate::domain::workflow::WorkflowEvent;

#[async_trait::async_trait]
pub(crate) trait DelegateContinuationGateway: Send + Sync {
    fn current_timestamp(&self) -> f64;
    async fn load_execution(
        &self,
        execution_id: &str,
    ) -> Result<WorkflowExecution, WorkflowRuntimeError>;
    async fn restore_provider(
        &self,
        session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;
    async fn send_instruction(
        &self,
        session_id: &str,
        child_execution_id: &str,
        instruction: &str,
    ) -> Result<(), WorkflowRuntimeError>;
    async fn commit(
        &self,
        commit: WorkflowControlPlaneCommit,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError>;
}

pub(crate) struct DelegateContinuationUsecase {
    pub(crate) gateway: std::sync::Arc<dyn DelegateContinuationGateway>,
}

impl DelegateContinuationUsecase {
    pub(crate) async fn execute(
        &self,
        execution_id: &str,
        injection: &DelegateInjection,
    ) -> Result<Option<RuntimeCommitSnapshot>, WorkflowRuntimeError> {
        let current = self.gateway.load_execution(execution_id).await?;
        if current
            .pending_delegate_injection(&injection.node_execution_id)
            .as_ref()
            != Some(injection)
        {
            return Ok(None);
        }
        let node = current
            .node_execution(&injection.node_execution_id)
            .unwrap();
        let session_id = node.session_id.as_deref().ok_or_else(|| {
            WorkflowRuntimeError::AgentSession("delegate parent has no provider session".into())
        })?;
        let artifact = node.artifact.as_ref().ok_or_else(|| {
            WorkflowRuntimeError::InvalidState("delegate parent has no Artifact".into())
        })?;
        let instruction = delegate_continuation_instruction(artifact);
        self.gateway
            .restore_provider(session_id, &injection.node_execution_id)
            .await?;
        self.gateway
            .send_instruction(session_id, &injection.child_execution_id, &instruction)
            .await?;
        let timestamp = self.gateway.current_timestamp();
        let mut candidate = current.clone();
        let outcome = candidate.record_delegate_injected(injection, timestamp);
        if outcome != TransitionOutcome::Applied {
            return Err(WorkflowRuntimeError::InvalidState(
                "delegate injection could not be recorded".into(),
            ));
        }
        self.gateway
            .commit(WorkflowControlPlaneCommit {
                execution_id: execution_id.into(),
                before: current,
                after: candidate,
                transition_outcome: outcome,
                workflow_events: vec![WorkflowEvent::DelegateResultInjected {
                    execution_id: execution_id.into(),
                    node_execution_id: injection.node_execution_id.clone(),
                    child_execution_id: injection.child_execution_id.clone(),
                    timestamp,
                }],
                provider_events: Vec::new(),
            })
            .await
            .map(Some)
    }
}

#[cfg(test)]
#[path = "delegate_test.rs"]
mod delegate_tests;
