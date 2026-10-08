use crate::adaptor::gateway::workflow::node_session_boundary::WorkflowAgentSessionPort;
use crate::adaptor::gateway::workflow::node_session_boundary::WorkflowSessionLaunchConfig;
use crate::domain::workflow::{
    ChildEntry, ExecutionOrigin, ExecutionParentRef, NodeDefinition, NodeKind, NodeKindName,
    SequenceSpec, WorkflowDefinition, WorkflowEvent,
};
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use std::sync::Arc;
pub const TREE: &str = "00000000-0000-4000-8000-00000000e001";
pub fn definition() -> WorkflowDefinition {
    WorkflowDefinition {
        name: "wf".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            NodeDefinition {
                name: "a".to_string(),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "run".to_string(),
                kind: NodeKind::Command(crate::domain::workflow::CommandSpec {
                    command: "true".to_string(),
                    env: [(
                        crate::domain::workflow::EnvironmentVariableName::new("DOC").unwrap(),
                        crate::domain::workflow::InputParameterRef::new("document").unwrap(),
                    )]
                    .into_iter()
                    .collect(),
                }),
                input: vec![crate::domain::workflow::InputParam {
                    name: "document".to_string(),
                    contract: None,
                }],
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    entry: None,
                    children: vec![ChildEntry::reference("a"), ChildEntry::reference("run")],
                }),
                ..NodeDefinition::default()
            },
        ],
        entry: "main".to_string(),
    }
}
pub fn started_event() -> WorkflowEvent {
    WorkflowEvent::ExecutionStarted {
        repository_root: None,
        execution_id: TREE.to_string(),
        workflow_name: "wf".to_string(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Cli,
        request: "please".to_string(),
        definition: definition(),
        timestamp: 1.0,
    }
}
pub fn node_started(
    node_execution_id: &str,
    node_name: &str,
    kind: NodeKindName,
    parent: Option<ExecutionParentRef>,
    timestamp: f64,
) -> WorkflowEvent {
    WorkflowEvent::NodeStarted {
        worktree: None,
        execution_id: TREE.to_string(),
        node_execution_id: node_execution_id.to_string(),
        node_name: node_name.to_string(),
        kind,
        attempt: 1,
        parent,
        timestamp,
    }
}

pub const PREDICATE_ROUTING: &str = include_str!("fixtures/valid/predicate-routing.yml");
pub const NESTED_PREDICATE: &str = "{and: [passed, {or: [clean, skipped]}]}";
pub fn predicate_yaml(on: &str) -> String {
    PREDICATE_ROUTING.replace(NESTED_PREDICATE, on)
}

use crate::adaptor::gateway::workflow::node_session_boundary::NodeSessionInfo;
use crate::domain::provider_lifecycle::ProviderKind;
pub const EFFECT_AGENT_SESSION_ID: &str = "agent-session-effect-test";

pub struct RecordingWorkflowAgentSessions {
    pub stop_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    pub prepare_calls: Arc<std::sync::Mutex<Vec<(String, String, WorkflowSessionLaunchConfig)>>>,
    pub provider_running_checks: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    pub recovery_fails: Arc<std::sync::atomic::AtomicBool>,
    pub failing_agent_session_id: String,
}

#[async_trait::async_trait]
impl WorkflowAgentSessionPort for RecordingWorkflowAgentSessions {
    async fn has_recoverable_conversation(&self, _id: &str) -> Result<bool, WorkflowRuntimeError> {
        Ok(true)
    }

    fn is_provider_available(&self, _provider: ProviderKind) -> bool {
        true
    }

    async fn prepare_workflow_agent_session(
        &self,
        _workspace_worktree_path: &str,
        _worktree_path: &str,
        config: WorkflowSessionLaunchConfig,
        workflow_execution_id: &str,
        node_execution_id: &str,
        _initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        self.prepare_calls.lock().unwrap().push((
            workflow_execution_id.to_string(),
            node_execution_id.to_string(),
            config,
        ));
        Ok(NodeSessionInfo {
            id: EFFECT_AGENT_SESSION_ID.to_string(),
        })
    }

    async fn activate_workflow_agent_session(
        &self,
        _node_session_id: &str,
        _node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }

    async fn confirm_workflow_agent_session_attachment(
        &self,
        _node_session_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }

    async fn dispatch_continuation(
        &self,
        _node_session_id: &str,
        _child_execution_id: &str,
        _instruction: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        panic!("unexpected delegate continuation")
    }

    async fn recover_workflow_agent_session_provider(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.provider_running_checks
            .lock()
            .unwrap()
            .push((node_execution_id.to_string(), node_session_id.to_string()));
        if self
            .recovery_fails
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(WorkflowRuntimeError::AgentSession(
                "intentional provider recovery failure".to_string(),
            ));
        }
        Ok(())
    }

    async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.stop_calls
            .lock()
            .unwrap()
            .push((node_execution_id.to_string(), node_session_id.to_string()));
        if node_session_id == self.failing_agent_session_id {
            return Err(WorkflowRuntimeError::AgentSession(
                "intentional stop failure".to_string(),
            ));
        }
        Ok(())
    }

    async fn rollback_workflow_agent_session(
        &self,
        _node_session_id: &str,
        _node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct ConfiguredWorktreeGateway {
    repository: std::sync::Arc<crate::usecase::repository_usecase::RepositoryUsecase>,
    repo_paths: Vec<String>,
}

impl ConfiguredWorktreeGateway {
    pub fn new(
        repository: std::sync::Arc<crate::usecase::repository_usecase::RepositoryUsecase>,
        repo_paths: Vec<String>,
    ) -> Self {
        Self {
            repository,
            repo_paths,
        }
    }
}

impl crate::domain::workflow::ManagedWorktreeGateway for ConfiguredWorktreeGateway {
    fn resolve(
        &self,
        worktree_path: &str,
    ) -> Result<String, crate::domain::workflow::WorkflowError> {
        super::worktree_gateway::canonicalize_managed_worktree_path_inner(
            &self.repository,
            self.repo_paths.clone(),
            worktree_path.to_string(),
        )
    }
}
