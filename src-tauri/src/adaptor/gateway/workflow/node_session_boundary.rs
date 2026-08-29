use std::sync::Arc;

use crate::domain::agent_session::ProviderAvailabilityReader;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{
    AgentSessionInitialInstructionUsecase, AgentSessionInterruptUsecase, AgentSessionLaunchUsecase,
    AgentSessionLaunchUsecaseError, AgentSessionLifecycleUsecase,
    WorkflowAgentSessionLaunchRequest,
};
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkflowSessionLaunchConfig {
    pub(crate) provider: ProviderKind,
    pub(crate) model: Option<String>,
    pub(crate) permission: Option<crate::domain::workflow::SessionPermission>,
}

impl WorkflowSessionLaunchConfig {
    pub(crate) fn from_session_spec(spec: &crate::domain::workflow::SessionSpec) -> Self {
        Self {
            provider: spec.provider,
            model: spec.model.clone(),
            permission: spec.permission,
        }
    }
}

/// 起動済み Workflow AgentSession の識別情報。
pub(crate) struct NodeSessionInfo {
    pub(crate) id: String,
}

#[async_trait::async_trait]
pub(crate) trait WorkflowAgentSessionPort: Send + Sync {
    fn is_provider_available(&self, provider: ProviderKind) -> bool;

