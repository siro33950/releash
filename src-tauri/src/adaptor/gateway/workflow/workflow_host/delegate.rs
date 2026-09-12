use super::*;
use crate::domain::workflow::entities::workflow_execution::DelegateInjection;
use crate::usecase::workflow::control_plane::WorkflowControlPlaneCommit;
use crate::usecase::workflow::delegate::DelegateContinuationGateway;

pub(crate) struct HostDelegateContinuation<R: tauri::Runtime> {
    pub(crate) host: WorkflowRuntimeHost,
    pub(crate) app: tauri::AppHandle<R>,
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> DelegateContinuationGateway for HostDelegateContinuation<R> {
    fn current_timestamp(&self) -> f64 {
        current_timestamp()
    }

    async fn load_execution(
        &self,
        execution_id: &str,
    ) -> Result<DomainWorkflowExecution, WorkflowRuntimeError> {
        self.host
            .load_control_plane_execution(execution_id)
            .await
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.into()))
    }

    async fn restore_provider(
        &self,
        session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.host
            .workflow_agent_sessions
            .recover_workflow_agent_session_provider(session_id, node_execution_id)
            .await
    }

    async fn send_instruction(
        &self,
        session_id: &str,
        child_execution_id: &str,
        instruction: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.host
            .workflow_agent_sessions
            .dispatch_continuation(session_id, child_execution_id, instruction)
            .await
    }

    async fn commit(
        &self,
        commit: WorkflowControlPlaneCommit,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        self.host
            .commit_workflow_control_plane(&self.app, commit)
            .await
    }
}

impl WorkflowRuntimeHost {
    pub(super) async fn inject_delegate_result<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        injection: &DelegateInjection,
    ) -> Result<(), WorkflowRuntimeError> {
        let gate = self.runtime_activation_gate(execution_id).await;
        let guard = gate.lock.lock().await;
        let continuation = self.delegate_continuation.as_ref().ok_or_else(|| {
            WorkflowRuntimeError::InvalidState("delegate continuation is not configured".into())
        })?;
        let snapshot = continuation.execute(execution_id, injection).await?;
        drop(guard);
        if let Some(snapshot) = snapshot {
            self.finalize_after_commit(app, &snapshot, &snapshot.worktree_path)
                .await;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "delegate_test.rs"]
mod delegate_tests;
