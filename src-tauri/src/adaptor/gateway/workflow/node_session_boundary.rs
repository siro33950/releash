use std::sync::Arc;

use crate::adaptor::gateway::workflow::execution_store::ExecutionStore;
use crate::adaptor::gateway::workflow::state::RuntimeCommitSnapshot;
use crate::adaptor::gateway::workflow::workflow_host::runtime_session as workflow_runtime_session;
use crate::domain::agent_session::ProviderAvailabilityGateway;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{
    ProviderAgentInitialInstructionUsecase, ProviderAgentSessionLaunchUsecase,
    ProviderAgentWorkflowSessionLaunchRequest,
};
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::runtime_events as workflow_runtime_events;

#[async_trait::async_trait]
pub(crate) trait WorkflowAgentSessionPort: Send + Sync {
    fn is_provider_available(&self, provider: ProviderKind) -> bool;

    async fn prepare_workflow_agent_session(
        &self,
        worktree_path: &str,
        provider: ProviderKind,
        workflow_execution_id: &str,
        node_execution_id: &str,
        initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError>;

    async fn activate_workflow_agent_session(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn dispatch_initial_instruction(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
        instruction: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn rollback_workflow_agent_session(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;
}

pub(crate) struct ProviderWorkflowAgentSessionPort {
    launch: Arc<ProviderAgentSessionLaunchUsecase>,
    initial_instruction: Arc<ProviderAgentInitialInstructionUsecase>,
    availability: Arc<dyn ProviderAvailabilityGateway>,
}

impl ProviderWorkflowAgentSessionPort {
    pub(crate) fn new(
        launch: Arc<ProviderAgentSessionLaunchUsecase>,
        initial_instruction: Arc<ProviderAgentInitialInstructionUsecase>,
        availability: Arc<dyn ProviderAvailabilityGateway>,
    ) -> Self {
        Self {
            launch,
            initial_instruction,
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
        provider: ProviderKind,
        workflow_execution_id: &str,
        node_execution_id: &str,
        initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        let launched = self
            .launch
            .prepare_workflow_node(ProviderAgentWorkflowSessionLaunchRequest {
                workspace: WorkspaceIdentity::new(worktree_path),
                worktree_path: worktree_path.to_string(),
                provider,
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
            .map_err(|error| {
                WorkflowRuntimeError::AgentSession(format!(
                    "activate Workflow AgentSession '{node_session_id}': {error:?}"
                ))
            })?;
        Ok(())
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

#[async_trait::async_trait]
pub(crate) trait NodeSessionDeps: Send + Sync {
    async fn prepare_workflow_agent_session(
        &self,
        worktree_path: &str,
        provider: ProviderKind,
        workflow_execution_id: &str,
        node_execution_id: &str,
        initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError>;

    async fn activate_workflow_agent_session(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn rollback_workflow_agent_session(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn broadcast_state(&self, worktree_path: &str, snapshot: RuntimeCommitSnapshot);

    async fn append_node_session_started(
        &self,
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowRuntimeError>;
}

#[derive(Clone, Debug)]
pub(crate) struct NodeSessionInfo {
    pub(crate) id: String,
}

pub(crate) struct RealNodeSessionDeps<'a, R: tauri::Runtime> {
    pub(crate) app: &'a tauri::AppHandle<R>,
    pub(crate) agent_sessions: &'a dyn WorkflowAgentSessionPort,
    pub(crate) execution_store: &'a Arc<ExecutionStore>,
}

#[async_trait::async_trait]
impl<'a, R: tauri::Runtime> NodeSessionDeps for RealNodeSessionDeps<'a, R> {
    async fn prepare_workflow_agent_session(
        &self,
        worktree_path: &str,
        provider: ProviderKind,
        workflow_execution_id: &str,
        node_execution_id: &str,
        initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        self.agent_sessions
            .prepare_workflow_agent_session(
                worktree_path,
                provider,
                workflow_execution_id,
                node_execution_id,
                initial_instruction,
            )
            .await
    }

    async fn activate_workflow_agent_session(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.agent_sessions
            .activate_workflow_agent_session(node_session_id, node_execution_id)
            .await
    }

    async fn rollback_workflow_agent_session(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.agent_sessions
            .rollback_workflow_agent_session(node_session_id, node_execution_id)
            .await
    }

    async fn broadcast_state(&self, worktree_path: &str, snapshot: RuntimeCommitSnapshot) {
        workflow_runtime_session::broadcast_state(self.app, worktree_path, snapshot).await;
    }

    async fn append_node_session_started(
        &self,
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowRuntimeError> {
        let Some(event) =
            workflow_runtime_events::node_session_started_event_for_snapshot(snapshot)?
        else {
            return Ok(());
        };
        let state_mutations = self
            .execution_store
            .prepare_atomic_existing_snapshot_mutations(snapshot)
            .await
            .map_err(|error| {
                WorkflowRuntimeError::SessionStore(format!(
                    "prepare NodeSessionStarted projection failed: {error}"
                ))
            })?;
        crate::adaptor::gateway::workflow::event_log_writer::
            append_required_events_with_mutations_for_app_as(
                self.app,
                crate::domain::local_event::CommitOperationKind::Workflow,
                &[event],
                state_mutations,
            )
            .map_err(|error| {
                WorkflowRuntimeError::SessionStore(format!(
                    "append NodeSessionStarted failed: {error}"
                ))
            })
    }
}