    async fn prepare_workflow_agent_session(
        &self,
        worktree_path: &str,
        config: WorkflowSessionLaunchConfig,
        workflow_execution_id: &str,
        node_execution_id: &str,
        initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError>;

    async fn activate_workflow_agent_session(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn confirm_workflow_agent_session_attachment(
        &self,
        node_session_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn dispatch_initial_instruction(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
        instruction: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn recover_workflow_agent_session_provider(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn interrupt_workflow_agent_session(
        &self,
        node_session_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn rollback_workflow_agent_session(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;
}

pub(crate) struct ProviderWorkflowAgentSessionPort {
    launch: Arc<AgentSessionLaunchUsecase>,
    initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
    interrupt: Arc<AgentSessionInterruptUsecase>,
    lifecycle: Arc<AgentSessionLifecycleUsecase>,
    availability: Arc<dyn ProviderAvailabilityReader>,
}

fn activation_error(
    node_session_id: &str,
    error: AgentSessionLaunchUsecaseError,
) -> WorkflowRuntimeError {
    WorkflowRuntimeError::AgentSession(format!(
        "activate Workflow AgentSession '{node_session_id}': {error}"
    ))
}

impl ProviderWorkflowAgentSessionPort {
    pub(crate) fn new(
        launch: Arc<AgentSessionLaunchUsecase>,
        initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
        interrupt: Arc<AgentSessionInterruptUsecase>,
        lifecycle: Arc<AgentSessionLifecycleUsecase>,
        availability: Arc<dyn ProviderAvailabilityReader>,
    ) -> Self {
        Self {
            launch,
            initial_instruction,
            interrupt,
            lifecycle,
            availability,
        }
    }
}

#[async_trait::async_trait]
impl WorkflowAgentSessionPort for ProviderWorkflowAgentSessionPort {
    fn is_provider_available(&self, provider: ProviderKind) -> bool {
        self.availability.is_available(provider)
    }

    async fn prepare_workflow_agent_session(
        &self,
        worktree_path: &str,
        config: WorkflowSessionLaunchConfig,
        workflow_execution_id: &str,
        node_execution_id: &str,
        initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        let launched = self
            .launch
            .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new(worktree_path),
                worktree_path: worktree_path.to_string(),
                provider: config.provider,
                model: config.model,
                permission: config.permission,
                workflow_execution_id: workflow_execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                initial_instruction: initial_instruction.to_string(),
                rows: 24,
                cols: 80,
                caller_request_id: format!("workflow-node-launch-{node_execution_id}"),
            })
            .await
            .map_err(|error| {
                WorkflowRuntimeError::AgentSession(format!(
                    "launch Workflow AgentSession for NodeExecution '{node_execution_id}': {error:?}"
                ))
            })?;
        Ok(NodeSessionInfo {
            id: launched.session().id().to_string(),
        })
    }

    async fn activate_workflow_agent_session(
        &self,
        node_session_id: &str,
        _node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.launch
            .activate_workflow_node(node_session_id)
            .await
            .map_err(|error| activation_error(node_session_id, error))?;
        Ok(())
    }

    async fn confirm_workflow_agent_session_attachment(
        &self,
        node_session_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.launch
            .confirm_workflow_node_attachment(node_session_id)
            .await
            .map_err(|error| {
                WorkflowRuntimeError::AgentSession(format!(
                    "confirm Workflow AgentSession attachment '{node_session_id}': {error:?}"
                ))
            })
    }

    async fn dispatch_initial_instruction(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
        instruction: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.initial_instruction
            .dispatch(
                node_session_id,
                instruction,
                &format!("workflow-node-initial-instruction-{node_execution_id}"),
            )
            .await
            .map_err(|error| {
                WorkflowRuntimeError::AgentSession(format!(
                    "dispatch initial instruction for AgentSession '{node_session_id}': {error:?}"
                ))
            })?;
        Ok(())
    }

    async fn recover_workflow_agent_session_provider(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.lifecycle
            .ensure_provider_running(
                node_session_id,
                24,
                80,
                &format!("workflow-node-provider-recovery-{node_execution_id}"),
            )
            .await
            .map_err(|error| {
                WorkflowRuntimeError::AgentSession(format!(
                    "recover provider for Workflow AgentSession '{node_session_id}': {error:?}"
                ))
            })?;
        Ok(())
    }

    async fn interrupt_workflow_agent_session(
        &self,
        node_session_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.interrupt
            .interrupt(node_session_id)
            .await
            .map_err(|error| {
                WorkflowRuntimeError::AgentSession(format!(
                    "interrupt Workflow AgentSession '{node_session_id}': {error:?}"
                ))
            })
    }

    async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.lifecycle
            .stop_for_terminal_execution_tree_node_preserving_checkpoint(
                node_session_id,
                node_execution_id,
                &format!("workflow-node-terminal-stop-{node_execution_id}"),
            )
            .await
            .map_err(|error| {
                WorkflowRuntimeError::AgentSession(format!(
                    "stop Workflow AgentSession '{node_session_id}' for NodeExecution '{node_execution_id}': {error:?}"
                ))
            })
    }

    async fn rollback_workflow_agent_session(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.launch
            .rollback_workflow_node(
                node_session_id,
                &format!("workflow-node-launch-rollback-{node_execution_id}-{node_session_id}"),
            )
            .await
            .map_err(|error| {
                WorkflowRuntimeError::AgentSession(format!(
                    "rollback unattached Workflow AgentSession '{node_session_id}': {error:?}"
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::ProviderAgentTerminalSpawnError;

    #[test]
    fn test_workflow_agent_session_activation_terminal_spawn分類をcontext付きで保持する() {
        let error = activation_error(
            "agent-session-1",
            AgentSessionLaunchUsecaseError::TerminalSpawn(
                ProviderAgentTerminalSpawnError::PtySpawn {
                    error: "openpty failed".to_string(),
                },
            ),
        );

        assert_eq!(
            error.to_string(),
            "activate Workflow AgentSession 'agent-session-1': kind=pty_spawn error=openpty failed"
        );
    }

    #[test]
    fn test_workflow_agent_session_activation_terminal以外の既存表現を維持する() {
        let error = activation_error(
            "agent-session-1",
            AgentSessionLaunchUsecaseError::LaunchUnavailable,
        );

        assert_eq!(
            error.to_string(),
            "activate Workflow AgentSession 'agent-session-1': LaunchUnavailable"
        );
    }
}
