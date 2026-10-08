use std::sync::Arc;

use crate::domain::agent_session::ProviderAvailabilityReader;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{
    AgentSessionInitialInstructionUsecase, AgentSessionLaunchUsecase,
    AgentSessionLaunchUsecaseError, AgentSessionLifecycleUsecase,
    WorkflowAgentSessionLaunchRequest,
};
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowSessionLaunchConfig {
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub permission: Option<crate::domain::workflow::SessionPermission>,
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
pub struct NodeSessionInfo {
    pub id: String,
}

#[async_trait::async_trait]
pub trait WorkflowAgentSessionPort: Send + Sync {
    fn is_provider_available(&self, provider: ProviderKind) -> bool;

    async fn prepare_workflow_agent_session(
        &self,
        workspace_worktree_path: &str,
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

    /// `child_execution_id` ごとに一度だけ届く。同じ child の再送は session 側が拒む。
    async fn dispatch_continuation(
        &self,
        node_session_id: &str,
        child_execution_id: &str,
        instruction: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn has_recoverable_conversation(
        &self,
        node_session_id: &str,
    ) -> Result<bool, WorkflowRuntimeError>;

    async fn recover_workflow_agent_session_provider(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
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

pub struct ProviderWorkflowAgentSessionPort {
    launch: Arc<AgentSessionLaunchUsecase>,
    initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
    lifecycle: Arc<AgentSessionLifecycleUsecase>,
    availability: Arc<dyn ProviderAvailabilityReader>,
}

fn activation_error(
    node_session_id: &str,
    error: AgentSessionLaunchUsecaseError,
) -> WorkflowRuntimeError {
    let context = format!("activate Workflow AgentSession '{node_session_id}'");
    if error.technical_failure().is_none() {
        return WorkflowRuntimeError::AgentSession(format!("{context}: {error}"));
    }
    launch_failure(context, error)
}

impl ProviderWorkflowAgentSessionPort {
    pub fn new(
        launch: Arc<AgentSessionLaunchUsecase>,
        initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
        lifecycle: Arc<AgentSessionLifecycleUsecase>,
        availability: Arc<dyn ProviderAvailabilityReader>,
    ) -> Self {
        Self {
            launch,
            initial_instruction,
            lifecycle,
            availability,
        }
    }
}

fn workflow_launch_request(
    workspace_worktree_path: &str,
    worktree_path: &str,
    config: WorkflowSessionLaunchConfig,
    workflow_execution_id: &str,
    node_execution_id: &str,
    initial_instruction: &str,
) -> WorkflowAgentSessionLaunchRequest {
    WorkflowAgentSessionLaunchRequest {
        workspace: WorkspaceIdentity::new(workspace_worktree_path),
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
    }
}

#[async_trait::async_trait]
impl WorkflowAgentSessionPort for ProviderWorkflowAgentSessionPort {
    fn is_provider_available(&self, provider: ProviderKind) -> bool {
        self.availability.is_available(provider)
    }

    async fn prepare_workflow_agent_session(
        &self,
        workspace_worktree_path: &str,
        worktree_path: &str,
        config: WorkflowSessionLaunchConfig,
        workflow_execution_id: &str,
        node_execution_id: &str,
        initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        let launched = self
            .launch
            .prepare_workflow_node(workflow_launch_request(
                workspace_worktree_path,
                worktree_path,
                config,
                workflow_execution_id,
                node_execution_id,
                initial_instruction,
            ))
            .await
            .map_err(|error| {
                launch_failure(
                    format!("launch Workflow AgentSession for NodeExecution '{node_execution_id}'"),
                    error,
                )
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
                launch_failure(
                    format!("confirm Workflow AgentSession attachment '{node_session_id}'"),
                    error,
                )
            })
    }

    async fn dispatch_continuation(
        &self,
        node_session_id: &str,
        child_execution_id: &str,
        instruction: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.initial_instruction
            .dispatch_continuation(
                node_session_id,
                instruction,
                &format!("workflow-delegate-continuation-{node_session_id}-{child_execution_id}"),
            )
            .await
            .map(|_| ())
            .map_err(|error| {
                WorkflowRuntimeError::AgentSession(format!(
                    "continue Workflow AgentSession '{node_session_id}': {error:?}"
                ))
            })
    }

    async fn has_recoverable_conversation(
        &self,
        node_session_id: &str,
    ) -> Result<bool, WorkflowRuntimeError> {
        self.lifecycle
            .has_recoverable_conversation(node_session_id)
            .await
            .map_err(|error| {
                lifecycle_error(
                    format!("read provider conversation for '{node_session_id}'"),
                    error,
                )
            })
    }

    async fn recover_workflow_agent_session_provider(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let outcome = self
            .lifecycle
            .ensure_provider_running(
                node_session_id,
                24,
                80,
                &format!(
                    "workflow-node-provider-recovery-{node_execution_id}-{}",
                    uuid::Uuid::new_v4()
                ),
            )
            .await
            .map_err(|error| {
                lifecycle_error(
                    format!("recover provider for Workflow AgentSession '{node_session_id}'"),
                    error,
                )
            })?;
        match outcome {
            crate::usecase::agent_session::AgentSessionOpenOutcome::Attached
            | crate::usecase::agent_session::AgentSessionOpenOutcome::Resumed => Ok(()),
            _ => Err(WorkflowRuntimeError::AgentSession(format!(
                "provider for Workflow AgentSession '{node_session_id}' did not resume"
            ))),
        }
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
                lifecycle_error(format!("stop Workflow AgentSession '{node_session_id}' for NodeExecution '{node_execution_id}'"), error)
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
                launch_failure(
                    format!("rollback unattached Workflow AgentSession '{node_session_id}'"),
                    error,
                )
            })
    }
}

fn lifecycle_error(
    context: String,
    error: crate::usecase::agent_session::AgentSessionLifecycleUsecaseError,
) -> WorkflowRuntimeError {
    match error.technical_failure() {
        Some(failure) => {
            WorkflowRuntimeError::Technical(crate::domain::failure::TechnicalFailure {
                nature: failure.nature,
                message: format!("{context}: {}", failure.message),
            })
        }
        None => WorkflowRuntimeError::AgentSession(format!("{context}: {error:?}")),
    }
}

fn launch_failure(context: String, error: AgentSessionLaunchUsecaseError) -> WorkflowRuntimeError {
    match error.technical_failure() {
        Some(failure) => {
            WorkflowRuntimeError::Technical(crate::domain::failure::TechnicalFailure {
                nature: failure.nature,
                message: format!("{context}: {}", failure.message),
            })
        }
        None => WorkflowRuntimeError::AgentSession(format!("{context}: {error:?}")),
    }
}

#[cfg(test)]
#[path = "node_session_boundary_test.rs"]
mod node_session_boundary_tests;
