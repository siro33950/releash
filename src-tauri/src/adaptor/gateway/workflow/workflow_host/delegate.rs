use super::*;
use crate::domain::workflow::entities::workflow_execution::DelegateInjection;
use crate::usecase::workflow::control_plane::WorkflowControlPlaneCommit;
use crate::usecase::workflow::delegate::DelegateContinuationGateway;

#[derive(Clone, Copy)]
pub(super) enum DelegateInjectionOrigin {
    Automatic,
    Resume,
}

pub(crate) struct HostDelegateContinuation {
    pub(crate) host: WorkflowRuntimeHost,
    pub(crate) app: WorkflowRuntimeDependencies,
}

#[async_trait::async_trait]
impl DelegateContinuationGateway for HostDelegateContinuation {
    fn current_timestamp(&self) -> f64 {
        current_timestamp()
    }

    async fn load_execution(
        &self,
        execution_id: &str,
    ) -> Result<DomainExecutionTree, WorkflowRuntimeError> {
        self.host
            .load_control_plane_execution(&self.app, execution_id)
            .await?
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
    pub(super) async fn inject_delegate_result(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        injection: &DelegateInjection,
        origin: DelegateInjectionOrigin,
    ) -> Result<(), WorkflowRuntimeError> {
        let gate = self.runtime_activation_gate(execution_id).await;
        let guard = gate.lock.lock().await;
        let continuation = self.delegate_continuation.as_ref().ok_or_else(|| {
            WorkflowRuntimeError::InvalidState("delegate continuation is not configured".into())
        })?;
        let result = run_runtime_activation(&gate, execution_id, "delegate", async {
            Ok(continuation.execute(execution_id, injection).await)
        })
        .await;

        drop(guard);
        let snapshot = match result {
            Err(_) => return Ok(()),
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(error)) => {
                if matches!(origin, DelegateInjectionOrigin::Resume) {
                    return Err(error);
                }
                return Box::pin(self.settle_runtime_failure_for_node(
                    app,
                    execution_id,
                    &injection.node_execution_id,
                    &error,
                ))
                .await;
            }
        };
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
