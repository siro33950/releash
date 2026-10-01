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

pub(crate) struct ProviderWorkflowAgentSessionPort {
    launch: Arc<AgentSessionLaunchUsecase>,
    initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
    lifecycle: Arc<AgentSessionLifecycleUsecase>,
    availability: Arc<dyn ProviderAvailabilityReader>,
}

fn activation_error(
    node_session_id: &str,
    error: AgentSessionLaunchUsecaseError,
) -> WorkflowRuntimeError {
    launch_failure(
        format!("activate Workflow AgentSession '{node_session_id}': {error}"),
        error,
    )
}

impl ProviderWorkflowAgentSessionPort {
    pub(crate) fn new(
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
                launch_failure(format!("launch Workflow AgentSession for NodeExecution '{node_execution_id}': {error:?}"), error)
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
                    format!(
                        "confirm Workflow AgentSession attachment '{node_session_id}': {error:?}"
                    ),
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
                    format!("read provider conversation for '{node_session_id}': {error:?}"),
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
                    format!(
                        "recover provider for Workflow AgentSession '{node_session_id}': {error:?}"
                    ),
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
                lifecycle_error(format!("stop Workflow AgentSession '{node_session_id}' for NodeExecution '{node_execution_id}': {error:?}"), error)
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
                    format!(
                        "rollback unattached Workflow AgentSession '{node_session_id}': {error:?}"
                    ),
                    error,
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::ProviderAgentTerminalSpawnError;

    #[test]
    fn test_session起動停止_分類をruntimeまで保持する() {
        use crate::adaptor::presenter::connect::ConnectFailure;
        use crate::common::operation_context::OperationStopped;
        // Given
        for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
            // When
            let error = activation_error(
                "session",
                AgentSessionLaunchUsecaseError::Technical(stopped.into()),
            );
            // Then
            assert_eq!(
                error.connect_code(),
                crate::domain::failure::TechnicalFailure::from(stopped).connect_code()
            );
        }
    }

    #[test]
    fn test_session依存先の技術的失敗_性質とメッセージをruntimeまで保持する() {
        use crate::domain::agent_session::{
            ProviderAgentLaunchGatewayError, ProviderAgentTerminalGatewayError,
        };
        use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
        use crate::usecase::agent_session::AgentSessionLifecycleUsecaseError;
        // Given
        for nature in [
            TechnicalFailureNature::Transient,
            TechnicalFailureNature::TimedOut,
            TechnicalFailureNature::Cancelled,
            TechnicalFailureNature::Other,
        ] {
            let failure = TechnicalFailure {
                nature,
                message: "source failure".into(),
            };
            let launch = ProviderAgentLaunchGatewayError::Technical(failure.clone());
            let terminal = ProviderAgentTerminalGatewayError::Technical(failure.clone());
            let spawn = ProviderAgentTerminalSpawnError::Technical(failure.clone());
            // When
            let errors = [
                launch_failure(
                    "context".into(),
                    AgentSessionLaunchUsecaseError::Launch(launch.clone()),
                ),
                launch_failure(
                    "context".into(),
                    AgentSessionLaunchUsecaseError::Terminal(terminal.clone()),
                ),
                launch_failure(
                    "context".into(),
                    AgentSessionLaunchUsecaseError::TerminalSpawn(spawn.clone()),
                ),
                lifecycle_error(
                    "context".into(),
                    AgentSessionLifecycleUsecaseError::Launch(launch),
                ),
                lifecycle_error(
                    "context".into(),
                    AgentSessionLifecycleUsecaseError::Terminal(terminal),
                ),
                lifecycle_error(
                    "context".into(),
                    AgentSessionLifecycleUsecaseError::TerminalSpawn(spawn),
                ),
            ];
            // Then
            for error in errors {
                assert!(
                    matches!(error, WorkflowRuntimeError::Technical(actual) if actual == failure)
                );
            }
        }
    }

    #[test]
    fn test_session起動要求_workspaceと隔離cwdをそれぞれの項目に写す() {
        // Given
        let config = WorkflowSessionLaunchConfig {
            provider: ProviderKind::Codex,
            model: Some("model".into()),
            permission: Some(crate::domain::workflow::SessionPermission::Auto),
        };
        // When
        let request = workflow_launch_request(
            "/repo-worktrees/development",
            "/repo-worktrees/.releash-isolated/node-a1",
            config.clone(),
            "execution",
            "node",
            "implement",
        );
        // Then
        assert_eq!(request.workspace.as_str(), "/repo-worktrees/development");
        assert_eq!(
            request.worktree_path,
            "/repo-worktrees/.releash-isolated/node-a1"
        );
        assert_eq!(request.provider, config.provider);
        assert_eq!(request.model, config.model);
        assert_eq!(request.permission, config.permission);
        assert_eq!(request.workflow_execution_id, "execution");
        assert_eq!(request.node_execution_id, "node");
        assert_eq!(request.initial_instruction, "implement");
    }

    #[test]
    fn test_workflow_agent_session_activation_terminal_spawn分類をcontext付きで保持する() {
        let error = activation_error(
            "agent-session-1",
            AgentSessionLaunchUsecaseError::TerminalSpawn(
                ProviderAgentTerminalSpawnError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: "openpty failed".to_string(),
                    },
                ),
            ),
        );

        assert_eq!(error.to_string(), "openpty failed");
    }

    #[test]
    fn test_workflow_agent_session_activation_terminal以外の既存表現を維持する() {
        let error = activation_error(
            "agent-session-1",
            AgentSessionLaunchUsecaseError::Launch(
                crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Transient,
                        message: "unavailable".into(),
                    },
                ),
            ),
        );

        assert_eq!(error.to_string(), "unavailable");
    }
}

fn lifecycle_error(
    message: String,
    error: crate::usecase::agent_session::AgentSessionLifecycleUsecaseError,
) -> WorkflowRuntimeError {
    use crate::usecase::agent_session::AgentSessionLifecycleUsecaseError as E;
    match error {
        E::Launch(crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
            failure,
        ))
        | E::Terminal(
            crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(failure),
        )
        | E::TerminalSpawn(
            crate::domain::agent_session::ProviderAgentTerminalSpawnError::Technical(failure),
        ) => WorkflowRuntimeError::Technical(failure),
        _ => WorkflowRuntimeError::AgentSession(message),
    }
}

fn launch_failure(message: String, error: AgentSessionLaunchUsecaseError) -> WorkflowRuntimeError {
    match error {
        AgentSessionLaunchUsecaseError::Technical(failure)
        | AgentSessionLaunchUsecaseError::Launch(
            crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(failure),
        )
        | AgentSessionLaunchUsecaseError::Terminal(
            crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(failure),
        )
        | AgentSessionLaunchUsecaseError::TerminalSpawn(
            crate::domain::agent_session::ProviderAgentTerminalSpawnError::Technical(failure),
        ) => WorkflowRuntimeError::Technical(failure),
        _ => WorkflowRuntimeError::AgentSession(message),
    }
}
