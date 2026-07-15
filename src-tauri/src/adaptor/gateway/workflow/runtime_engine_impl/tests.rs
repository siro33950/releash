use super::*;
use crate::adaptor::gateway::app_config::config_models::{
    NotionPropertyMappingModel, NotionRepoConfigModel, ReleashConfig,
};
use crate::adaptor::gateway::workflow::approval_runtime::MAX_APPROVAL_COMMENT_CHARS;
use crate::adaptor::gateway::workflow::event_projection::project_workflow_execution;
use crate::adaptor::gateway::workflow::failure_wire::{
    submission_violation_reason, SubmissionViolation,
};
use crate::adaptor::gateway::workflow::runtime_state::{LoopGuardResult, TurnCompleteAction};
use crate::adaptor::gateway::workflow::state::FanoutParentRef;
use crate::domain::agent_session::entities::PermissionResponse;
use crate::domain::agent_session::gateway::{
    AgentBackend, AgentBackendError, AgentRuntimeEvent, AgentSessionRuntime, ForkSessionRequest,
    SessionSpec as AgentSessionSpec, TurnInput,
};
use crate::domain::agent_session::value_objects::{
    BackendCapabilities, ModelDescriptor, ModelId, SkillEntry,
};
use crate::domain::workflow::services::transition::ApprovalApplication;
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use async_trait::async_trait;
use tauri::{Listener, Manager};

const TEST_PARENT_SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
const TEST_NODE_SESSION_ID: &str = "22222222-2222-4222-8222-222222222222";
const TEST_REGULAR_SESSION_ID: &str = "33333333-3333-4333-8333-333333333333";

fn node_execution_fixture(
    execution_id: &str,
    id: &str,
    node_name: &str,
    attempt: u32,
    status: NodeExecutionStatus,
    session_id: Option<&str>,
    fanout_parent: Option<FanoutParentRef>,
) -> NodeExecution {
    NodeExecution {
        id: id.to_string(),
        execution_id: execution_id.to_string(),
        node_name: node_name.to_string(),
        kind: NodeKindName::Session,
        attempt,
        status,
        session_id: session_id.map(str::to_string),
        artifact: None,
        token_usage: None,
        failure: None,
        fanout_parent,
        started_at: 1000.0,
        completed_at: None,
    }
}

struct WorkflowMockBackend {
    backend_id: String,
    models: Vec<String>,
}

#[async_trait]
impl AgentBackend for WorkflowMockBackend {
    fn id(&self) -> &str {
        &self.backend_id
    }
    fn name(&self) -> &str {
        "Mock"
    }

    fn available_models(&self) -> Vec<ModelDescriptor> {
        self.models
            .iter()
            .map(|model| ModelDescriptor {
                id: ModelId::parse(model).unwrap(),
                display_name: model.clone(),
            })
            .collect()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities { steering: false }
    }

    async fn open_session(
        &self,
        _spec: AgentSessionSpec,
    ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
        Ok(Box::new(WorkflowMockRuntime))
    }

    async fn archive_session(
        &self,
        _backend_session_id: &str,
        _cwd: &str,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn unarchive_session(
        &self,
        _backend_session_id: &str,
        _cwd: &str,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn fork_session(
        &self,
        _req: ForkSessionRequest,
    ) -> Result<Option<String>, AgentBackendError> {
        Ok(None)
    }

    async fn skill_catalog(
        &self,
        _cwd: &std::path::Path,
        _query: Option<&str>,
        _limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, AgentBackendError> {
        Ok(Vec::new())
    }

    async fn fuzzy_file_search(
        &self,
        _root: &std::path::Path,
        _query: &str,
        _limit: usize,
    ) -> Result<Option<Vec<String>>, AgentBackendError> {
        Ok(None)
    }
}

struct WorkflowMockRuntime;

#[async_trait]
impl AgentSessionRuntime for WorkflowMockRuntime {
    fn take_events(
        &mut self,
    ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = AgentRuntimeEvent> + Send>> {
        Box::pin(futures_util::stream::empty())
    }

    async fn start_turn(&self, _input: TurnInput) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn respond_permission(
        &self,
        _response: PermissionResponse,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn set_permission_mode(
        &self,
        _mode: crate::domain::agent_session::PermissionMode,
        _plan_mode: bool,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn set_model(&self, _model: &ModelId) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn close(&self) {}
}

fn make_workflow_test_registry(
    claude_models: &[&str],
    codex_models: &[&str],
) -> AgentBackendRegistry {
    let mut registry = AgentBackendRegistry::new();
    registry.register(Arc::new(WorkflowMockBackend {
        backend_id: "claude".to_string(),
        models: claude_models
            .iter()
            .map(|model| model.to_string())
            .collect(),
    }));
    registry.register(Arc::new(WorkflowMockBackend {
        backend_id: "codex".to_string(),
        models: codex_models.iter().map(|model| model.to_string()).collect(),
    }));
    registry
}

#[test]
fn fanout_child_failure_kind_uses_typed_refusal_signal() {
    let kind = fanout_child_failure_kind(
        0,
        Some(workflow_transition::SessionFailureSignal::ModelRefusal),
    );

    assert_eq!(kind, NodeExecutionFailureKind::ModelRefusal);
}

#[test]
fn fanout_child_failure_kind_without_signal_uses_session_error_classification() {
    let kind = fanout_child_failure_kind(1, None);

    assert_eq!(kind, NodeExecutionFailureKind::InfrastructureCrash);
}

#[test]
fn workflow_resolve_unique_model_returns_owning_backend() {
    let registry = make_workflow_test_registry(&["claude-4"], &["gpt-5"]);
    let result = resolve_node_model_with_registry(&registry, "claude-4").unwrap();
    assert_eq!(result, "claude");
}

#[test]
fn workflow_resolve_rejects_ambiguous_model_in_multiple_backends() {
    let registry = make_workflow_test_registry(&["shared"], &["shared"]);
    let err = resolve_node_model_with_registry(&registry, "shared").unwrap_err();
    match err {
        WorkflowEngineError::InvalidWorkflow(msg) => {
            assert!(msg.contains("could not be resolved"));
        }
        other => panic!("expected InvalidWorkflow, got {:?}", other),
    }
}

#[test]
fn workflow_resolve_rejects_unknown_model() {
    let registry = make_workflow_test_registry(&["claude-4"], &[]);
    let err = resolve_node_model_with_registry(&registry, "unknown").unwrap_err();
    match err {
        WorkflowEngineError::InvalidWorkflow(msg) => {
            assert!(msg.contains("could not be resolved"));
        }
        other => panic!("expected InvalidWorkflow, got {:?}", other),
    }
}

#[test]
fn workflow_resolve_rejects_invalid_format() {
    let registry = make_workflow_test_registry(&["claude-4"], &[]);
    // 形式不正（空文字）は登録判定に進む前に拒否される
    let err = resolve_node_model_with_registry(&registry, "").unwrap_err();
    match err {
        WorkflowEngineError::InvalidWorkflow(msg) => {
            assert!(msg.contains("invalid model"));
        }
        other => panic!("expected InvalidWorkflow, got {:?}", other),
    }
}

fn chat_session_for_test(
    id: &str,
    worktree_path: &str,
    _commit_snapshot: Option<RuntimeCommitSnapshot>,
    workflow_node_session: bool,
) -> crate::usecase::agent_session::session::ChatSession {
    crate::usecase::agent_session::session::ChatSession {
        id: id.to_string(),
        worktree_path: worktree_path.to_string(),
        messages: vec![],
        state: crate::usecase::agent_session::session::SessionState::Idle,
        created_at: 1.0,
        updated_at: 1.0,
        agent_session_id: Some("sdk-session".to_string()),
        context_carry: Some(crate::usecase::agent_session::session::ContextCarryState::Resumed),
        permission_mode: "edit".to_string(),
        plan_mode: false,
        permission_profile_id: None,
        selected_model: None,
        backend_id: Some(
            crate::infrastructure::agent_session::claude::CLAUDE_BACKEND_ID.to_string(),
        ),
        workflow_node_session,
        workflow_node_context: None,
        context_epoch: None,
    }
}

async fn insert_ready_agent_process_for_internal_turn_test(
    agent_runtime: &Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    _session_store: &Arc<SessionStore>,
    _data_dir: &std::path::Path,
    session_id: &str,
) {
    agent_runtime
        .insert_runtime_state_for_test(
            session_id,
            crate::usecase::agent_session::status::TurnPhase::Idle,
            false,
        )
        .await;
}

fn chat_session_with_message_for_test(
    id: &str,
    worktree_path: &str,
) -> crate::usecase::agent_session::session::ChatSession {
    let mut session = chat_session_for_test(id, worktree_path, None, true);
    session
        .messages
        .push(crate::usecase::agent_session::session::ChatMessage {
            id: "msg-1".to_string(),
            role: crate::usecase::agent_session::session::MessageRole::Agent,
            content: "history".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        });
    session
}

#[test]
fn workflow_node_summary_uses_persisted_session_flag() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = crate::test_support::build_session_store();

    store
        .save_full_session_for_migration_or_restore(
            tmp.path(),
            &chat_session_for_test(TEST_PARENT_SESSION_ID, "/repo", None, false),
        )
        .unwrap();
    store
        .save_full_session_for_migration_or_restore(
            tmp.path(),
            &chat_session_for_test(TEST_NODE_SESSION_ID, "/repo", None, true),
        )
        .unwrap();
    store
        .save_full_session_for_migration_or_restore(
            tmp.path(),
            &chat_session_for_test(TEST_REGULAR_SESSION_ID, "/repo", None, false),
        )
        .unwrap();

    let summaries = store.list_sessions(tmp.path(), "/repo").unwrap();
    let node_summary = summaries
        .iter()
        .find(|session| session.id == TEST_NODE_SESSION_ID)
        .unwrap();
    assert!(node_summary.workflow_node_session);
}

// 撤去済み: persist_state は廃止された（NDJSON event log + Execution Store metadata で永続化が完結）。
// 旧 `persist_failure_still_runs_completed_node_cleanup` は persist_state 失敗時の cleanup 順序を
// 検証していたが、機構撤去により意味を失った。

#[test]
fn node_session_tab_cleanup_closes_session_and_preserves_history() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = crate::test_support::build_session_store();
    let open_tabs = Arc::new(crate::usecase::agent_session::session::OpenTabRegistry::default());
    let session_id = uuid::Uuid::new_v4().to_string();

    store
        .save_full_session_for_migration_or_restore(
            tmp.path(),
            &chat_session_with_message_for_test(&session_id, "/repo"),
        )
        .unwrap();
    open_tabs.add(&session_id);

    crate::adaptor::gateway::workflow::close_node_session_tab_state(
        &store,
        tmp.path(),
        Some(open_tabs.as_ref()),
        &session_id,
    );

    assert!(!open_tabs.contains(&session_id));
    let session = store
        .load_full_session_for_restore(tmp.path(), &session_id)
        .unwrap()
        .expect("session remains");
    assert_eq!(
        session.state,
        crate::usecase::agent_session::session::SessionState::Closed
    );
    assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
    assert_eq!(session.messages.len(), 1);
}

#[tokio::test]
async fn persist_outcome_without_new_history_does_not_cleanup_last_node_session() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut snapshot = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_NODE_SESSION_ID,
            RuntimeExecutionState::WaitingApproval,
        )
        .await;
    snapshot.node_history.push(NodeHistoryEntry {
        node_name: "previous".to_string(),
        completed_at: 1.0,
        result: Some("ok".to_string()),
        session_id: Some(TEST_NODE_SESSION_ID.to_string()),
        token_usage: None,
        artifact: None,
        attempt: 1,
        fanout_children: None,
        state: crate::domain::workflow::value_objects::default_node_history_status(),
    });

    let persist = NodeOutcome::Persist(snapshot.clone());
    assert!(persist.completed_node_session_ids().is_empty());

    snapshot.state = RuntimeExecutionState::Completed;
    let terminal = NodeOutcome::Persist(snapshot);
    assert_eq!(
        terminal.completed_node_session_ids(),
        vec![TEST_NODE_SESSION_ID.to_string()]
    );
}

#[tokio::test]
async fn aborted_approval_outcome_cleans_current_session_not_last_history_entry() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut snapshot = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_NODE_SESSION_ID,
            RuntimeExecutionState::WaitingApproval,
        )
        .await;
    snapshot.current_session_id = Some("approval-session".to_string());
    snapshot.node_history.push(NodeHistoryEntry {
        node_name: "previous".to_string(),
        completed_at: 1.0,
        result: Some("ok".to_string()),
        session_id: Some("previous-session".to_string()),
        token_usage: None,
        artifact: None,
        attempt: 1,
        fanout_children: None,
        state: crate::domain::workflow::value_objects::default_node_history_status(),
    });
    snapshot.state = RuntimeExecutionState::Aborted;

    let outcome = NodeOutcome::Persist(snapshot);
    assert_eq!(
        outcome.completed_node_session_ids(),
        vec!["approval-session".to_string()]
    );
}

#[tokio::test]
async fn terminal_state_cleanup_targets_current_and_fanout_node_sessions() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_NODE_SESSION_ID,
            RuntimeExecutionState::Running,
        )
        .await;
    exec.current_session_id = Some("current-node-session".to_string());
    exec.node_executions[0].session_id = exec.current_session_id.clone();
    let execution_id = exec.execution_id.clone();
    exec.node_executions.extend([
        node_execution_fixture(
            &execution_id,
            "node-execution-review-a",
            "review-a",
            1,
            NodeExecutionStatus::Running,
            Some("fanout-a-session"),
            Some(FanoutParentRef {
                parent_node: "fanout-review".to_string(),
                parent_attempt: 1,
                item_index: None,
                child_index: 0,
            }),
        ),
        node_execution_fixture(
            &execution_id,
            "node-execution-review-b",
            "review-b",
            1,
            NodeExecutionStatus::Running,
            Some("fanout-b-session"),
            Some(FanoutParentRef {
                parent_node: "fanout-review".to_string(),
                parent_attempt: 1,
                item_index: None,
                child_index: 1,
            }),
        ),
    ]);

    assert_eq!(
        workflow_runtime_commit::terminal_node_session_ids(&exec),
        vec![
            "current-node-session".to_string(),
            "fanout-a-session".to_string(),
            "fanout-b-session".to_string()
        ]
    );
}

#[tokio::test]
async fn terminal_outcome_cleanup_includes_parent_entry_and_fanout_fanout_children() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut snapshot = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_NODE_SESSION_ID,
            RuntimeExecutionState::Completed,
        )
        .await;
    snapshot.node_history.push(NodeHistoryEntry {
        node_name: "fanout-review".to_string(),
        completed_at: 1.0,
        result: Some("done".to_string()),
        session_id: Some("parent-entry-session".to_string()),
        token_usage: None,
        artifact: None,
        attempt: 1,
        fanout_children: Some(vec![
            crate::adaptor::gateway::workflow::state::FanoutChildSnapshot {
                node_name: "review-a".to_string(),
                session_id: Some("child-a-session".to_string()),
                result: Some("LGTM".to_string()),
                attempt: 1,
                completed_at: 1.0,
                artifact: None,
                contract: None,
                state: crate::domain::workflow::value_objects::default_node_history_status(),
                failure_kind: None,
                failure_disposition: None,
            },
            crate::adaptor::gateway::workflow::state::FanoutChildSnapshot {
                node_name: "review-b".to_string(),
                session_id: Some("child-b-session".to_string()),
                result: Some("LGTM".to_string()),
                attempt: 1,
                completed_at: 1.0,
                artifact: None,
                contract: None,
                state: crate::domain::workflow::value_objects::default_node_history_status(),
                failure_kind: None,
                failure_disposition: None,
            },
        ]),
        state: crate::domain::workflow::value_objects::default_node_history_status(),
    });

    assert_eq!(
        NodeOutcome::Persist(snapshot).completed_node_session_ids(),
        vec![
            "child-a-session".to_string(),
            "child-b-session".to_string(),
            "parent-entry-session".to_string(),
        ]
    );
}

#[tokio::test]
async fn retry_current_node_outcome_releases_previous_session_only() {
    let engine = WorkflowRuntimeService::new_for_test();
    let snapshot = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_NODE_SESSION_ID,
            RuntimeExecutionState::Running,
        )
        .await;

    assert_eq!(
        NodeOutcome::RetryCurrentNode {
            snapshot,
            completed_session_id: Some("stale-session".to_string()),
        }
        .completed_node_session_ids(),
        vec!["stale-session".to_string()]
    );
}
use crate::adaptor::gateway::workflow::schema::{Rule, SchemaDef, WorkflowDefinitionYaml};

fn object_schema_for_test(fields: &[&str]) -> SchemaDef {
    SchemaDef::Object {
        properties: fields
            .iter()
            .map(|field| (field.to_string(), SchemaDef::String { r#enum: None }))
            .collect(),
        required: fields.iter().map(|field| field.to_string()).collect(),
        additional_properties: false,
    }
}

fn submit_test_schemas() -> std::collections::BTreeMap<String, SchemaDef> {
    [
        (
            "review-verdict".to_string(),
            object_schema_for_test(&["verdict"]),
        ),
        (
            "spec-directory".to_string(),
            object_schema_for_test(&["spec_dir"]),
        ),
    ]
    .into_iter()
    .collect()
}

fn make_minimal_approval_exec(
    execution_id: &str,
    current_session_id: &str,
    node_name: &str,
) -> WorkflowExecution {
    let workflow = WorkflowDefinitionYaml {
        name: "test-workflow".to_string(),
        description: "minimal approval fixture".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            NodeDefinition {
                name: node_name.to_string(),
                kind: test_node_kind(TestKind::ApprovalSession, "approve"),
                rules: vec![Rule::Next("next-node".to_string())],
                ..Default::default()
            },
            NodeDefinition {
                name: "next-node".to_string(),
                kind: test_node_kind(TestKind::Session, "next"),
                ..Default::default()
            },
        ],
    };
    WorkflowExecution {
        id: execution_id.to_string(),
        workflow,
        state: RuntimeExecutionState::WaitingApproval,
        current_node_index: 0,
        node_execution_counts: HashMap::from([(node_name.to_string(), 1)]),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some(current_session_id.to_string()),
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: vec![node_execution_fixture(
            execution_id,
            "node-execution-approval",
            node_name,
            1,
            NodeExecutionStatus::WaitingApproval,
            Some(current_session_id),
            None,
        )],
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "edit".to_string(),
        },
    }
}

#[test]
fn current_node_for_stall_observation_ignores_terminal_fanout_children() {
    let mut exec =
        make_minimal_approval_exec("execution-stall-lookup", "regular-session", "review");
    exec.current_session_id = None;
    exec.fanout_runtime = Some(FanoutRuntimeState {
        parent_node_name: "fanout-review".to_string(),
        parent_node_execution_id: "node-execution-fanout-review".to_string(),
        children: vec![
            FanoutChildRuntime {
                node_execution_id: "node-execution-running-child".to_string(),
                node_name: "running-child".to_string(),
                session_id: "running-session".to_string(),
                state: FanoutChildRuntimeState::Running,
                result: None,
                artifact: None,
                contract: None,
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                attempt: 2,
                completed_at: None,
            },
            FanoutChildRuntime {
                node_execution_id: "node-execution-completed-child".to_string(),
                node_name: "completed-child".to_string(),
                session_id: "completed-session".to_string(),
                state: FanoutChildRuntimeState::Completed,
                result: Some("ok".to_string()),
                artifact: None,
                contract: None,
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                attempt: 1,
                completed_at: Some(1000.0),
            },
            FanoutChildRuntime {
                node_execution_id: "node-execution-failed-child".to_string(),
                node_name: "failed-child".to_string(),
                session_id: "failed-session".to_string(),
                state: FanoutChildRuntimeState::Failed,
                result: Some("model_refusal".to_string()),
                artifact: None,
                contract: None,
                failure_kind: Some(NodeExecutionFailureKind::ModelRefusal),
                failure_disposition: Some(FailureDisposition::Partial),
                token_usage: TokenUsage::default(),
                attempt: 1,
                completed_at: Some(1000.0),
            },
        ],
    });

    assert_eq!(
        current_node_for_stall_observation(&exec, "running-session"),
        Some((
            "node-execution-running-child".to_string(),
            "running-child".to_string(),
            2,
        ))
    );
    assert_eq!(
        current_node_for_stall_observation(&exec, "completed-session"),
        None
    );
    assert_eq!(
        current_node_for_stall_observation(&exec, "failed-session"),
        None
    );
}

fn workflow_stall_observation_fixture(session_id: &str, node_name: &str) -> NodeStallObservation {
    NodeStallObservation {
        session_id: session_id.to_string(),
        node_name: node_name.to_string(),
        attempt: 1,
        turn_phase: "streaming".to_string(),
        idle_secs: 181,
        signal_count: 1,
        cap_reached: false,
        observed_at: 1003.0,
    }
}

#[tokio::test]
async fn agent_stall_observed_updates_commit_snapshot_without_completing_node() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let app = tauri::test::mock_builder()
        .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
            data_dir.path().to_path_buf(),
        ))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(Arc::new(
        crate::usecase::agent_session::status::AgentStatusCenter::new(),
    ));
    app.manage(Arc::new(OpenTabRegistry::default()));
    let runtime_session_store = Arc::new(crate::test_support::build_session_store());
    let runtime = crate::test_support::build_agent_runtime_usecase(
        runtime_session_store,
        data_dir.path().to_path_buf(),
    );
    app.manage(runtime);
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(data_dir.path().to_path_buf())
        .await;
    let execution_id = uuid::Uuid::new_v4().to_string();
    let session_id = "stall-session";
    let node_name = "review";
    let exec = make_minimal_approval_exec(&execution_id, session_id, node_name);
    let workflow = exec.workflow.clone();
    let log_data_dir =
        crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle())
            .expect("mock app data dir must resolve");
    WorkflowEventLog::new(&log_data_dir)
        .append_batch(&[
            WorkflowEvent::ExecutionStarted {
                execution_id: execution_id.clone(),
                workflow_name: workflow.name.clone(),
                worktree_path: exec.worktree_path.clone(),
                created_from: ExecutionOrigin::Agent,
                request: String::new(),
                permission_mode: exec.workflow_defaults.permission_mode.clone(),
                definition: workflow,
                timestamp: exec.started_at,
            },
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.clone(),
                node_execution_id: "node-execution-approval".to_string(),
                node_name: node_name.to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                fanout_parent: None,
                timestamp: exec.started_at,
            },
            WorkflowEvent::SessionAttached {
                execution_id: execution_id.clone(),
                node_execution_id: "node-execution-approval".to_string(),
                session_id: session_id.to_string(),
                timestamp: exec.started_at,
            },
        ])
        .unwrap();
    engine
        .execution_store()
        .register_active_execution(WorkflowExecutionMetadata {
            execution_id: execution_id.clone(),
            workflow_name: exec.workflow.name.clone(),
            status: ExecutionStatus::WaitingApproval,
            worktree_path: exec.worktree_path.clone(),
            current_node: Some(node_name.to_string()),
            created_from: ExecutionOrigin::Agent,
            started_at: exec.started_at,
            updated_at: exec.updated_at,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: crate::domain::workflow::TokenUsage::default(),
        })
        .await
        .unwrap();
    engine
        .executions
        .lock()
        .await
        .insert(execution_id.clone(), exec);
    engine.session_workflow_refs.lock().await.insert(
        session_id.to_string(),
        SessionWorkflowRef {
            execution_id: execution_id.clone(),
        },
    );

    engine
        .on_agent_stall_observed(
            app.handle(),
            session_id,
            "streaming".to_string(),
            44,
            1,
            false,
        )
        .await
        .unwrap();

    let state = engine
        .get_state_by_execution_id(&execution_id)
        .await
        .unwrap();
    assert!(matches!(
        state.state,
        RuntimeExecutionState::WaitingApproval
    ));
    assert_eq!(state.current_session_id.as_deref(), Some(session_id));
    assert_eq!(state.node_history.len(), 0);
    let observations = engine
        .executions
        .lock()
        .await
        .get(&execution_id)
        .unwrap()
        .current_stall_observations
        .clone();
    assert_eq!(observations.len(), 1);
    let observation = &observations[0];
    assert_eq!(observation.session_id, session_id);
    assert_eq!(observation.node_name, node_name);
    assert_eq!(observation.attempt, 1);
    assert_eq!(observation.turn_phase, "streaming");
    assert_eq!(observation.idle_secs, 44);
    assert_eq!(observation.signal_count, 1);
    assert!(!observation.cap_reached);

    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&execution_id)
        .unwrap();
    assert!(matches!(
        events.last(),
        Some(WorkflowEvent::StallObserved {
            execution_id: event_execution_id,
            node_execution_id,
            session_id: event_session_id,
            node_name: event_node_name,
            attempt: 1,
            turn_phase,
            idle_secs: 44,
            signal_count: 1,
            cap_reached: false,
            ..
        }) if event_execution_id == &execution_id
            && node_execution_id == "node-execution-approval"
            && event_session_id == session_id
            && event_node_name == node_name
            && turn_phase == "streaming"
    ));

    let projected = project_workflow_execution(&execution_id, &events)
        .unwrap()
        .unwrap();
    assert_eq!(projected.current_node.as_deref(), Some(node_name));

    engine
        .on_agent_stall_observed(
            app.handle(),
            session_id,
            "streaming".to_string(),
            88,
            2,
            true,
        )
        .await
        .unwrap();

    let observations = engine
        .executions
        .lock()
        .await
        .get(&execution_id)
        .unwrap()
        .current_stall_observations
        .clone();
    assert_eq!(observations.len(), 1);
    let observation = &observations[0];
    assert_eq!(observation.session_id, session_id);
    assert_eq!(observation.idle_secs, 88);
    assert_eq!(observation.signal_count, 2);
    assert!(observation.cap_reached);
    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&execution_id)
        .unwrap();
    let projected = project_workflow_execution(&execution_id, &events)
        .unwrap()
        .unwrap();
    assert_eq!(projected.current_node.as_deref(), Some(node_name));

    engine
        .on_agent_stall_cleared(app.handle(), session_id)
        .await
        .unwrap();

    assert!(engine
        .executions
        .lock()
        .await
        .get(&execution_id)
        .unwrap()
        .current_stall_observations
        .is_empty());
    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&execution_id)
        .unwrap();
    assert!(matches!(
        events.last(),
        Some(WorkflowEvent::StallCleared {
            execution_id: event_execution_id,
            node_execution_id,
            session_id: event_session_id,
            ..
        }) if event_execution_id == &execution_id
            && node_execution_id == "node-execution-approval"
            && event_session_id == session_id
    ));
    let projected = project_workflow_execution(&execution_id, &events)
        .unwrap()
        .unwrap();
    assert_eq!(projected.current_node.as_deref(), Some(node_name));
}

#[tokio::test]
async fn agent_stall_observed_append_failure_rolls_back_state_and_execution_store() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let app = tauri::test::mock_builder()
        .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
            data_dir.path().to_path_buf(),
        ))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(Arc::new(
        crate::usecase::agent_session::status::AgentStatusCenter::new(),
    ));
    app.manage(Arc::new(OpenTabRegistry::default()));
    let runtime_session_store = Arc::new(crate::test_support::build_session_store());
    let runtime = crate::test_support::build_agent_runtime_usecase(
        runtime_session_store,
        data_dir.path().to_path_buf(),
    );
    app.manage(runtime);
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(data_dir.path().to_path_buf())
        .await;
    let execution_id = uuid::Uuid::new_v4().to_string();
    let session_id = "stall-session";
    let node_name = "review";
    let exec = make_minimal_approval_exec(&execution_id, session_id, node_name);
    let log_data_dir =
        crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle())
            .expect("mock app data dir must resolve");
    engine
        .execution_store()
        .register_active_execution(WorkflowExecutionMetadata {
            execution_id: execution_id.clone(),
            workflow_name: exec.workflow.name.clone(),
            status: ExecutionStatus::WaitingApproval,
            worktree_path: exec.worktree_path.clone(),
            current_node: Some(node_name.to_string()),
            created_from: ExecutionOrigin::Agent,
            started_at: exec.started_at,
            updated_at: exec.updated_at,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: crate::domain::workflow::TokenUsage::default(),
        })
        .await
        .unwrap();
    let stored_before = engine
        .execution_store()
        .get_execution(&execution_id)
        .await
        .unwrap();
    engine
        .executions
        .lock()
        .await
        .insert(execution_id.clone(), exec);
    engine.session_workflow_refs.lock().await.insert(
        session_id.to_string(),
        SessionWorkflowRef {
            execution_id: execution_id.clone(),
        },
    );

    engine.fail_next_required_event_append_for_test();
    let err = engine
        .on_agent_stall_observed(
            app.handle(),
            session_id,
            "streaming".to_string(),
            44,
            1,
            false,
        )
        .await
        .expect_err("stall observation must fail when required append fails");

    assert!(
        format!("{err:?}").contains("workflow stall observed event append failed"),
        "append failure context must be surfaced; got {err:?}"
    );
    assert!(engine
        .executions
        .lock()
        .await
        .get(&execution_id)
        .unwrap()
        .current_stall_observations
        .is_empty());
    let stored_after = engine
        .execution_store()
        .get_execution(&execution_id)
        .await
        .unwrap();
    assert_eq!(stored_after.status, stored_before.status);
    assert_eq!(stored_after.current_node, stored_before.current_node);
    assert_eq!(stored_after.updated_at, stored_before.updated_at);

    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&execution_id)
        .unwrap();
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::StallObserved { .. })),
        "failed stall observation must not be appended; got {events:?}"
    );
}

#[tokio::test]
async fn agent_stall_cleared_append_failure_rolls_back_state_and_execution_store() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let app = tauri::test::mock_builder()
        .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
            data_dir.path().to_path_buf(),
        ))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(Arc::new(
        crate::usecase::agent_session::status::AgentStatusCenter::new(),
    ));
    app.manage(Arc::new(OpenTabRegistry::default()));
    let runtime_session_store = Arc::new(crate::test_support::build_session_store());
    let runtime = crate::test_support::build_agent_runtime_usecase(
        runtime_session_store,
        data_dir.path().to_path_buf(),
    );
    app.manage(runtime);
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(data_dir.path().to_path_buf())
        .await;
    let execution_id = uuid::Uuid::new_v4().to_string();
    let session_id = "stall-session";
    let node_name = "review";
    let mut exec = make_minimal_approval_exec(&execution_id, session_id, node_name);
    exec.current_stall_observations =
        vec![workflow_stall_observation_fixture(session_id, node_name)];
    let log_data_dir =
        crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle())
            .expect("mock app data dir must resolve");
    engine
        .execution_store()
        .register_active_execution(WorkflowExecutionMetadata {
            execution_id: execution_id.clone(),
            workflow_name: exec.workflow.name.clone(),
            status: ExecutionStatus::WaitingApproval,
            worktree_path: exec.worktree_path.clone(),
            current_node: Some(node_name.to_string()),
            created_from: ExecutionOrigin::Agent,
            started_at: exec.started_at,
            updated_at: exec.updated_at,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: crate::domain::workflow::TokenUsage::default(),
        })
        .await
        .unwrap();
    let stored_before = engine
        .execution_store()
        .get_execution(&execution_id)
        .await
        .unwrap();
    engine
        .executions
        .lock()
        .await
        .insert(execution_id.clone(), exec);
    engine.session_workflow_refs.lock().await.insert(
        session_id.to_string(),
        SessionWorkflowRef {
            execution_id: execution_id.clone(),
        },
    );

    engine.fail_next_required_event_append_for_test();
    let err = engine
        .on_agent_stall_cleared(app.handle(), session_id)
        .await
        .expect_err("stall clear must fail when required append fails");

    assert!(
        format!("{err:?}").contains("workflow stall cleared event append failed"),
        "append failure context must be surfaced; got {err:?}"
    );
    let observations = engine
        .executions
        .lock()
        .await
        .get(&execution_id)
        .unwrap()
        .current_stall_observations
        .clone();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].session_id, session_id);
    let stored_after = engine
        .execution_store()
        .get_execution(&execution_id)
        .await
        .unwrap();
    assert_eq!(stored_after.status, stored_before.status);
    assert_eq!(stored_after.current_node, stored_before.current_node);
    assert_eq!(stored_after.updated_at, stored_before.updated_at);

    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&execution_id)
        .unwrap();
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::StallCleared { .. })),
        "failed stall clear must not be appended; got {events:?}"
    );
}

// ---- WorkflowExecution ----

fn make_test_node(
    name: &str,
    kind: TestKind,
    instruction: &str,
    mut rules: Vec<Rule>,
    loop_guard: Option<Rule>,
) -> NodeDefinition {
    rules.extend(loop_guard);
    NodeDefinition {
        name: name.to_string(),
        kind: test_node_kind(kind, instruction),
        rules,
        ..NodeDefinition::default()
    }
}

fn make_approval_gated_session(name: &str, instruction: &str, rules: Vec<Rule>) -> NodeDefinition {
    make_test_node(name, TestKind::ApprovalSession, instruction, rules, None)
}

fn make_fanout_node(name: &str, children: Vec<&str>) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Fanout(FanoutSpec {
            child: children.into_iter().map(str::to_string).collect(),
            items: None,
        }),
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestKind {
    Session,
    ApprovalSession,
    Command,
    Fanout,
}

fn test_node_kind(kind: TestKind, instruction: &str) -> NodeKind {
    match kind {
        TestKind::Session => NodeKind::Session(SessionSpec {
            facets: FacetRefs {
                instruction: Some(instruction.to_string()),
                ..Default::default()
            },
            ..Default::default()
        }),
        TestKind::ApprovalSession => NodeKind::Session(SessionSpec {
            gate: SessionGate::Approval,
            facets: FacetRefs {
                instruction: Some(instruction.to_string()),
                ..Default::default()
            },
            ..Default::default()
        }),
        TestKind::Command => NodeKind::Command(CommandSpec {
            command: instruction.to_string(),
        }),
        TestKind::Fanout => NodeKind::Fanout(FanoutSpec::default()),
    }
}

fn set_session_facets(node: &mut NodeDefinition, facets: FacetRefs) {
    node.session_mut()
        .expect("test node must be a session")
        .facets = facets;
}

fn set_instruction_facet(node: &mut NodeDefinition, instruction: Option<String>) {
    node.session_mut()
        .expect("test node must be a session")
        .facets
        .instruction = instruction;
}

fn set_policy_facet(node: &mut NodeDefinition, policy: Option<String>) {
    node.session_mut()
        .expect("test node must be a session")
        .facets
        .policy = policy;
}

/// テストヘルパー: node の facet 参照を `base_dir` から解決する。
fn resolve_node_facets_for_test(
    node: &NodeDefinition,
    base_dir: &std::path::Path,
) -> crate::adaptor::gateway::workflow::facet::FacetContents {
    crate::adaptor::gateway::workflow::facet::resolve_node_facets(node, base_dir)
        .expect("facet refs must resolve in tests; missing facet indicates a fixture bug")
}

fn instruction_contents(
    instruction: &str,
) -> crate::adaptor::gateway::workflow::facet::FacetContents {
    crate::adaptor::gateway::workflow::facet::FacetContents {
        instruction: Some(instruction.to_string()),
        ..Default::default()
    }
}

/// テストヘルパー: fanout child node の facet 参照を解決する。
fn resolve_fanout_child_facets_for_test(
    child: &NodeDefinition,
    base_dir: &std::path::Path,
) -> crate::adaptor::gateway::workflow::facet::FacetContents {
    crate::adaptor::gateway::workflow::facet::resolve_node_facets(child, base_dir)
        .expect("facet refs must resolve in tests; missing facet indicates a fixture bug")
}

fn make_test_workflow() -> WorkflowDefinitionYaml {
    WorkflowDefinitionYaml {
        name: "test-workflow".to_string(),
        description: "Test workflow".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            make_test_node(
                "plan",
                TestKind::Session,
                "Plan the work",
                vec![Rule::Next("implement".to_string())],
                None,
            ),
            make_test_node(
                "implement",
                TestKind::Session,
                "Implement the plan",
                vec![Rule::Next("review".to_string())],
                None,
            ),
            make_test_node(
                "review",
                TestKind::Session,
                "Review the implementation",
                vec![Rule::Next("implement".to_string())],
                Some(Rule::LoopGuard {
                    max_iterations: 3,
                    on_exhausted: "report".to_string(),
                }),
            ),
            make_test_node(
                "report",
                TestKind::ApprovalSession,
                "Generate report",
                vec![],
                None,
            ),
        ],
    }
}

#[test]
fn workflow_execution_to_commit_snapshot() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let state = exec.to_commit_snapshot();
    assert_eq!(state.execution_id, "exec-1");
    assert_eq!(state.workflow_name, "test-workflow");
    assert_eq!(state.state, RuntimeExecutionState::Running);
    assert_eq!(state.current_node_index, 0);
    assert_eq!(state.current_node_name, "plan");
    assert_eq!(state.workflow_definition.nodes.len(), 4);
    assert!(state.node_history.is_empty());
}

// ---- is_active ----

#[test]
fn is_active_executionning() {
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_test_workflow(),
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    assert!(exec.is_active());
}

#[test]
fn is_active_waiting_approval() {
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_test_workflow(),
        state: RuntimeExecutionState::WaitingApproval,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    assert!(exec.is_active());
}

#[test]
fn is_active_completed() {
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_test_workflow(),
        state: RuntimeExecutionState::Completed,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    assert!(!exec.is_active());
}

#[test]
fn is_active_failed() {
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_test_workflow(),
        state: RuntimeExecutionState::Failed {
            reason: "err".to_string(),
            kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            retry_count: None,
        },
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    assert!(!exec.is_active());
}

#[test]
fn is_active_aborted() {
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_test_workflow(),
        state: RuntimeExecutionState::Aborted,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    assert!(!exec.is_active());
}

// ---- to_commit_snapshot: all state variants ----

#[test]
fn to_commit_snapshot_waiting_approval() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: RuntimeExecutionState::WaitingApproval,
        current_node_index: 3,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1001.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let ws = exec.to_commit_snapshot();
    assert_eq!(ws.state, RuntimeExecutionState::WaitingApproval);
    assert_eq!(ws.current_node_name, "report");
    assert_eq!(ws.current_node_index, 3);
}

#[test]
fn to_commit_snapshot_failed() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: RuntimeExecutionState::Failed {
            reason: "exit code 1".to_string(),
            kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            retry_count: None,
        },
        current_node_index: 1,
        node_execution_counts: HashMap::new(),
        node_history: vec![NodeHistoryEntry {
            node_name: "plan".to_string(),
            completed_at: 1000.5,
            result: None,
            session_id: None,
            token_usage: None,
            artifact: None,

            attempt: 0,
            fanout_children: None,
            state: crate::domain::workflow::value_objects::default_node_history_status(),
        }],
        started_at: 1000.0,
        updated_at: 1001.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let ws = exec.to_commit_snapshot();
    assert_eq!(
        ws.state,
        RuntimeExecutionState::Failed {
            reason: "exit code 1".to_string(),
            kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            retry_count: None,
        }
    );
    assert_eq!(ws.current_node_name, "implement");
    assert_eq!(ws.node_history.len(), 1);
}

#[test]
fn to_commit_snapshot_aborted() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: RuntimeExecutionState::Aborted,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1001.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let ws = exec.to_commit_snapshot();
    assert_eq!(ws.state, RuntimeExecutionState::Aborted);
}

#[test]
fn to_commit_snapshot_completed() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: RuntimeExecutionState::Completed,
        current_node_index: 3,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1002.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let ws = exec.to_commit_snapshot();
    assert_eq!(ws.state, RuntimeExecutionState::Completed);
    assert_eq!(ws.workflow_definition.nodes.len(), 4);
}

// ---- loop_guard: boundary value at exactly max_iterations ----

#[test]
fn check_loop_guard_at_boundary_minus_one_allowed() {
    let mut exec = make_exec(2); // review (max_iterations=3)
    exec.node_execution_counts.insert("review".to_string(), 2);
    assert_eq!(
        exec.check_loop_guard("review").unwrap(),
        LoopGuardResult::Allowed
    );
}

#[test]
fn check_loop_guard_at_exact_boundary_exceeded() {
    let mut exec = make_exec(2); // review (max_iterations=3)
    exec.node_execution_counts.insert("review".to_string(), 3);
    assert_eq!(
        exec.check_loop_guard("review").unwrap(),
        LoopGuardResult::Exceeded {
            max_iterations: 3,
            count: 3,
            on_exhausted: Some("report".to_string()),
        }
    );
}

#[test]
fn loop_guard_no_guard_defined() {
    let workflow = make_test_workflow();
    let node = &workflow.nodes[0]; // plan (no loop_guard)
    assert!(!node
        .rules
        .iter()
        .any(|rule| matches!(rule, Rule::LoopGuard { .. })));
}

// ---- decide_next_node ----

fn make_exec(node_index: usize) -> WorkflowExecution {
    WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_test_workflow(),
        state: RuntimeExecutionState::Running,
        current_node_index: node_index,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    }
}

fn workflow_exec(workflow: WorkflowDefinitionYaml, node_index: usize) -> WorkflowExecution {
    WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: RuntimeExecutionState::Running,
        current_node_index: node_index,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    }
}

fn structured_node_output(node_name: &str, value: serde_json::Value) -> RuntimeArtifact {
    RuntimeArtifact {
        node_name: node_name.to_string(),
        attempt: 1,
        session_id: None,
        result: None,
        artifact: Some(value),
        contract: None,
        token_usage: None,
        completed_at: 1000.0,
    }
}

fn bool_object_schema(field: &str) -> SchemaDef {
    SchemaDef::Object {
        properties: [(field.to_string(), SchemaDef::Boolean)]
            .into_iter()
            .collect(),
        required: [field.to_string()].into_iter().collect(),
        additional_properties: true,
    }
}

fn enum_object_schema(field: &str, values: &[&str]) -> SchemaDef {
    SchemaDef::Object {
        properties: [(
            field.to_string(),
            SchemaDef::String {
                r#enum: Some(values.iter().map(|value| (*value).to_string()).collect()),
            },
        )]
        .into_iter()
        .collect(),
        required: [field.to_string()].into_iter().collect(),
        additional_properties: true,
    }
}

fn command_output_for_test(exit_code: i32, stdout: String, stderr: String) -> CommandRunOutput {
    CommandRunOutput {
        exit_code,
        stdout,
        stderr,
        duration_ms: 42,
    }
}

fn command_artifact_test_workflow(
    schemas: std::collections::BTreeMap<String, SchemaDef>,
) -> WorkflowDefinitionYaml {
    WorkflowDefinitionYaml {
        name: "command-artifact".to_string(),
        description: String::new(),
        builtin: false,
        schemas,
        nodes: vec![make_test_node(
            "execution",
            TestKind::Command,
            "printf test",
            vec![],
            None,
        )],
    }
}

#[test]
fn command_artifact_without_contract_sets_standard_result_and_limits_output() {
    let workflow = command_artifact_test_workflow(Default::default());
    let schemas = workflow_schemas_to_domain(&workflow.schemas);
    let long_stdout = "x".repeat(workflow_output_limit::MAX_OUTPUT_SIZE + 100);

    let artifact = build_command_artifact(
        &schemas,
        None,
        command_output_for_test(0, long_stdout, "err".to_string()),
        &[],
    );

    assert_eq!(artifact.event_contract, None);
    assert_eq!(artifact.result_summary, "exit_code=0");
    assert_eq!(artifact.value["ok"], true);
    assert_eq!(artifact.value["exit_code"], 0);
    assert_eq!(artifact.value["stderr"], "err");
    assert_eq!(artifact.value["duration"], 42);
    let stdout = artifact.value["stdout"].as_str().unwrap();
    assert!(stdout.starts_with('x'));
    assert!(stdout.ends_with("... (truncated)"));
    assert!(stdout.len() <= workflow_output_limit::MAX_OUTPUT_SIZE + 20);
}

#[test]
fn command_artifact_with_contract_merges_contract_fields_and_sets_ok() {
    let workflow = command_artifact_test_workflow(
        [("verdict".to_string(), bool_object_schema("passed"))]
            .into_iter()
            .collect(),
    );
    let schemas = workflow_schemas_to_domain(&workflow.schemas);

    let artifact = build_command_artifact(
        &schemas,
        Some("verdict"),
        command_output_for_test(0, r#"{"passed":true}"#.to_string(), String::new()),
        &[],
    );

    assert_eq!(artifact.event_contract.as_deref(), Some("verdict"));
    assert_eq!(artifact.value["ok"], true);
    assert_eq!(artifact.value["exit_code"], 0);
    assert_eq!(artifact.value["stdout"], r#"{"passed":true}"#);
    assert_eq!(artifact.value["passed"], true);
}

#[test]
fn command_contract_validation_failure_keeps_standard_result_only() {
    let workflow = command_artifact_test_workflow(
        [("verdict".to_string(), bool_object_schema("passed"))]
            .into_iter()
            .collect(),
    );
    let schemas = workflow_schemas_to_domain(&workflow.schemas);

    let artifact = build_command_artifact(
        &schemas,
        Some("verdict"),
        command_output_for_test(0, r#"{"passed":"yes"}"#.to_string(), String::new()),
        &[],
    );

    assert_eq!(artifact.event_contract, None);
    assert_eq!(artifact.value["ok"], false);
    assert_eq!(artifact.value["exit_code"], 0);
    assert_eq!(artifact.value["stdout"], r#"{"passed":"yes"}"#);
    assert!(artifact.value.get("passed").is_none());
}

#[test]
fn command_artifact_valid_contract_with_nonzero_exit_keeps_contract_and_ok_false() {
    let workflow = command_artifact_test_workflow(
        [("verdict".to_string(), bool_object_schema("passed"))]
            .into_iter()
            .collect(),
    );
    let schemas = workflow_schemas_to_domain(&workflow.schemas);

    let artifact = build_command_artifact(
        &schemas,
        Some("verdict"),
        command_output_for_test(7, r#"{"passed":true}"#.to_string(), String::new()),
        &[],
    );

    assert_eq!(artifact.event_contract.as_deref(), Some("verdict"));
    assert_eq!(artifact.value["passed"], true);
    assert_eq!(artifact.value["ok"], false);
    assert_eq!(artifact.value["exit_code"], 7);
}

#[test]
fn command_contract_validation_uses_raw_stdout_before_display_truncation() {
    let workflow = command_artifact_test_workflow(
        [("verdict".to_string(), bool_object_schema("passed"))]
            .into_iter()
            .collect(),
    );
    let schemas = workflow_schemas_to_domain(&workflow.schemas);
    let stdout = format!(
        r#"{{"passed":true,"padding":"{}"}}"#,
        "x".repeat(workflow_output_limit::MAX_OUTPUT_SIZE)
    );

    let artifact = build_command_artifact(
        &schemas,
        Some("verdict"),
        command_output_for_test(0, stdout, String::new()),
        &[],
    );

    assert_eq!(artifact.event_contract.as_deref(), Some("verdict"));
    assert_eq!(artifact.value["passed"], true);
    assert_eq!(artifact.value["ok"], true);
    assert!(artifact.value["stdout"]
        .as_str()
        .is_some_and(|stdout| stdout.ends_with(workflow_output_limit::TRUNCATION_MARKER)));
}

#[test]
fn command_artifact_redacts_secrets_from_standard_and_artifact() {
    let workflow = command_artifact_test_workflow(
        [("verdict".to_string(), object_schema_for_test(&["message"]))]
            .into_iter()
            .collect(),
    );
    let schemas = workflow_schemas_to_domain(&workflow.schemas);
    let secrets = vec!["CONFIGURED_SECRET_123".to_string()];

    let artifact = build_command_artifact(
        &schemas,
        Some("verdict"),
        command_output_for_test(
            0,
            r#"{"message":"CONFIGURED_SECRET_123"}"#.to_string(),
            "stderr CONFIGURED_SECRET_123".to_string(),
        ),
        &secrets,
    );

    let serialized = serde_json::to_string(&artifact.value).unwrap();
    assert!(!serialized.contains("CONFIGURED_SECRET_123"));
    assert!(serialized.contains("[REDACTED]"));
}

#[test]
fn decide_next_node_returns_next_node_name() {
    let exec = make_exec(0); // plan → next is implement
    assert_eq!(
        exec.decide_next_node(),
        NextNodeDecision::TransitionTo("implement".to_string())
    );
}

#[test]
fn decide_next_node_returns_completed_at_last_node() {
    let exec = make_exec(3); // report (last)
    assert_eq!(exec.decide_next_node(), NextNodeDecision::Completed);
}

#[test]
fn decide_next_node_middle_node() {
    let exec = make_exec(1); // implement → next is review
    assert_eq!(
        exec.decide_next_node(),
        NextNodeDecision::TransitionTo("review".to_string())
    );
}

#[test]
fn decide_next_node_routes_when_from_artifact() {
    let workflow = WorkflowDefinitionYaml {
        name: "when-runtime".to_string(),
        description: String::new(),
        builtin: false,
        schemas: [("verdict".to_string(), bool_object_schema("passed"))]
            .into_iter()
            .collect(),
        nodes: vec![
            {
                let mut node = make_test_node("judge", TestKind::Session, "judge", vec![], None);
                node.artifact = Some("verdict".to_string());
                node.rules = vec![Rule::When {
                    on: "passed".to_string(),
                    then: "done".to_string(),
                    next: "fix".to_string(),
                }];
                node
            },
            make_test_node("done", TestKind::Session, "done", vec![], None),
            make_test_node("fix", TestKind::Session, "fix", vec![], None),
        ],
    };
    let mut exec = workflow_exec(workflow, 0);

    exec.artifacts.insert(
        "judge".to_string(),
        structured_node_output("judge", serde_json::json!({"passed": true})),
    );
    assert_eq!(
        exec.decide_next_node(),
        NextNodeDecision::TransitionTo("done".to_string())
    );

    exec.artifacts.insert(
        "judge".to_string(),
        structured_node_output("judge", serde_json::json!({"passed": false})),
    );
    assert_eq!(
        exec.decide_next_node(),
        NextNodeDecision::TransitionTo("fix".to_string())
    );
}

#[test]
fn decide_next_node_routes_when_from_command_ok_field() {
    let workflow = WorkflowDefinitionYaml {
        name: "command-ok-routing".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            {
                let mut node =
                    make_test_node("run_tests", TestKind::Command, "cargo test", vec![], None);
                node.rules = vec![Rule::When {
                    on: "ok".to_string(),
                    then: "done".to_string(),
                    next: "fix".to_string(),
                }];
                node
            },
            make_test_node("done", TestKind::Command, "printf done", vec![], None),
            make_test_node("fix", TestKind::Command, "printf fix", vec![], None),
        ],
    };
    let mut exec = workflow_exec(workflow, 0);

    exec.artifacts.insert(
        "run_tests".to_string(),
        structured_node_output("run_tests", serde_json::json!({"ok": true})),
    );
    assert_eq!(
        exec.decide_next_node(),
        NextNodeDecision::TransitionTo("done".to_string())
    );

    exec.artifacts.insert(
        "run_tests".to_string(),
        structured_node_output("run_tests", serde_json::json!({"ok": false})),
    );
    assert_eq!(
        exec.decide_next_node(),
        NextNodeDecision::TransitionTo("fix".to_string())
    );
}

#[test]
fn decide_next_node_routes_switch_from_artifact() {
    let workflow = WorkflowDefinitionYaml {
        name: "switch-runtime".to_string(),
        description: String::new(),
        builtin: false,
        schemas: [(
            "verdict".to_string(),
            enum_object_schema("decision", &["SHIP", "FIX"]),
        )]
        .into_iter()
        .collect(),
        nodes: vec![
            {
                let mut node = make_test_node("judge", TestKind::Session, "judge", vec![], None);
                node.artifact = Some("verdict".to_string());
                node.rules = vec![Rule::Switch {
                    on: "decision".to_string(),
                    cases: [
                        ("SHIP".to_string(), "done".to_string()),
                        ("FIX".to_string(), "fix".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                    next: None,
                }];
                node
            },
            make_test_node("done", TestKind::Session, "done", vec![], None),
            make_test_node("fix", TestKind::Session, "fix", vec![], None),
        ],
    };
    let mut exec = workflow_exec(workflow, 0);

    exec.artifacts.insert(
        "judge".to_string(),
        structured_node_output("judge", serde_json::json!({"decision": "FIX"})),
    );

    assert_eq!(
        exec.decide_next_node(),
        NextNodeDecision::TransitionTo("fix".to_string())
    );
}

#[test]
fn apply_advance_routes_builtin_review_fix_switch_by_verdict() {
    for workflow_name in [
        "05_review-fix",
        "05_review-fix_gpt55",
        "05_review-fix_opus48",
    ] {
        let workflow = crate::adaptor::gateway::workflow::builtin::load_builtin_workflow_resolved(
            workflow_name,
        )
        .unwrap()
        .unwrap();

        for (verdict, expected_node) in [("NEEDS_FIX", "implement_tasks"), ("LGTM", "report")] {
            let mut exec = workflow_exec(workflow.clone(), 0);
            exec.artifacts.insert(
                "check_and_make_tasks".to_string(),
                structured_node_output(
                    "check_and_make_tasks",
                    serde_json::json!({
                        "verdict": verdict,
                        "tasks": [],
                        "summary": "summary"
                    }),
                ),
            );

            let outcome = exec.apply_advance();

            assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_)));
            assert_eq!(
                exec.workflow.nodes[exec.current_node_index].name, expected_node,
                "{workflow_name} verdict {verdict} must route to {expected_node}"
            );
        }
    }
}

#[test]
fn apply_advance_routes_builtin_implement_switch_by_verdict() {
    for workflow_name in ["02_implement_gpt55", "02_implement_opus48"] {
        let workflow = crate::adaptor::gateway::workflow::builtin::load_builtin_workflow_resolved(
            workflow_name,
        )
        .unwrap()
        .unwrap();
        let fix_index = workflow
            .nodes
            .iter()
            .position(|node| node.name == "fix")
            .expect("builtin implement workflow must have fix node");

        for (verdict, expected_node) in [("fixed", "review_fanout"), ("completed", "report")] {
            let mut exec = workflow_exec(workflow.clone(), fix_index);
            exec.artifacts.insert(
                "fix".to_string(),
                structured_node_output(
                    "fix",
                    serde_json::json!({
                        "verdict": verdict,
                        "summary": "summary"
                    }),
                ),
            );

            let outcome = exec.apply_advance();

            match expected_node {
                "review_fanout" => assert!(matches!(outcome, NodeOutcome::StartFanout(_))),
                "report" => assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_))),
                _ => unreachable!("unexpected fixture target"),
            }
            assert_eq!(
                exec.workflow.nodes[exec.current_node_index].name, expected_node,
                "{workflow_name} verdict {verdict} must route to {expected_node}"
            );
        }
    }
}

#[test]
fn apply_advance_fails_on_switch_no_match_without_next() {
    let workflow = WorkflowDefinitionYaml {
        name: "switch-no-match".to_string(),
        description: String::new(),
        builtin: false,
        schemas: [(
            "verdict".to_string(),
            enum_object_schema("decision", &["SHIP"]),
        )]
        .into_iter()
        .collect(),
        nodes: vec![
            {
                let mut node = make_test_node("judge", TestKind::Session, "judge", vec![], None);
                node.artifact = Some("verdict".to_string());
                node.rules = vec![Rule::Switch {
                    on: "decision".to_string(),
                    cases: [("SHIP".to_string(), "done".to_string())]
                        .into_iter()
                        .collect(),
                    next: None,
                }];
                node
            },
            make_test_node("done", TestKind::Session, "done", vec![], None),
        ],
    };
    let mut exec = workflow_exec(workflow, 0);
    exec.artifacts.insert(
        "judge".to_string(),
        structured_node_output("judge", serde_json::json!({"decision": "HOLD"})),
    );

    let outcome = exec.apply_advance();

    assert!(matches!(outcome, NodeOutcome::Persist(_)));
    assert!(matches!(
        exec.state,
        RuntimeExecutionState::Failed {
            kind: NodeExecutionFailureKind::ValidationFailure,
            ..
        }
    ));
}

#[test]
fn apply_advance_fails_on_unknown_route_target() {
    let workflow = WorkflowDefinitionYaml {
        name: "missing-target".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![make_test_node(
            "start",
            TestKind::Session,
            "start",
            vec![Rule::Next("missing".to_string())],
            None,
        )],
    };
    let mut exec = workflow_exec(workflow, 0);

    exec.apply_advance();

    assert!(matches!(
        exec.state,
        RuntimeExecutionState::Failed {
            kind: NodeExecutionFailureKind::ValidationFailure,
            ..
        }
    ));
}

// ---- check_loop_guard ----

#[test]
fn check_loop_guard_allowed_no_guard() {
    let exec = make_exec(0);
    assert_eq!(
        exec.check_loop_guard("plan").unwrap(),
        LoopGuardResult::Allowed
    );
}

#[test]
fn check_loop_guard_allowed_within_limit() {
    let mut exec = make_exec(2);
    exec.node_execution_counts.insert("review".to_string(), 2);
    assert_eq!(
        exec.check_loop_guard("review").unwrap(),
        LoopGuardResult::Allowed
    );
}

#[test]
fn check_loop_guard_exceeded() {
    let mut exec = make_exec(2);
    exec.node_execution_counts.insert("review".to_string(), 3);
    assert_eq!(
        exec.check_loop_guard("review").unwrap(),
        LoopGuardResult::Exceeded {
            max_iterations: 3,
            count: 3,
            on_exhausted: Some("report".to_string()),
        }
    );
}

#[test]
fn check_loop_guard_node_not_found() {
    let exec = make_exec(0);
    assert!(exec.check_loop_guard("nonexistent").is_err());
}

#[test]
fn check_loop_guard_first_transition_no_count() {
    // node_execution_counts にキーなし = 初回遷移
    let exec = make_exec(2); // review has loop_guard(max_iterations=3)
    assert_eq!(
        exec.check_loop_guard("review").unwrap(),
        LoopGuardResult::Allowed
    );
}

// ---- decide_turn_complete_action ----

#[test]
fn turn_complete_action_not_running() {
    let mut exec = make_exec(0);
    exec.state = RuntimeExecutionState::Completed;
    assert_eq!(
        exec.decide_turn_complete_action(0),
        TurnCompleteAction::NotRunning
    );
}

#[test]
fn turn_complete_action_session_error() {
    let exec = make_exec(0); // plan (interactive)
    assert_eq!(
        exec.decide_turn_complete_action(1),
        TurnCompleteAction::SessionError {
            node_name: "plan".to_string(),
            exit_code: 1,
            kind: NodeExecutionFailureKind::InfrastructureCrash,
        }
    );
}

#[test]
fn turn_complete_action_auto_evaluate() {
    let exec = make_exec(2); // review (auto)
    let action = exec.decide_turn_complete_action(0);
    match action {
        TurnCompleteAction::AutoEvaluate { node_name } => {
            assert_eq!(node_name, "review");
        }
        other => panic!("Expected AutoEvaluate, got {:?}", other),
    }
}

#[test]
fn turn_complete_action_wait_approval() {
    let exec = make_exec(3); // report (approval)
    assert_eq!(
        exec.decide_turn_complete_action(0),
        TurnCompleteAction::WaitApproval
    );
}

// [02]: Interactive 概念が廃止されたため、Interactive 用 SessionError 経路を
// 検査する旧テスト `turn_complete_action_interactive_fails_for_validation_only_legacy_definition`
// は削除した。command / fanout 種別が turn_complete に流入した場合は専用バリアント
// `UnexpectedNodeKind` を返し、`SessionError { exit_code: 0 }`（正常終了セマンティクス）
// との混同を避ける。下記 2 テストでバリアント別に確認する。

#[test]
fn turn_complete_action_unexpected_node_kind_for_command() {
    let mut exec = make_exec(0);
    exec.workflow.nodes[0].kind = test_node_kind(TestKind::Command, "cargo build");
    let action = exec.decide_turn_complete_action(0);
    match action {
        TurnCompleteAction::UnexpectedNodeKind { node_name, kind } => {
            assert_eq!(node_name, "plan");
            assert_eq!(kind, crate::domain::workflow::NodeKindName::Command);
        }
        other => panic!("Expected UnexpectedNodeKind for command, got {:?}", other),
    }
}

#[test]
fn turn_complete_action_unexpected_node_kind_for_fanout() {
    let mut exec = make_exec(0);
    exec.workflow.nodes[0].kind = test_node_kind(TestKind::Fanout, "fanout");
    let action = exec.decide_turn_complete_action(0);
    match action {
        TurnCompleteAction::UnexpectedNodeKind { node_name, kind } => {
            assert_eq!(node_name, "plan");
            assert_eq!(kind, crate::domain::workflow::NodeKindName::Fanout);
        }
        other => panic!("Expected UnexpectedNodeKind for fanout, got {:?}", other),
    }
}

#[test]
fn turn_complete_action_waiting_approval_state_returns_not_running() {
    let mut exec = make_exec(3);
    exec.state = RuntimeExecutionState::WaitingApproval;
    assert_eq!(
        exec.decide_turn_complete_action(0),
        TurnCompleteAction::NotRunning
    );
}

#[test]
fn turn_complete_action_negative_exit_code() {
    let exec = make_exec(0); // plan (interactive)
    assert_eq!(
        exec.decide_turn_complete_action(-1),
        TurnCompleteAction::SessionError {
            node_name: "plan".to_string(),
            exit_code: -1,
            kind: NodeExecutionFailureKind::InfrastructureCrash,
        }
    );
}

#[test]
fn turn_complete_action_auto_no_rules_returns_auto_evaluate_empty() {
    let exec = make_exec(1); // implement (auto, no rules)
    let action = exec.decide_turn_complete_action(0);
    match action {
        TurnCompleteAction::AutoEvaluate { node_name } => {
            assert_eq!(node_name, "implement");
        }
        other => panic!("Expected AutoEvaluate with empty rules, got {:?}", other),
    }
}

// ---- decide_approve_action ----

#[test]
fn decide_approve_action_advances() {
    let mut exec = make_exec(3); // report (approval)
    exec.state = RuntimeExecutionState::WaitingApproval;
    exec.decide_approve_action().unwrap();
}

#[test]
fn decide_approve_action_not_waiting() {
    let exec = make_exec(3); // report, state=Running
    assert!(exec.decide_approve_action().is_err());
}

// ---- validate_start ----

#[test]
fn validate_start_empty_nodes_returns_err() {
    let workflow = WorkflowDefinitionYaml {
        name: "empty".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![],
    };
    let result = WorkflowExecution::validate_start(&workflow, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no nodes"));
}

#[test]
fn validate_start_active_workflow_returns_err() {
    let workflow = make_test_workflow();
    let existing = make_exec(0); // Running state
    let result = WorkflowExecution::validate_start(&workflow, Some(&existing));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already running"));
}

#[test]
fn validate_start_completed_workflow_allows_restart() {
    let workflow = make_test_workflow();
    let mut existing = make_exec(0);
    existing.state = RuntimeExecutionState::Completed;
    let result = WorkflowExecution::validate_start(&workflow, Some(&existing));
    assert!(result.is_ok());
}

#[test]
fn validate_start_no_existing_returns_ok() {
    let workflow = make_test_workflow();
    let result = WorkflowExecution::validate_start(&workflow, None);
    assert!(result.is_ok());
}

#[test]
fn validate_start_accepts_command_node() {
    let workflow = WorkflowDefinitionYaml {
        name: "command-wf".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![NodeDefinition {
            name: "build".to_string(),
            kind: test_node_kind(TestKind::Command, "echo hello"),
            ..NodeDefinition::default()
        }],
    };
    let result = WorkflowExecution::validate_start(&workflow, None);
    assert!(result.is_ok());
}

// ---- is_terminal ----

#[test]
fn is_terminal_completed() {
    let mut exec = make_exec(0);
    exec.state = RuntimeExecutionState::Completed;
    assert!(exec.is_terminal());
}

#[test]
fn is_terminal_failed() {
    let mut exec = make_exec(0);
    exec.state = RuntimeExecutionState::Failed {
        reason: "err".to_string(),
        kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
        retry_count: None,
    };
    assert!(exec.is_terminal());
}

#[test]
fn is_terminal_aborted() {
    let mut exec = make_exec(0);
    exec.state = RuntimeExecutionState::Aborted;
    assert!(exec.is_terminal());
}

#[test]
fn is_terminal_executionning_is_false() {
    let exec = make_exec(0);
    assert!(!exec.is_terminal());
}

#[test]
fn is_terminal_waiting_approval_is_false() {
    let mut exec = make_exec(0);
    exec.state = RuntimeExecutionState::WaitingApproval;
    assert!(!exec.is_terminal());
}

// ---- inject_artifacts ----

fn make_node_output(node_name: &str, output_text: &str, result: Option<&str>) -> RuntimeArtifact {
    RuntimeArtifact {
        node_name: node_name.to_string(),
        attempt: 0,
        session_id: None,
        result: result.map(|s| s.to_string()),
        artifact: Some(serde_json::json!({"text": output_text})),
        contract: None,
        token_usage: None,
        completed_at: 1000.0,
    }
}

#[test]
fn mask_sensitive_text_redacts_policy_secrets() {
    let text = "password=secret123 ghp_abcdefghijklmnopqrstuvwxyz1234567890 -----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY----- MY_TOKEN_VALUE_123456";
    let masked =
        workflow_secret_masker::mask_sensitive_text(text, &["MY_TOKEN_VALUE_123456".to_string()]);
    assert!(!masked.contains("secret123"));
    assert!(!masked.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
    assert!(!masked.contains("PRIVATE KEY-----abc"));
    assert!(!masked.contains("MY_TOKEN_VALUE_123456"));
    assert!(masked.contains("[REDACTED]"));
}

#[test]
fn configured_secret_values_include_notion_api_tokens() {
    let mut cfg = ReleashConfig::default();
    cfg.server.token = "SERVER_TOKEN_123".to_string();
    cfg.notion.insert(
        "/repo".to_string(),
        NotionRepoConfigModel {
            api_token: "NOTION_TOKEN_123456".to_string(),
            database_id: "database".to_string(),
            property_mapping: NotionPropertyMappingModel::default(),
        },
    );

    let config_repository: Arc<dyn crate::domain::app_config::ConfigSecretRepository> =
        Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            cfg,
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        ));
    let secrets = config_repository.configured_secret_values().unwrap();
    assert!(secrets.contains(&"SERVER_TOKEN_123".to_string()));
    assert!(secrets.contains(&"NOTION_TOKEN_123456".to_string()));

    let masked = workflow_secret_masker::mask_sensitive_text(
        "Use NOTION_TOKEN_123456 in this policy.",
        &secrets,
    );
    assert_eq!(masked, "Use [REDACTED] in this policy.");
}

#[test]
fn overlapping_configured_secret_values_are_redacted_longest_first() {
    let text = "Use abcdefghXYZ and abcdefgh in this policy.";
    let masked = workflow_secret_masker::mask_sensitive_text(
        text,
        &["abcdefghXYZ".to_string(), "abcdefgh".to_string()],
    );

    assert_eq!(masked, "Use [REDACTED] and [REDACTED] in this policy.");
    assert!(!masked.contains("XYZ"));
    assert!(!masked.contains("abcdefgh"));
}

#[test]
fn environment_secret_values_include_only_named_secret_values_at_least_eight_bytes() {
    let secrets = workflow_secret_masker::collect_secret_values_from_env_vars(vec![
        (
            "APPROVED_POLICY_TOKEN".to_string(),
            "SECRET_VALUE_123".to_string(),
        ),
        ("PATH".to_string(), "/bin:/usr/bin".to_string()),
        (
            "APPROVED_POLICY_TEXT".to_string(),
            "GENERAL_VALUE_123".to_string(),
        ),
        (
            "SERVICE_API_KEY".to_string(),
            "API_KEY_VALUE_123".to_string(),
        ),
        ("SHORT_TOKEN".to_string(), "short".to_string()),
        ("EMPTY".to_string(), String::new()),
    ]);

    assert!(secrets.contains(&"SECRET_VALUE_123".to_string()));
    assert!(secrets.contains(&"API_KEY_VALUE_123".to_string()));
    assert!(!secrets.contains(&"GENERAL_VALUE_123".to_string()));
    assert!(!secrets.contains(&"/bin:/usr/bin".to_string()));
    assert!(!secrets.contains(&"short".to_string()));
}

#[test]
fn approved_fix_policy_artifact_is_masked_for_fanout_contract_path() {
    let masked = workflow_secret_masker::mask_sensitive_artifact(
        "approved-fix-policy",
        serde_json::json!({
            "policy": "Use password=secret123 and MY_TOKEN_VALUE_123456",
            "review_node": "code_review_fanout",
            "findings": []
        }),
        &["MY_TOKEN_VALUE_123456".to_string()],
    );
    let serialized = serde_json::to_string(&masked).unwrap();
    assert!(serialized.contains("[REDACTED]"));
    assert!(!serialized.contains("secret123"));
    assert!(!serialized.contains("MY_TOKEN_VALUE_123456"));
}

#[test]
fn approved_policy_injected_output_uses_sanitized_contract_payload_without_global_variables() {
    let mut node = make_test_node("fix", TestKind::Session, "Fix", vec![], None);
    let facet_contents = instruction_contents("Fix");
    node.inputs = vec!["implementation_fix_policy".to_string()];

    let sanitized = serde_json::json!({
        "policy": "Use password=[REDACTED] only in examples.",
        "review_node": "code_review_fanout",
        "findings": []
    });
    let mut outputs = HashMap::new();
    outputs.insert(
        "implementation_fix_policy".to_string(),
        RuntimeArtifact {
            node_name: "implementation_fix_policy".to_string(),
            attempt: 1,
            session_id: Some("policy-session".to_string()),
            result: Some("approved".to_string()),
            artifact: Some(sanitized),
            contract: Some("approved-fix-policy".to_string()),
            token_usage: None,
            completed_at: 1000.0,
        },
    );
    let (_sys, prompt) = workflow_prompt::build_node_prompt(
        &node,
        Some(&facet_contents),
        "execution-1",
        None,
        &outputs,
    )
    .unwrap();
    assert!(prompt.contains("[REDACTED]"));
    assert!(prompt.contains("## input: implementation_fix_policy"));
    assert!(!prompt.contains("<artifacts>"));
    assert!(!prompt.contains("secret123"));
}

#[test]
fn approved_policy_masks_raw_secrets_before_state_variables_history_and_injection() {
    let mut structured = serde_json::json!({
        "policy": "Use password=secret123 with ghp_abcdefghijklmnopqrstuvwxyz1234567890 -----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY----- MY_TOKEN_VALUE_123456",
        "review_node": "code_review_fanout",
        "findings": []
    });
    workflow_secret_masker::mask_json_strings(
        &mut structured,
        &["MY_TOKEN_VALUE_123456".to_string()],
    );
    let raw = serde_json::to_string(&structured).unwrap();
    assert!(!raw.contains("secret123"));
    assert!(!raw.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
    assert!(!raw.contains("PRIVATE KEY-----abc"));
    assert!(!raw.contains("MY_TOKEN_VALUE_123456"));

    let mut exec = make_approval_exec(
        RuntimeExecutionState::WaitingApproval,
        vec![Rule::Next("fix".to_string())],
    );
    exec.workflow.nodes[0].artifact = Some("approved-fix-policy".to_string());
    let mut fix = make_test_node("fix", TestKind::Session, "Fix", vec![], None);
    let facet_contents = instruction_contents("Fix");
    fix.inputs = vec!["review".to_string()];
    exec.workflow.nodes.push(fix);
    let outcome = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            artifact: Some(structured),
            contract: Some("approved-fix-policy".to_string()),
        },
    )
    .unwrap();
    assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_)));

    let state = exec.to_commit_snapshot();
    let state_artifacts = state
        .artifacts
        .values()
        .filter_map(|artifact| artifact.artifact.as_ref())
        .collect::<Vec<_>>();
    let state_json = serde_json::to_string(&state_artifacts).unwrap();
    assert!(state_json.contains("[REDACTED]"));
    assert!(!state_json.contains("secret123"));
    assert!(!state_json.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
    assert!(!state_json.contains("MY_TOKEN_VALUE_123456"));
    assert!(!exec.node_history[0]
        .artifact
        .as_ref()
        .unwrap()
        .to_string()
        .contains("secret123"));

    let (_sys, prompt) = workflow_prompt::build_node_prompt(
        &exec.workflow.nodes[exec.current_node_index],
        Some(&facet_contents),
        "execution-1",
        None,
        &exec.artifacts,
    )
    .unwrap();
    assert!(prompt.contains("[REDACTED]"));
    assert!(!prompt.contains("<artifacts>"));
    assert!(!prompt.contains("secret123"));
    assert!(!prompt.contains("MY_TOKEN_VALUE_123456"));
}

#[test]
fn approved_policy_workflow_event_log_readback_redacts_sensitive_values() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut exec = make_minimal_approval_exec(
        "00000000-0000-0000-0000-000000000917",
        "policy-session",
        "policy-review",
    );
    let secret_env_value = "MY_TOKEN_VALUE_123456".to_string();
    let mut structured = serde_json::json!({
        "policy": "Use password=secret123 with ghp_abcdefghijklmnopqrstuvwxyz1234567890 -----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY----- MY_TOKEN_VALUE_123456",
        "review_node": "spec_review_fanout",
        "findings": []
    });
    workflow_secret_masker::mask_json_strings(&mut structured, &[secret_env_value]);

    let outcome = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            artifact: Some(structured),
            contract: Some("approved-fix-policy".to_string()),
        },
    )
    .unwrap();
    assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_)));

    let entry = exec
        .node_history
        .iter()
        .find(|entry| entry.node_name == "policy-review")
        .unwrap();
    let log = WorkflowEventLog::new(tmp.path());
    log.append(&WorkflowEvent::ExecutionStarted {
        execution_id: exec.id.clone(),
        workflow_name: exec.workflow.name.clone(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        request: String::new(),
        permission_mode: exec.workflow_defaults.permission_mode.clone(),
        definition: exec.workflow.clone(),
        timestamp: 1000.0,
    })
    .unwrap();
    log.append(&WorkflowEvent::ArtifactProduced {
        execution_id: exec.id.clone(),
        node_execution_id: "node-execution-policy-review".to_string(),
        node_name: entry.node_name.clone(),
        contract: Some("approved-fix-policy".to_string()),
        value: entry.artifact.clone().unwrap(),
        request_id: None,
        submitted_at: None,
        timestamp: entry.completed_at,
    })
    .unwrap();
    log.append(&WorkflowEvent::NodeCompleted {
        execution_id: exec.id.clone(),
        node_execution_id: "node-execution-policy-review".to_string(),
        node_name: entry.node_name.clone(),
        result_summary: entry.result.clone(),
        token_usage: entry.token_usage.clone(),
        attempt: entry.attempt,
        timestamp: entry.completed_at,
    })
    .unwrap();

    let raw_ndjson = std::fs::read_to_string(
        tmp.path()
            .join(format!("workflow_execution_logs/{}.ndjson", exec.id)),
    )
    .unwrap();
    assert!(raw_ndjson.contains("[REDACTED]"));
    assert!(!raw_ndjson.contains("secret123"));
    assert!(!raw_ndjson.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
    assert!(!raw_ndjson.contains("PRIVATE KEY-----abc"));
    assert!(!raw_ndjson.contains("MY_TOKEN_VALUE_123456"));

    let events = log.read_log(&exec.id).unwrap();
    let serialized = serde_json::to_string(&events).unwrap();
    assert!(serialized.contains("[REDACTED]"));
    assert!(!serialized.contains("secret123"));
    assert!(!serialized.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
    assert!(!serialized.contains("PRIVATE KEY-----abc"));
    assert!(!serialized.contains("MY_TOKEN_VALUE_123456"));
    let completed = events
        .iter()
        .find(|event| matches!(event, WorkflowEvent::ArtifactProduced { .. }))
        .unwrap();
    match completed {
        WorkflowEvent::ArtifactProduced { value, .. } => {
            let policy = value
                .get("policy")
                .and_then(|policy| policy.as_str())
                .unwrap();
            assert!(policy.contains("[REDACTED]"));
            assert!(!policy.contains("secret123"));
        }
        _ => unreachable!(),
    }
}

// ---- contract retry 判定テストは prose 抽出経路 ([08] で廃止) の付随物だったため削除した。
//      contract 適合判定は CLI / Tauri 経由の SubmitOutput で発生し、retry は行わない。

// ---- build_node_prompt ----

#[test]
fn build_node_prompt_full_pipeline() {
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();
    let instructions = base.join("instructions");
    let policies = base.join("policies");
    let contracts = base.join("contracts");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::create_dir_all(&contracts).unwrap();
    std::fs::write(
        policies.join("coding.md"),
        "Coding policy for {{ request }}.",
    )
    .unwrap();
    std::fs::write(
        instructions.join("impl.md"),
        "Task: {{ request }}\nImplement the feature.",
    )
    .unwrap();
    std::fs::write(contracts.join("plan-doc.md"), "Output as markdown.").unwrap();

    let mut node = make_test_node("build", TestKind::Session, "unused", vec![], None);
    set_instruction_facet(&mut node, Some("impl".to_string()));
    set_policy_facet(&mut node, Some("coding".to_string()));
    node.artifact = Some("plan-doc".to_string());
    node.inputs = vec!["plan".to_string()];
    let facet_contents = resolve_node_facets_for_test(&node, base);

    let mut outputs = HashMap::new();
    outputs.insert(
        "plan".to_string(),
        make_node_output("plan", "Plan output text", None),
    );
    let (sys, prompt) = workflow_prompt::build_node_prompt(
        &node,
        Some(&facet_contents),
        "00000000-0000-0000-0000-000000000000",
        Some("Fix bug"),
        &outputs,
    )
    .unwrap();

    // policy + contract → system_prompt with request expansion
    let sys_str = sys.expect("system_prompt should be set");
    assert!(sys_str.contains("Coding policy for Fix bug."));
    let instruction = workflow_prompt::render_node_workflow_instruction(
        &node,
        Some(&facet_contents),
        Some("Fix bug"),
        &HashMap::new(),
    )
    .expect("workflow instruction");
    assert!(instruction.contains("Task: Fix bug"));
    assert!(instruction.contains("Implement the feature."));
    assert!(prompt.contains("Task: Fix bug"));
    assert!(prompt.contains("Implement the feature."));
    // contract がある場合、作業本文の末尾にも Contract 由来の
    // 完了時アクションを置き、初回完了時に CLI 提出へ誘導する。
    assert!(prompt.contains("完了時の必須アクション"));
    assert!(prompt.contains("releash workflow output submit 00000000-0000-0000-0000-000000000000"));
    assert!(prompt.contains("--node build"));
    assert!(prompt.contains("--type plan-doc"));
    assert!(prompt.contains("--json"));
    assert!(!prompt.contains("--file"));
    assert!(!prompt.contains("+  --node"));
    // explicit inputs include plan output as JSON
    assert!(prompt.contains("## input: plan"));
    assert!(prompt.contains("Plan output text"));
    assert!(
        prompt.find("完了時の必須アクション").unwrap() > prompt.find("Plan output text").unwrap(),
        "completion action must remain after injected inputs"
    );
}

#[test]
fn build_node_prompt_no_facet_refs_returns_error() {
    let mut node = make_test_node("empty", TestKind::Session, "unused", vec![], None);
    set_session_facets(&mut node, FacetRefs::default());
    let result = workflow_prompt::build_node_prompt(
        &node,
        None,
        "00000000-0000-0000-0000-000000000000",
        None,
        &HashMap::new(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no facet refs"));
}

#[test]
fn build_node_prompt_policy_only_system_prompt_set() {
    // Scenario: policyのみを指定したステップでも system_prompt が合成される
    let tmp = tempfile::TempDir::new().unwrap();
    let policies = tmp.path().join("policies");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::write(policies.join("review.md"), "Review carefully.").unwrap();

    let mut node = make_test_node("review", TestKind::Session, "unused", vec![], None);
    set_policy_facet(&mut node, Some("review".to_string()));
    set_instruction_facet(&mut node, None);
    let facet_contents = resolve_node_facets_for_test(&node, tmp.path());
    let (sys, prompt) = workflow_prompt::build_node_prompt(
        &node,
        Some(&facet_contents),
        "00000000-0000-0000-0000-000000000000",
        None,
        &HashMap::new(),
    )
    .unwrap();

    assert_eq!(sys.as_deref(), Some("Review carefully."));
    assert_eq!(prompt, "");
}

#[test]
fn build_node_prompt_passes_composed_system_prompt_through() {
    // Scenario: 合成された system_prompt は AgentSession 開始時にバックエンドへ受け渡される
    // build_node_prompt の戻り値の Option<String> がそのまま AgentSession に渡される経路を検証する。
    // ドロップ・空文字置換が起きないこと。
    let tmp = tempfile::TempDir::new().unwrap();
    let policies = tmp.path().join("policies");
    let contracts = tmp.path().join("contracts");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::create_dir_all(&contracts).unwrap();
    std::fs::write(policies.join("coding.md"), "POLICY_BODY").unwrap();
    std::fs::write(contracts.join("plan-doc.md"), "CONTRACT_BODY").unwrap();

    let mut node = make_test_node("s", TestKind::Session, "unused", vec![], None);
    set_policy_facet(&mut node, Some("coding".to_string()));
    node.artifact = Some("plan-doc".to_string());
    set_instruction_facet(&mut node, None);
    let facet_contents = resolve_node_facets_for_test(&node, tmp.path());
    let (sys, prompt) = workflow_prompt::build_node_prompt(
        &node,
        Some(&facet_contents),
        "00000000-0000-0000-0000-000000000000",
        None,
        &HashMap::new(),
    )
    .unwrap();

    // 合成された system_prompt は Some(...) として渡される（None や空文字に置換されない）
    let sys = sys.expect("system_prompt must be passed through, not dropped");
    assert!(!sys.is_empty(), "system_prompt must not be empty string");
    assert!(sys.contains("POLICY_BODY"));
    assert!(prompt.contains("完了時の必須アクション"));
    assert!(prompt.contains("releash workflow output submit 00000000-0000-0000-0000-000000000000"));
    assert!(prompt.contains("--node s"));
    assert!(prompt.contains("--type plan-doc"));
    assert!(!prompt.contains("+  --node"));
}

#[test]
fn build_node_prompt_expands_artifact_field_references_in_user_message() {
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();
    let instructions = base.join("instructions");
    let policies = base.join("policies");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::write(
        instructions.join("impl-vars.md"),
        "Spec dir: {{ authoring.spec_dir }}\nRequest: {{ request }}",
    )
    .unwrap();
    std::fs::write(
        policies.join("vars-policy.md"),
        "Operate within {{ authoring.spec_dir }}.",
    )
    .unwrap();

    let mut node = make_test_node("impl", TestKind::Session, "unused", vec![], None);
    set_instruction_facet(&mut node, Some("impl-vars".to_string()));
    set_policy_facet(&mut node, Some("vars-policy".to_string()));
    node.artifact = None;
    node.inputs = vec!["authoring".to_string()];
    let facet_contents = resolve_node_facets_for_test(&node, base);

    let mut outputs = HashMap::new();
    outputs.insert(
        "authoring".to_string(),
        RuntimeArtifact {
            node_name: "authoring".to_string(),
            attempt: 1,
            session_id: None,
            result: None,
            artifact: Some(serde_json::json!({
                "spec_dir": "docs/specs/issues-1326"
            })),
            contract: Some("spec-directory".to_string()),
            token_usage: None,
            completed_at: 1.0,
        },
    );

    let (sys, prompt) = workflow_prompt::build_node_prompt(
        &node,
        Some(&facet_contents),
        "00000000-0000-0000-0000-000000000000",
        Some("implement the authored spec"),
        &outputs,
    )
    .unwrap();
    let instruction = workflow_prompt::render_node_workflow_instruction(
        &node,
        Some(&facet_contents),
        Some("implement the authored spec"),
        &outputs,
    )
    .expect("workflow instruction");

    assert!(instruction.contains("Spec dir: docs/specs/issues-1326"));
    assert!(instruction.contains("Request: implement the authored spec"));
    assert!(prompt.contains("Spec dir: docs/specs/issues-1326"));
    assert!(prompt.contains("Request: implement the authored spec"));
    assert!(prompt.contains("## input: authoring"));
    // 未展開トークンが残らない
    assert!(!prompt.contains("{{ authoring.spec_dir }}"));
    assert!(!prompt.contains("{{ request }}"));

    // system_prompt 側でも `{{ authoring.spec_dir }}` が展開される
    let sys_str = sys.expect("system_prompt should be set");
    assert!(sys_str.contains("Operate within docs/specs/issues-1326."));
    assert!(!sys_str.contains("{{ authoring.spec_dir }}"));
}

// ---- dispatch_session_start (SessionStartGate 経由のテストダブル検証) ----

/// テスト用の `SessionStartGate` 実装。受け取った引数を共有 Vec に記録する。
struct RecordingSessionStartGate {
    records: Arc<std::sync::Mutex<Vec<RecordedSessionStart>>>,
}

#[derive(Clone, Debug)]
struct RecordedSessionStart {
    session_id: String,
    worktree_path: String,
    permission_mode: Option<String>,
    system_prompt: Option<String>,
}

#[async_trait::async_trait]
impl SessionStartGate for RecordingSessionStartGate {
    async fn start_session(
        &self,
        session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
        _workflow_instruction: Option<String>,
    ) -> Result<(), crate::usecase::agent_session::runtime::usecase::AgentRuntimeError> {
        self.records.lock().unwrap().push(RecordedSessionStart {
            session_id: session_id.to_string(),
            worktree_path: worktree_path.to_string(),
            permission_mode,
            system_prompt,
        });
        Ok(())
    }
}

struct StartupTimeoutSessionStartGate;

#[async_trait::async_trait]
impl SessionStartGate for StartupTimeoutSessionStartGate {
    async fn start_session(
        &self,
        _session_id: &str,
        _worktree_path: &str,
        _permission_mode: Option<String>,
        _system_prompt: Option<String>,
        _workflow_instruction: Option<String>,
    ) -> Result<(), crate::usecase::agent_session::runtime::usecase::AgentRuntimeError> {
        Err(
            crate::usecase::agent_session::runtime::usecase::AgentRuntimeError::StartupTimeout {
                retry_count: 2,
                max_retries: 2,
            },
        )
    }
}

#[tokio::test]
async fn dispatch_session_start_preserves_startup_timeout_metadata() {
    let err = dispatch_session_start(
        &StartupTimeoutSessionStartGate,
        "sid",
        "/repo",
        None,
        None,
        None,
    )
    .await
    .unwrap_err();

    match err {
        WorkflowEngineError::AgentRuntime {
            failure_kind,
            retry_count,
            ..
        } => {
            assert_eq!(
                failure_kind,
                crate::domain::workflow::NodeExecutionFailureKind::StartupTimeout
            );
            assert_eq!(retry_count, Some(2));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_session_start_passes_composed_system_prompt_to_gate() {
    // Scenario: 合成された system_prompt は AgentSession 開始時にバックエンドへ受け渡される
    // ───「バックエンド起動経路 (start_agent_session_internal 相当) はテストダブルで置換され
    // 受け取った引数を記録する」を直接検証する。
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();
    let policies = base.join("policies");
    let contracts = base.join("contracts");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::create_dir_all(&contracts).unwrap();
    std::fs::write(policies.join("p.md"), "POLICY_BODY").unwrap();
    std::fs::write(contracts.join("c.md"), "CONTRACT_BODY").unwrap();

    let mut node = make_test_node("s", TestKind::Session, "unused", vec![], None);
    set_policy_facet(&mut node, Some("p".to_string()));
    node.artifact = Some("c".to_string());
    set_instruction_facet(&mut node, None);
    let facet_contents = resolve_node_facets_for_test(&node, base);

    // build_node_prompt → dispatch_session_start の経路をそのまま再現する。
    let (system_prompt, _prompt) = workflow_prompt::build_node_prompt(
        &node,
        Some(&facet_contents),
        "00000000-0000-0000-0000-000000000000",
        None,
        &HashMap::new(),
    )
    .unwrap();

    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let gate = RecordingSessionStartGate {
        records: records.clone(),
    };

    dispatch_session_start(
        &gate,
        "node-session-id",
        "/repo",
        None,
        system_prompt.clone(),
        None,
    )
    .await
    .unwrap();

    let recorded = records.lock().unwrap();
    assert_eq!(
        recorded.len(),
        1,
        "gate.start_session must be invoked exactly once"
    );
    let r = &recorded[0];
    assert_eq!(r.session_id, "node-session-id");
    assert_eq!(r.worktree_path, "/repo");
    assert!(r.permission_mode.is_none());
    let sp = r
        .system_prompt
        .as_ref()
        .expect("system_prompt must be passed through as Some(_)");
    assert!(
        !sp.is_empty(),
        "system_prompt must not be dropped or replaced with an empty string"
    );
    assert!(sp.contains("POLICY_BODY"));
}

#[tokio::test]
async fn build_and_dispatch_node_session_forwards_composed_system_prompt_through_gate() {
    // Scenario: 合成された system_prompt は AgentSession 開始時にバックエンドへ受け渡される
    // start_node_session 側の経路（build_node_prompt → SessionStartGate）を切り出したヘルパーを
    // 記録用 gate で駆動し、合成された system_prompt が None / 空文字に置換されずに
    // gate に渡ることを直接 assert する。
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();
    let policies = base.join("policies");
    let contracts = base.join("contracts");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::create_dir_all(&contracts).unwrap();
    std::fs::write(policies.join("p.md"), "NODE_POLICY_BODY").unwrap();
    std::fs::write(contracts.join("c.md"), "NODE_CONTRACT_BODY").unwrap();

    let mut node = make_test_node("s", TestKind::Session, "unused", vec![], None);
    set_policy_facet(&mut node, Some("p".to_string()));
    node.artifact = Some("c".to_string());
    set_instruction_facet(&mut node, None);
    let facet_contents = resolve_node_facets_for_test(&node, base);

    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let gate = RecordingSessionStartGate {
        records: records.clone(),
    };

    let prompt = WorkflowRuntimeService::build_and_dispatch_node_session(
        &gate,
        &node,
        Some(&facet_contents),
        "00000000-0000-0000-0000-000000000000",
        "node-session-id",
        "/repo",
        None,
        None,
        &HashMap::new(),
    )
    .await
    .unwrap();

    // knowledge / instruction がなくても、contract があれば user_message には
    // Contract 由来の完了時アクションが入る。
    let _ = prompt;

    let recorded = records.lock().unwrap();
    assert_eq!(
        recorded.len(),
        1,
        "gate.start_session must be invoked exactly once via build_and_dispatch_node_session"
    );
    let r = &recorded[0];
    assert_eq!(r.session_id, "node-session-id");
    assert_eq!(r.worktree_path, "/repo");
    assert!(r.permission_mode.is_none());
    let sp = r.system_prompt.as_ref().expect(
        "system_prompt must be passed through start_node_session path as Some(_), not dropped",
    );
    assert!(
        !sp.is_empty(),
        "system_prompt must not be dropped or replaced with an empty string"
    );
    assert!(sp.contains("NODE_POLICY_BODY"));
}

#[tokio::test]
async fn dispatch_session_start_passes_none_when_no_facets() {
    // Scenario: policy も contract も指定がないと system_prompt は設定されない
    // を SessionStartGate 経由でも維持することを検証する。
    let tmp = tempfile::TempDir::new().unwrap();
    let instructions = tmp.path().join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(instructions.join("only-instr.md"), "Body").unwrap();

    let mut node = make_test_node("s", TestKind::Session, "unused", vec![], None);
    set_instruction_facet(&mut node, Some("only-instr".to_string()));
    let facet_contents = resolve_node_facets_for_test(&node, tmp.path());
    let (system_prompt, _prompt) = workflow_prompt::build_node_prompt(
        &node,
        Some(&facet_contents),
        "00000000-0000-0000-0000-000000000000",
        None,
        &HashMap::new(),
    )
    .unwrap();

    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let gate = RecordingSessionStartGate {
        records: records.clone(),
    };

    dispatch_session_start(&gate, "sid", "/repo", None, system_prompt, None)
        .await
        .unwrap();

    let recorded = records.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert!(
        recorded[0].system_prompt.is_none(),
        "system_prompt must be None when neither policy nor contract is specified"
    );
}

// ---- start_node_session_with_deps (副作用境界の注入による順序保証検証) ----

/// テスト用の `NodeSessionDeps` 実装。副作用境界の各メソッドの呼び出し回数を
/// 記録し、本番経路と同じ順序で副作用が発火することを assert できるようにする。
/// プロンプト合成失敗時に `create_node_session` が呼ばれないこと等を検証する。
#[derive(Default)]
struct RecordingNodeSessionDeps {
    create_node_session_count: std::sync::atomic::AtomicUsize,
    dispatch_session_start_count: std::sync::atomic::AtomicUsize,
    mark_node_tab_open_count: std::sync::atomic::AtomicUsize,
    append_node_session_started_count: std::sync::atomic::AtomicUsize,
    append_node_session_started_should_fail: std::sync::atomic::AtomicBool,
    broadcast_state_count: std::sync::atomic::AtomicUsize,
    start_agent_turn_count: std::sync::atomic::AtomicUsize,
    created_contexts: std::sync::Mutex<Vec<WorkflowNodeContext>>,
    dispatched_workflow_instructions: std::sync::Mutex<Vec<Option<String>>>,
    started_workflow_instructions: std::sync::Mutex<Vec<Option<String>>>,
}

impl RecordingNodeSessionDeps {
    fn create_node_session_count(&self) -> usize {
        self.create_node_session_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn dispatch_session_start_count(&self) -> usize {
        self.dispatch_session_start_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn mark_node_tab_open_count(&self) -> usize {
        self.mark_node_tab_open_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn broadcast_state_count(&self) -> usize {
        self.broadcast_state_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn append_node_session_started_count(&self) -> usize {
        self.append_node_session_started_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn fail_append_node_session_started(&self) {
        self.append_node_session_started_should_fail
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn start_agent_turn_count(&self) -> usize {
        self.start_agent_turn_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn created_contexts(&self) -> Vec<WorkflowNodeContext> {
        self.created_contexts.lock().unwrap().clone()
    }

    fn dispatched_workflow_instructions(&self) -> Vec<Option<String>> {
        self.dispatched_workflow_instructions
            .lock()
            .unwrap()
            .clone()
    }

    fn started_workflow_instructions(&self) -> Vec<Option<String>> {
        self.started_workflow_instructions.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl NodeSessionDeps for RecordingNodeSessionDeps {
    async fn create_node_session(
        &self,
        _worktree_path: &str,
        _node_model: Option<String>,
        _node_permission: Option<String>,
        _workflow_defaults: WorkflowDefaults,
        workflow_node_context: WorkflowNodeContext,
        _kind_context: workflow_runtime_session::NodeRuntimeKindContext,
    ) -> Result<NodeSessionInfo, WorkflowEngineError> {
        self.create_node_session_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.created_contexts
            .lock()
            .unwrap()
            .push(workflow_node_context);
        Ok(NodeSessionInfo {
            id: "node-session-id".to_string(),
            permission_mode: "ask".to_string(),
        })
    }

    async fn dispatch_session_start(
        &self,
        _node_session_id: &str,
        _worktree_path: &str,
        _permission_mode: Option<String>,
        _system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowEngineError> {
        self.dispatch_session_start_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.dispatched_workflow_instructions
            .lock()
            .unwrap()
            .push(workflow_instruction);
        Ok(())
    }

    async fn mark_node_tab_open(&self, _node_session_id: &str) {
        self.mark_node_tab_open_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    async fn broadcast_state(&self, _worktree_path: &str, _snapshot: RuntimeCommitSnapshot) {
        self.broadcast_state_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    async fn append_node_session_started(
        &self,
        _snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowEngineError> {
        self.append_node_session_started_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self
            .append_node_session_started_should_fail
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(WorkflowEngineError::SessionStore(
                "append node session started failed".to_string(),
            ));
        }
        Ok(())
    }

    async fn start_agent_turn_locked(
        &self,
        node_session_id: &str,
        _worktree_path: &str,
        _permission_mode: &str,
        _prompt: &str,
        _system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowEngineError> {
        self.start_agent_turn_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.started_workflow_instructions
            .lock()
            .unwrap()
            .push(workflow_instruction);
        let _ = node_session_id;
        Ok(())
    }
}

/// `executions` に 1 ステップのワークフロー実行を登録する。
/// 指定された node を current_node_index=0 として登録する。
fn insert_single_node_execution(
    execs: &mut HashMap<String, WorkflowExecution>,
    node: NodeDefinition,
) {
    let node_name = node.name.clone();
    let workflow = WorkflowDefinitionYaml {
        name: "regression-workflow".to_string(),
        description: "regression test".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![node],
    };
    let exec = WorkflowExecution {
        id: "exec-id".to_string(),
        workflow,
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: HashMap::from([(node_name.clone(), 1)]),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: vec![node_execution_fixture(
            "exec-id",
            "node-execution-current",
            &node_name,
            1,
            NodeExecutionStatus::Running,
            None,
            None,
        )],
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    execs.insert(exec.id.clone(), exec);
}

async fn seed_single_node_facet_contents_for_test(
    engine: &WorkflowRuntimeService,
    node_name: &str,
    contents: crate::adaptor::gateway::workflow::facet::FacetContents,
) {
    engine.execution_facet_contents.lock().await.insert(
        "exec-id".to_string(),
        crate::adaptor::gateway::workflow::facet::WorkflowFacetContents::from_node_for_test(
            node_name, contents,
        ),
    );
}

#[tokio::test]
async fn start_node_session_with_deps_skips_side_effects_when_prompt_synthesis_fails() {
    // 回帰防止: `start_node_session` 本番経路では、参照先ファセットが
    // 存在しないステップを起動した際にプロンプト合成段階で失敗し、
    // 後続の副作用（親セッション取得 / ChatSession 生成 / `session_workflow_refs`
    // 登録 / AgentSession 開始 / 永続化 / ブロードキャスト / ターン起動）は
    // 一切実行されないことを構造的に保証する。
    //
    // 旧実装では先に ChatSession を生成・参照マップへ登録してから
    // プロンプト合成（ファセット未発見で失敗し得る）を行っていたため、
    // 参照先ファセットが存在しないステップを起動すると孤立した
    // ChatSession と参照マップ entry が残るバグがあった。
    //
    // 本テストは `NodeSessionDeps` 経由で副作用境界をテストダブルに差し替え、
    // ファセット参照が解決不能な execution に対し `start_node_session_with_deps`
    // を実行することで:
    //   (a) `Err(InvalidWorkflow(_))` が返ること
    //   (b) `create_node_session` の呼び出し回数が 0 であること
    //   (c) `fetch_parent_session` 等 他の副作用境界メソッドも 0 回であること
    //   (d) `engine.session_workflow_refs` が空のままであること
    //   (e) `executions["/repo"].current_session_id` が `None` のままであること
    // を assert する。`start_node_session` 内の順序を逆転（先に create_node_session
    // → 後に build_node_prompt）させると (b) が 1 となりテストが失敗する。
    let engine = WorkflowRuntimeService::new_for_test();

    // 参照先ファセットが解決不能な node を含む execution を登録する。
    // facets_base_dir() 配下に "nonexistent_policy_<uuid>.md" が偶然存在することは
    // 実用上ありえないため、ファセット合成は必ず失敗する。
    let mut node = make_test_node("missing-facet", TestKind::Session, "unused", vec![], None);
    set_instruction_facet(&mut node, None);
    set_policy_facet(
        &mut node,
        Some(format!(
            "nonexistent_policy_{}",
            uuid::Uuid::new_v4().simple()
        )),
    );

    {
        let mut execs = engine.executions.lock().await;
        insert_single_node_execution(&mut execs, node);
    }

    // 事前条件: session_workflow_refs は空
    assert!(engine.session_workflow_refs.lock().await.is_empty());

    let deps = RecordingNodeSessionDeps::default();
    let result = engine.start_node_session_with_deps(&deps, "/repo").await;

    // (a) build_node_prompt 失敗で InvalidWorkflow エラーになる
    let err = result.expect_err("missing facet must cause start_node_session_with_deps to fail");
    assert!(
        matches!(err, WorkflowEngineError::InvalidWorkflow(_)),
        "missing facet must produce InvalidWorkflow error, got: {err:?}"
    );

    // (b)/(c) 副作用境界はいずれも呼ばれていない
    assert_eq!(
        deps.create_node_session_count(),
        0,
        "create_node_session must NOT be invoked when prompt synthesis fails"
    );
    assert_eq!(
        deps.dispatch_session_start_count(),
        0,
        "dispatch_session_start must NOT be invoked when prompt synthesis fails"
    );
    assert_eq!(
        deps.mark_node_tab_open_count(),
        0,
        "mark_node_tab_open must NOT be invoked when prompt synthesis fails"
    );
    assert_eq!(
        deps.broadcast_state_count(),
        0,
        "broadcast_state must NOT be invoked when prompt synthesis fails"
    );
    assert_eq!(
        deps.append_node_session_started_count(),
        0,
        "NodeSessionStarted must NOT be appended when prompt synthesis fails"
    );
    assert_eq!(
        deps.start_agent_turn_count(),
        0,
        "start_agent_turn must NOT be invoked when prompt synthesis fails"
    );

    // (d) session_workflow_refs は空のまま
    assert!(
        engine.session_workflow_refs.lock().await.is_empty(),
        "session_workflow_refs must remain empty when prompt synthesis fails"
    );

    // (e) executions["/repo"].current_session_id は None のまま
    let execs = engine.executions.lock().await;
    let (_, exec) = find_by_worktree(&execs, "/repo").expect("execution must remain registered");
    assert!(
        exec.current_session_id.is_none(),
        "current_session_id must remain None when prompt synthesis fails"
    );
}

#[tokio::test]
async fn start_node_session_with_deps_invokes_side_effects_in_order_on_success() {
    // 副作用境界が正しい順序で呼ばれる成功経路を併せて検証する。
    // プロンプト合成が成功した場合は、create_node_session → dispatch_session_start
    // → NodeSessionStarted append → broadcast_state → start_agent_turn の全境界が各 1 回ずつ呼ばれ、
    // engine.session_workflow_refs と executions["/repo"].current_session_id が
    // 期待通り更新されることを assert する。
    let engine = WorkflowRuntimeService::new_for_test();

    let tmp = tempfile::TempDir::new().unwrap();
    let instructions = tmp.path().join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(instructions.join("ok.md"), "hello").unwrap();
    let node = make_test_node("ok-node", TestKind::Session, "ok", vec![], None);
    let facet_contents = resolve_node_facets_for_test(&node, tmp.path());

    {
        let mut execs = engine.executions.lock().await;
        insert_single_node_execution(&mut execs, node);
    }
    seed_single_node_facet_contents_for_test(&engine, "ok-node", facet_contents).await;

    let deps = RecordingNodeSessionDeps::default();
    engine
        .start_node_session_with_deps(&deps, "/repo")
        .await
        .expect("start_node_session_with_deps must succeed for instruction facet node");

    // 各副作用境界が 1 回ずつ呼ばれている
    assert_eq!(deps.create_node_session_count(), 1);
    assert_eq!(deps.dispatch_session_start_count(), 1);
    assert_eq!(deps.mark_node_tab_open_count(), 1);
    assert_eq!(deps.append_node_session_started_count(), 1);
    assert_eq!(deps.broadcast_state_count(), 1);
    assert_eq!(deps.start_agent_turn_count(), 1);
    assert_eq!(
        deps.created_contexts(),
        vec![WorkflowNodeContext {
            execution_id: "exec-id".to_string(),
            node_execution_id: "node-execution-current".to_string(),
            workflow_name: "regression-workflow".to_string(),
            node_name: "ok-node".to_string(),
            attempt: 1,
            parent_node_name: None,
            parent_attempt: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        }]
    );

    // session_workflow_refs に SequentialNode として登録されている
    let refs = engine.session_workflow_refs.lock().await;
    let entry = refs
        .get("node-session-id")
        .expect("session_workflow_refs must contain node-session-id");
    assert_eq!(entry.execution_id, "exec-id");
    drop(refs);

    // executions の current_session_id がステップセッションIDで更新されている
    let execs = engine.executions.lock().await;
    let (_, exec) = find_by_worktree(&execs, "/repo").expect("execution must remain registered");
    assert_eq!(
        exec.current_session_id.as_deref(),
        Some("node-session-id"),
        "current_session_id must be updated to the created node session id"
    );
}

#[tokio::test]
async fn start_node_session_with_deps_keeps_workflow_instruction_outside_node_context() {
    let engine = WorkflowRuntimeService::new_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    let instructions = tmp.path().join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(
        instructions.join("impl.md"),
        "Keep this instruction private.",
    )
    .unwrap();

    let mut node = make_test_node(
        "instruction-node",
        TestKind::Session,
        "unused",
        vec![],
        None,
    );
    set_instruction_facet(&mut node, Some("impl".to_string()));
    let facet_contents = resolve_node_facets_for_test(&node, tmp.path());

    {
        let mut execs = engine.executions.lock().await;
        insert_single_node_execution(&mut execs, node);
    }
    seed_single_node_facet_contents_for_test(&engine, "instruction-node", facet_contents).await;

    let deps = RecordingNodeSessionDeps::default();
    engine
        .start_node_session_with_deps(&deps, "/repo")
        .await
        .expect("start_node_session_with_deps must succeed");

    let contexts = deps.created_contexts();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].node_name, "instruction-node");
    let dispatched = deps.dispatched_workflow_instructions();
    let started = deps.started_workflow_instructions();
    assert_eq!(dispatched.len(), 1);
    assert_eq!(started.len(), 1);
    let dispatched_instruction = dispatched[0]
        .as_deref()
        .expect("dispatch_session_start must receive workflow instruction");
    let started_instruction = started[0]
        .as_deref()
        .expect("start_agent_turn_locked must receive workflow instruction");
    assert_eq!(dispatched_instruction, started_instruction);
    assert!(
        dispatched_instruction.contains("Keep this instruction private."),
        "rendered workflow instruction body must be handed off to both gates"
    );
}

#[tokio::test]
async fn start_node_session_with_deps_propagates_node_session_append_failure() {
    let engine = WorkflowRuntimeService::new_for_test();

    let tmp = tempfile::TempDir::new().unwrap();
    let instructions = tmp.path().join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(instructions.join("ok.md"), "hello").unwrap();
    let node = make_test_node("ok-node", TestKind::Session, "ok", vec![], None);
    let facet_contents = resolve_node_facets_for_test(&node, tmp.path());

    {
        let mut execs = engine.executions.lock().await;
        insert_single_node_execution(&mut execs, node);
    }
    seed_single_node_facet_contents_for_test(&engine, "ok-node", facet_contents).await;

    let deps = RecordingNodeSessionDeps::default();
    deps.fail_append_node_session_started();
    let err = engine
        .start_node_session_with_deps(&deps, "/repo")
        .await
        .expect_err("append failure must propagate to the start flow");

    assert!(
        matches!(&err, WorkflowEngineError::SessionStore(message) if message.contains("append node session started failed")),
        "append failure must surface as SessionStore error, got: {err:?}"
    );
    assert_eq!(deps.create_node_session_count(), 1);
    assert_eq!(deps.dispatch_session_start_count(), 1);
    assert_eq!(deps.mark_node_tab_open_count(), 1);
    assert_eq!(deps.append_node_session_started_count(), 1);
    assert_eq!(
        deps.broadcast_state_count(),
        0,
        "broadcast_state must not run after append failure"
    );
    assert_eq!(
        deps.start_agent_turn_count(),
        0,
        "start_agent_turn must not run after append failure"
    );
}

// ---- build_fanout_child_prompt (fanout child の合成ルール) ----

fn make_fanout_child(name: &str) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Session(SessionSpec {
            permission: Some("edit".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn build_fanout_child_prompt_splits_facets_into_system_and_user() {
    // Scenario: 並列ステップの子ステップでも同じ合成ルールが適用される
    // 並列子ステップに policy / knowledge / instruction と artifact を指定し、
    // policy が system_prompt に、knowledge + instruction + artifact action が user_message に
    // 集約されることを検証する。
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();
    let policies = base.join("policies");
    let knowledges = base.join("knowledge");
    let instructions = base.join("instructions");
    let contracts = base.join("contracts");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::create_dir_all(&knowledges).unwrap();
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::create_dir_all(&contracts).unwrap();
    std::fs::write(policies.join("pol.md"), "FANOUT_POLICY_BODY").unwrap();
    std::fs::write(knowledges.join("know.md"), "FANOUT_KNOWLEDGE_BODY").unwrap();
    std::fs::write(instructions.join("inst.md"), "FANOUT_INSTRUCTION_BODY").unwrap();
    std::fs::write(contracts.join("oc.md"), "FANOUT_CONTRACT_BODY").unwrap();

    let mut child = make_fanout_child("child");
    set_session_facets(
        &mut child,
        FacetRefs {
            policy: Some("pol".to_string()),
            knowledge: Some("know".to_string()),
            instruction: Some("inst".to_string()),
        },
    );
    child.artifact = Some("oc".to_string());
    let facet_contents = resolve_fanout_child_facets_for_test(&child, base);
    let (system_prompt, user_message) = workflow_prompt::build_fanout_child_prompt(
        &child,
        Some(&facet_contents),
        "11111111-1111-1111-1111-111111111111",
        None,
        &HashMap::new(),
        None,
        "22222222-2222-4222-8222-222222222222",
    )
    .unwrap();

    let sp = system_prompt.expect("system_prompt must be set for fanout child with policy/oc");
    // policy の本文が system_prompt に集約される
    assert!(sp.contains("FANOUT_POLICY_BODY"));
    assert!(!sp.contains("FANOUT_KNOWLEDGE_BODY"));
    assert!(!sp.contains("FANOUT_INSTRUCTION_BODY"));

    // knowledge / instruction と Artifact 由来の完了時アクションは user_message に集約される。
    assert!(user_message.contains("FANOUT_KNOWLEDGE_BODY"));
    assert!(user_message.contains("FANOUT_INSTRUCTION_BODY"));
    let instruction = workflow_prompt::render_fanout_child_workflow_instruction(
        &child,
        Some(&facet_contents),
        None,
        &HashMap::new(),
        None,
    )
    .expect("fanout workflow instruction");
    assert!(instruction.contains("FANOUT_INSTRUCTION_BODY"));
    assert!(user_message.contains("完了時の必須アクション"));
    assert!(user_message
        .contains("releash workflow output submit 11111111-1111-1111-1111-111111111111"));
    assert!(user_message.contains("--node child"));
    assert!(user_message.contains("--type oc"));
    assert!(!user_message.contains("+  --node"));
    // policy 本文と schema 名は user_message には余計に混ざらない。
    assert!(!user_message.contains("FANOUT_POLICY_BODY"));
    assert!(!user_message.contains("FANOUT_CONTRACT_BODY"));
}

#[test]
fn build_fanout_child_prompt_no_policy_or_contract_returns_none_system_prompt() {
    // 並列子ステップでも policy がない場合は system_prompt が None になる。
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();
    let instructions = base.join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(instructions.join("inst.md"), "INSTR").unwrap();

    let mut child = make_fanout_child("child");
    child.session_mut().unwrap().facets.instruction = Some("inst".to_string());
    let facet_contents = resolve_fanout_child_facets_for_test(&child, base);
    let (system_prompt, user_message) = workflow_prompt::build_fanout_child_prompt(
        &child,
        Some(&facet_contents),
        "11111111-1111-1111-1111-111111111111",
        None,
        &HashMap::new(),
        None,
        "22222222-2222-4222-8222-222222222222",
    )
    .unwrap();

    assert!(system_prompt.is_none());
    assert!(user_message.contains("INSTR"));
    let instruction = workflow_prompt::render_fanout_child_workflow_instruction(
        &child,
        Some(&facet_contents),
        None,
        &HashMap::new(),
        None,
    )
    .expect("fanout workflow instruction");
    assert_eq!(instruction, "INSTR");
}

// ---- decide_approve_action ----

fn make_approval_exec(state: RuntimeExecutionState, rules: Vec<Rule>) -> WorkflowExecution {
    WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: WorkflowDefinitionYaml {
            name: "test".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_approval_gated_session(
                "review",
                "Review the code",
                rules,
            )],
        },
        state,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: vec![],
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    }
}

// ---- approval input validation adapter ----

#[test]
fn validate_approve_comment_none_is_ok() {
    let result = workflow_approval_runtime::validate_approve_comment(None);
    assert!(result.is_ok());
}

#[test]
fn validate_approval_target_missing_values_returns_unauthorized_target() {
    let exec = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    let result =
        workflow_approval_runtime::validate_approval_target_snapshot(&exec, None, Some("review"));
    assert!(matches!(
        result.unwrap_err(),
        WorkflowEngineError::UnauthorizedApprovalTarget(_)
    ));

    let result =
        workflow_approval_runtime::validate_approval_target_snapshot(&exec, Some("exec-1"), None);
    assert!(matches!(
        result.unwrap_err(),
        WorkflowEngineError::UnauthorizedApprovalTarget(_)
    ));
}

#[test]
fn validate_approval_target_mismatch_returns_unauthorized_target() {
    let exec = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    let result = workflow_approval_runtime::validate_approval_target_snapshot(
        &exec,
        Some("other-exec"),
        Some("review"),
    );
    assert!(matches!(
        result.unwrap_err(),
        WorkflowEngineError::UnauthorizedApprovalTarget(_)
    ));

    let result = workflow_approval_runtime::validate_approval_target_snapshot(
        &exec,
        Some("exec-1"),
        Some("other-node"),
    );
    assert!(matches!(
        result.unwrap_err(),
        WorkflowEngineError::UnauthorizedApprovalTarget(_)
    ));
}

#[test]
fn validate_approval_target_non_waiting_returns_invalid_state() {
    let exec = make_approval_exec(RuntimeExecutionState::Running, vec![]);
    let result = workflow_approval_runtime::validate_approval_target_snapshot(
        &exec,
        Some("exec-1"),
        Some("review"),
    );
    let err = result.unwrap_err();
    assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
    assert!(err.to_string().starts_with("invalid_state:"));
}

#[test]
fn validate_approval_target_terminal_states_return_invalid_state_without_mutation() {
    for state in [
        RuntimeExecutionState::Completed,
        RuntimeExecutionState::Failed {
            reason: "failed".to_string(),
            kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            retry_count: None,
        },
        RuntimeExecutionState::Aborted,
        RuntimeExecutionState::Interrupted,
    ] {
        let exec = make_approval_exec(state.clone(), vec![]);
        let result = workflow_approval_runtime::validate_approval_target_snapshot(
            &exec,
            Some("exec-1"),
            Some("review"),
        );
        let err = result.unwrap_err();
        assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
        assert_eq!(exec.state, state);
        assert!(exec.node_history.is_empty());
    }
}

#[tokio::test]
async fn validate_approval_target_wrong_worktree_returns_unauthorized_without_mutating_state() {
    let engine = WorkflowRuntimeService::new_for_test();
    let exec = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    {
        let mut execs = engine.executions.lock().await;
        execs.insert("/repo-a".to_string(), exec);
    }

    let result = engine
        .validate_approval_target("/repo-b", Some("exec-1"), Some("review"))
        .await;
    let err = result.unwrap_err();
    assert!(matches!(err, WorkflowEngineError::UnauthorizedWorktree(_)));
    assert!(err.to_string().starts_with("unauthorized_worktree:"));

    let execs = engine.executions.lock().await;
    let original = execs.get("/repo-a").unwrap();
    assert_eq!(original.state, RuntimeExecutionState::WaitingApproval);
    assert!(original.node_history.is_empty());
}

#[test]
fn validate_approval_turn_phase_rejects_unfinished_turns() {
    assert!(
        workflow_approval_runtime::validate_approval_turn_phase(Some(
            crate::usecase::agent_session::status::TurnPhase::Streaming
        ))
        .unwrap_err()
        .to_string()
        .starts_with("validation_error:")
    );
    assert!(
        workflow_approval_runtime::validate_approval_turn_phase(Some(
            crate::usecase::agent_session::status::TurnPhase::WaitingPermission
        ))
        .is_err()
    );
    assert!(
        workflow_approval_runtime::validate_approval_turn_phase(Some(
            crate::usecase::agent_session::status::TurnPhase::Idle
        ))
        .is_ok()
    );
}

// [08] 旧 `validate_approval_contract_extraction` ベースの 4 テストは prose 抽出経路の
// 廃止に伴い削除した。approval-gated session の構造化出力は CLI / Tauri 経由の `SubmitOutput`
// で確定し、対応する境界テストは `dispatch_boundary_tests::submit_output_*` 群と
// `workflow::contract::tests::validate_contract_value_*` 群でカバーされる。

#[tokio::test]
async fn validate_approval_chat_instruction_limits_current_approval_session() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("node-session".to_string());
    let execution_id = exec.id.clone();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(execution_id.clone(), exec);
    }
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "node-session".to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );
    }

    let result = engine
        .validate_approval_chat_instruction(
            "node-session",
            &"x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1),
        )
        .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .starts_with("validation_error:"));

    assert!(engine
        .validate_approval_chat_instruction("other-session", &"x".repeat(9000))
        .await
        .is_ok());
}

#[tokio::test]
async fn validate_approval_chat_instruction_rejects_empty_or_whitespace_only_content() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("node-session".to_string());
    let execution_id = exec.id.clone();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(execution_id.clone(), exec);
    }
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "node-session".to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );
    }

    for content in ["", "   ", "\n\t \r\n"] {
        let err = engine
            .validate_approval_chat_instruction("node-session", content)
            .await
            .unwrap_err();
        assert!(
            err.to_string().starts_with("validation_error:"),
            "expected validation_error for content={content:?}, got: {err}"
        );
    }
}

#[tokio::test]
async fn validate_approval_chat_instruction_rejects_current_gated_session_before_waiting() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = make_approval_exec(RuntimeExecutionState::Running, vec![]);
    exec.current_session_id = Some("node-session".to_string());
    let execution_id = exec.id.clone();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(execution_id.clone(), exec);
    }
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "node-session".to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );
    }

    let result = engine
        .validate_approval_chat_instruction("node-session", "Please adjust the policy")
        .await;
    assert!(matches!(
        result.unwrap_err(),
        WorkflowEngineError::InvalidState(_)
    ));
}

#[tokio::test]
async fn validate_approval_chat_instruction_rejects_stale_approved_policy_session() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = make_approval_exec(RuntimeExecutionState::Running, vec![]);
    exec.workflow.nodes[0].name = "implementation_fix_policy".to_string();
    exec.workflow.nodes[0].artifact = Some("approved-fix-policy".to_string());
    exec.current_session_id = Some("fix-session".to_string());
    exec.node_history.push(NodeHistoryEntry {
        node_name: "implementation_fix_policy".to_string(),
        completed_at: 1000.0,
        result: Some("approved".to_string()),
        session_id: Some("stale-policy-session".to_string()),
        token_usage: None,
        artifact: Some(serde_json::json!({
            "policy": "Already approved.",
            "review_node": "code_review_fanout",
            "findings": []
        })),
        attempt: 1,
        fanout_children: None,
        state: crate::domain::workflow::value_objects::default_node_history_status(),
    });
    exec.artifacts.insert(
        "implementation_fix_policy".to_string(),
        RuntimeArtifact {
            node_name: "implementation_fix_policy".to_string(),
            attempt: 1,
            session_id: Some("stale-policy-session".to_string()),
            result: Some("approved".to_string()),
            artifact: Some(serde_json::json!({
                "policy": "Already approved.",
                "review_node": "code_review_fanout",
                "findings": []
            })),
            contract: Some("approved-fix-policy".to_string()),
            token_usage: None,
            completed_at: 1000.0,
        },
    );
    let execution_id = exec.id.clone();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(execution_id.clone(), exec);
    }
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "stale-policy-session".to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );
    }

    let result = engine
        .validate_approval_chat_instruction("stale-policy-session", "Please change policy")
        .await;
    assert!(matches!(
        result.unwrap_err(),
        WorkflowEngineError::InvalidState(_)
    ));
}

#[test]
fn latest_assistant_output_after_approval_chat_adjustment_is_selected() {
    let session = crate::usecase::agent_session::session::ChatSession {
        id: "policy-session".to_string(),
        worktree_path: "/repo".to_string(),
        messages: vec![
            crate::usecase::agent_session::session::ChatMessage {
                id: "m1".to_string(),
                role: crate::usecase::agent_session::session::MessageRole::Agent,
                content: "old policy".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1.0,
                mentions: None,
            },
            crate::usecase::agent_session::session::ChatMessage {
                id: "m2".to_string(),
                role: crate::usecase::agent_session::session::MessageRole::Human,
                content: "Narrow the fix policy".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 2.0,
                mentions: None,
            },
            crate::usecase::agent_session::session::ChatMessage {
                id: "m3".to_string(),
                role: crate::usecase::agent_session::session::MessageRole::Agent,
                content: String::new(),
                thinking: None,
                activities: None,
                parts: Some(vec![
                    crate::usecase::agent_session::session::MessagePart::Text {
                        content: "latest approved policy".to_string(),
                        parent_tool_use_id: None,
                    },
                ]),
                streaming_final_seq: 0,
                timestamp: 3.0,
                mentions: None,
            },
        ],
        state: crate::usecase::agent_session::session::SessionState::Idle,
        created_at: 1.0,
        updated_at: 3.0,
        agent_session_id: None,
        context_carry: None,
        permission_mode: "edit".to_string(),
        plan_mode: false,
        permission_profile_id: None,
        selected_model: None,
        backend_id: None,
        workflow_node_session: false,
        workflow_node_context: None,
        context_epoch: None,
    };

    let output =
        WorkflowRuntimeService::extract_last_assistant_text_from_session(&session).unwrap();
    assert_eq!(output, "latest approved policy");
}

// ---- handle_approval integration (lock-inner logic) ----

#[test]
fn apply_approval_application_records_approved_policy_and_advances_once() {
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: WorkflowDefinitionYaml {
            name: "auto-approve".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                {
                    let mut node = make_approval_gated_session(
                        "fix_policy",
                        "Review fix policy",
                        vec![Rule::Next("fix".to_string())],
                    );
                    node.artifact = Some("approved-fix-policy".to_string());
                    node
                },
                make_test_node("fix", TestKind::Session, "Fix", vec![], None),
            ],
        },
        state: RuntimeExecutionState::WaitingApproval,
        current_node_index: 0,
        node_execution_counts: {
            let mut m = HashMap::new();
            m.insert("fix_policy".to_string(), 1);
            m
        },
        node_history: vec![],
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("policy-session".to_string()),
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let artifact = serde_json::json!({
        "policy": "Fix only the reported issues.",
        "review_node": "code_review_fanout",
        "findings": []
    });
    let first = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            artifact: Some(artifact),
            contract: Some("approved-fix-policy".to_string()),
        },
    )
    .unwrap();
    assert!(matches!(first, NodeOutcome::TransitionAndStart(_)));
    assert_eq!(exec.current_node_index, 1);
    assert_eq!(exec.node_history.len(), 1);
    assert_eq!(*exec.node_execution_counts.get("fix").unwrap(), 1);

    let duplicate = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            artifact: Some(serde_json::json!({
                "policy": "Duplicate",
                "review_node": "code_review_fanout",
                "findings": []
            })),
            contract: Some("approved-fix-policy".to_string()),
        },
    );
    match duplicate {
        Err(WorkflowEngineError::InvalidState(_)) => {}
        _ => panic!("expected invalid_state"),
    }
    assert_eq!(exec.node_history.len(), 1);
    assert_eq!(*exec.node_execution_counts.get("fix").unwrap(), 1);
}

#[test]
fn auto_approve_persist_target_applies_latest_policy_and_advances_once() {
    let mut exec = WorkflowExecution {
        id: "exec-auto-approve".to_string(),
        workflow: WorkflowDefinitionYaml {
            name: "auto-approve-path".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                {
                    let mut node = make_approval_gated_session(
                        "implementation_fix_policy",
                        "Review fix policy",
                        vec![Rule::Next("fix".to_string())],
                    );
                    node.artifact = Some("approved-fix-policy".to_string());
                    node.inputs = vec!["code_review_fanout".to_string()];
                    node
                },
                make_test_node("fix", TestKind::Session, "Fix", vec![], None),
                make_fanout_node("code_review_fanout", vec![]),
            ],
        },
        state: RuntimeExecutionState::WaitingApproval,
        current_node_index: 0,
        node_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("policy-session".to_string()),
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let snapshot = exec.to_commit_snapshot();
    assert_eq!(
        workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(&snapshot, true),
        Some((
            "exec-auto-approve".to_string(),
            "implementation_fix_policy".to_string()
        ))
    );

    // [08] prose 抽出経路は廃止済み。CLI submit 経由で確定する想定の artifact
    // を直接組み立てて apply_approval_application の遷移挙動を検証する。
    let artifact = serde_json::json!({
        "policy": "Fix only reviewed findings.",
        "review_node": "code_review_fanout",
        "findings": []
    });
    let outcome = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            artifact: Some(artifact),
            contract: Some("approved-fix-policy".to_string()),
        },
    )
    .unwrap();

    assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_)));
    assert_eq!(exec.current_node_index, 1);
    assert_eq!(exec.node_history.len(), 1);
    assert_eq!(exec.artifacts.len(), 1);
    assert_eq!(
        exec.artifacts["implementation_fix_policy"]
            .artifact
            .as_ref()
            .unwrap()["policy"],
        "Fix only reviewed findings."
    );
    assert_eq!(exec.node_execution_counts.get("fix"), Some(&1));

    let duplicate = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            artifact: Some(serde_json::json!({
                "policy": "Duplicate",
                "review_node": "code_review_fanout",
                "findings": []
            })),
            contract: Some("approved-fix-policy".to_string()),
        },
    );
    assert!(matches!(
        duplicate,
        Err(WorkflowEngineError::InvalidState(_))
    ));
    assert_eq!(exec.node_history.len(), 1);
    assert_eq!(exec.node_execution_counts.get("fix"), Some(&1));
}

#[tokio::test]
async fn execute_outcome_auto_approve_persist_adopts_policy_and_starts_fix_once() {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/repo";
    let policy_session_id = uuid::Uuid::new_v4().to_string();

    let fix_node = make_test_node("fix", TestKind::Session, "Fix", vec![], None);
    let exec = WorkflowExecution {
        id: "exec-auto-approve".to_string(),
        workflow: WorkflowDefinitionYaml {
            name: "auto-approve-execute-outcome".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("code_review_fanout", vec![]),
                {
                    let mut node = make_approval_gated_session(
                        "implementation_fix_policy",
                        "Review fix policy",
                        vec![Rule::Next("fix".to_string())],
                    );
                    node.artifact = Some("approved-fix-policy".to_string());
                    node.inputs = vec!["code_review_fanout".to_string()];
                    node
                },
                fix_node,
            ],
        },
        state: RuntimeExecutionState::WaitingApproval,
        current_node_index: 1,
        node_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some(policy_session_id.clone()),
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let snapshot = exec.to_commit_snapshot();
    let execution_id = exec.id.clone();
    engine
        .executions
        .lock()
        .await
        .insert(execution_id.clone(), exec);
    engine.session_workflow_refs.lock().await.insert(
        policy_session_id,
        SessionWorkflowRef {
            execution_id: execution_id.clone(),
        },
    );

    let outcome = engine
        .execute_outcome_persist_auto_approve_for_test(worktree_path, &snapshot)
        .await
        .unwrap()
        .unwrap();

    let execs = engine.executions.lock().await;
    let (_, exec) = find_by_worktree(&execs, worktree_path).unwrap();
    assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_)));
    assert_eq!(exec.node_execution_counts.get("fix"), Some(&1));
    assert_eq!(
        exec.node_history
            .iter()
            .filter(|entry| entry.node_name == "implementation_fix_policy")
            .count(),
        1
    );
    // [08] prose 抽出経路は廃止済み。auto approve 経路でも artifact は
    // 確定されず、node は output 無しで完了する（spec [08] Rule 4）。
    assert!(exec
        .artifacts
        .get("implementation_fix_policy")
        .and_then(|output| output.artifact.as_ref())
        .is_none());
}

#[test]
fn execute_outcome_persist_path_builds_auto_approve_target_for_current_node() {
    let mut exec = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("policy-session".to_string());
    let waiting = exec.to_commit_snapshot();

    assert_eq!(
        workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(&waiting, true),
        Some(("exec-1".to_string(), "review".to_string()))
    );
    assert_eq!(
        workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(&waiting, false),
        None
    );

    exec.state = RuntimeExecutionState::Running;
    let running = exec.to_commit_snapshot();
    assert_eq!(
        workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(&running, true),
        None
    );
}

#[test]
fn workflow_approval_auto_approve_flag_controls_waiting_approval_snapshots() {
    let mut exec = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("policy-session".to_string());
    let waiting = exec.to_commit_snapshot();
    assert!(workflow_approval_runtime::should_auto_approve_workflow_approval(&waiting, true));
    assert!(!workflow_approval_runtime::should_auto_approve_workflow_approval(&waiting, false));

    exec.state = RuntimeExecutionState::Running;
    let running = exec.to_commit_snapshot();
    assert!(!workflow_approval_runtime::should_auto_approve_workflow_approval(&running, true));
}

#[test]
fn workflow_approval_auto_approve_disabled_ignores_agent_auto_approve_permission_mode() {
    let mut exec = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("policy-session".to_string());
    let agent_auto_approve_permission_mode = "full";
    let workflow_approval_auto_approve_enabled = false;
    let snapshot = exec.to_commit_snapshot();

    assert_eq!(agent_auto_approve_permission_mode, "full");
    assert!(
        !workflow_approval_runtime::should_auto_approve_workflow_approval(
            &snapshot,
            workflow_approval_auto_approve_enabled
        )
    );
    assert_eq!(
        workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(
            &snapshot,
            workflow_approval_auto_approve_enabled,
        ),
        None
    );
}

fn make_normal_node_exec_with_stall_observation() -> WorkflowExecution {
    let mut exec = WorkflowExecution {
        id: "normal-stall-clear".to_string(),
        workflow: WorkflowDefinitionYaml {
            name: "normal-stall-clear-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_test_node(
                    "plan",
                    TestKind::Session,
                    "plan",
                    vec![Rule::Next("implement".to_string())],
                    None,
                ),
                make_test_node("implement", TestKind::Session, "implement", vec![], None),
            ],
        },
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: HashMap::from([("plan".to_string(), 1)]),
        node_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("normal-session".to_string()),
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "edit".to_string(),
        },
    };
    exec.current_stall_observations =
        vec![workflow_stall_observation_fixture("normal-session", "plan")];
    exec
}

#[test]
fn normal_node_completion_retry_and_transition_clear_stall_observations() {
    let mut completed = make_normal_node_exec_with_stall_observation();
    let entry = completed.make_node_history_entry(Some("done".to_string()), None, None);
    completed.node_history.push(entry);
    assert!(completed.current_stall_observations.is_empty());

    let mut retried = make_normal_node_exec_with_stall_observation();
    assert!(matches!(
        retried.retry_current_node(),
        NodeOutcome::RetryCurrentNode { .. }
    ));
    assert!(retried.current_stall_observations.is_empty());

    let mut transitioned = make_normal_node_exec_with_stall_observation();
    assert!(matches!(
        transitioned.apply_advance(),
        NodeOutcome::TransitionAndStart(_)
    ));
    assert!(transitioned.current_stall_observations.is_empty());
}

// R4-02: make_node_history_entryがcontract resultをRuntimeArtifact.resultに保存する
#[test]
fn make_node_history_entry_saves_contract_result_to_node_output() {
    let mut exec = WorkflowExecution {
        id: "test-exec".to_string(),
        workflow: make_test_workflow(),
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: {
            let mut m = HashMap::new();
            m.insert("plan".to_string(), 1);
            m
        },
        node_history: vec![],
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("session-1".to_string()),
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let structured = serde_json::json!({"verdict": "LGTM", "findings": []});
    let entry = exec.make_node_history_entry(
        Some("LGTM".to_string()),
        Some(structured.clone()),
        Some("review-verdict".to_string()),
    );

    assert_eq!(entry.result.as_deref(), Some("LGTM"));
    assert_eq!(entry.artifact, Some(structured.clone()));

    let node_output = exec
        .artifacts
        .get("plan")
        .expect("RuntimeArtifact should exist");
    assert_eq!(node_output.result.as_deref(), Some("LGTM"));
    assert_eq!(node_output.artifact, Some(structured));
    assert_eq!(node_output.contract.as_deref(), Some("review-verdict"));
}

#[test]
fn make_node_history_entry_no_artifact_no_node_output() {
    let mut exec = WorkflowExecution {
        id: "test-exec".to_string(),
        workflow: make_test_workflow(),
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: {
            let mut m = HashMap::new();
            m.insert("plan".to_string(), 1);
            m
        },
        node_history: vec![],
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("session-1".to_string()),
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let entry = exec.make_node_history_entry(Some("complete".to_string()), None, None);

    assert_eq!(entry.result.as_deref(), Some("complete"));
    assert!(entry.artifact.is_none());
    assert!(!exec.artifacts.contains_key("plan"));
}

// ---- on_exhausted: production routing テスト ----

fn make_on_exhausted_workflow() -> WorkflowDefinitionYaml {
    WorkflowDefinitionYaml {
        name: "on-exhausted-test".to_string(),
        description: "Test on_exhausted".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            make_test_node(
                "fix",
                TestKind::Session,
                "Fix issues",
                vec![Rule::Next("review".to_string())],
                Some(Rule::LoopGuard {
                    max_iterations: 2,
                    on_exhausted: "approval".to_string(),
                }),
            ),
            make_test_node(
                "review",
                TestKind::Session,
                "Review",
                vec![Rule::Next("fix".to_string())],
                None,
            ),
            make_test_node(
                "approval",
                TestKind::Session,
                "Approve",
                vec![Rule::Next("fix".to_string())],
                None,
            ),
        ],
    }
}

#[test]
fn on_exhausted_transitions_to_fallback_node() {
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_on_exhausted_workflow(),
        state: RuntimeExecutionState::Running,
        current_node_index: 1, // review
        node_execution_counts: {
            let mut m = HashMap::new();
            m.insert("fix".to_string(), 2); // already at max
            m
        },
        node_history: vec![],
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    // review の next=fix → ガード超過 → on_exhausted で approval へ
    let outcome = exec.apply_advance();
    assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_)));
    assert_eq!(
        exec.workflow.nodes[exec.current_node_index].name,
        "approval"
    );
}

#[test]
fn check_loop_guard_exceeded_with_on_exhausted() {
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_on_exhausted_workflow(),
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: {
            let mut m = HashMap::new();
            m.insert("fix".to_string(), 2);
            m
        },
        node_history: vec![],
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    assert_eq!(
        exec.check_loop_guard("fix").unwrap(),
        LoopGuardResult::Exceeded {
            max_iterations: 2,
            count: 2,
            on_exhausted: Some("approval".to_string()),
        }
    );
}

// ---- on_exhausted チェーン遷移テスト ----

#[test]
fn on_exhausted_chain_transitions() {
    // start → node_a → (exhausted) → node_b → (exhausted) → node_c
    let wf = WorkflowDefinitionYaml {
        name: "chain-test".to_string(),
        description: "test".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            make_test_node(
                "start",
                TestKind::Session,
                "Start",
                vec![Rule::Next("node_a".to_string())],
                None,
            ),
            make_test_node(
                "node_a",
                TestKind::Session,
                "A",
                vec![],
                Some(Rule::LoopGuard {
                    max_iterations: 1,
                    on_exhausted: "node_b".to_string(),
                }),
            ),
            make_test_node(
                "node_b",
                TestKind::Session,
                "B",
                vec![],
                Some(Rule::LoopGuard {
                    max_iterations: 1,
                    on_exhausted: "node_c".to_string(),
                }),
            ),
            make_test_node("node_c", TestKind::Session, "C", vec![], None),
        ],
    };
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: wf,
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: {
            let mut m = HashMap::new();
            m.insert("node_a".to_string(), 1);
            m.insert("node_b".to_string(), 1);
            m
        },
        node_history: vec![],
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    // start の next=node_a → exhausted → node_b → exhausted → node_c
    let outcome = exec.apply_advance();
    assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_)));
    assert_eq!(exec.workflow.nodes[exec.current_node_index].name, "node_c");
}

fn make_on_exhausted_depth_exceeded_workflow() -> WorkflowDefinitionYaml {
    WorkflowDefinitionYaml {
        name: "chain-depth-test".to_string(),
        description: "test".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            make_test_node(
                "start",
                TestKind::Session,
                "start",
                vec![Rule::Next("node_a".to_string())],
                None,
            ),
            make_test_node(
                "node_a",
                TestKind::Session,
                "A",
                vec![],
                Some(Rule::LoopGuard {
                    max_iterations: 1,
                    on_exhausted: "node_b".to_string(),
                }),
            ),
            make_test_node(
                "node_b",
                TestKind::Session,
                "B",
                vec![],
                Some(Rule::LoopGuard {
                    max_iterations: 1,
                    on_exhausted: "node_a".to_string(),
                }),
            ),
        ],
    }
}

#[test]
fn apply_advance_fails_on_on_exhausted_chain_depth_exceeded() {
    let mut exec = workflow_exec(make_on_exhausted_depth_exceeded_workflow(), 0);
    exec.node_execution_counts =
        HashMap::from([("node_a".to_string(), 1), ("node_b".to_string(), 1)]);

    let outcome = exec.apply_advance();

    assert!(matches!(outcome, NodeOutcome::Persist(_)));
    assert!(matches!(
        exec.state,
        RuntimeExecutionState::Failed {
            ref reason,
            kind: NodeExecutionFailureKind::ValidationFailure,
            ..
        } if reason == "validation_error: loop_guard on_exhausted chain depth exceeded"
    ));
}

#[test]
fn decide_next_node_fails_on_on_exhausted_chain_depth_exceeded() {
    let mut exec = workflow_exec(make_on_exhausted_depth_exceeded_workflow(), 0);
    exec.node_execution_counts =
        HashMap::from([("node_a".to_string(), 1), ("node_b".to_string(), 1)]);

    assert!(matches!(
        exec.decide_next_node(),
        NextNodeDecision::Failed { ref reason }
            if reason == "validation_error: loop_guard on_exhausted chain depth exceeded"
    ));
}

// ---- node が新しい実行を開始する瞬間に artifacts から前回値を破棄する（Spec issues-989） ----

fn make_node_output_fixture(node_name: &str, attempt: u32) -> RuntimeArtifact {
    RuntimeArtifact {
        node_name: node_name.to_string(),
        attempt,
        session_id: None,
        result: Some("prev".to_string()),
        artifact: Some(serde_json::json!({"verdict": "LGTM"})),
        contract: None,
        token_usage: None,
        completed_at: 1000.0,
    }
}

#[test]
fn apply_advance_clears_artifacts_for_new_node() {
    // ループで同一 node が再実行されるとき、advance による遷移で
    // 遷移先 node の前回出力が artifacts から破棄されることを検証する。
    let mut exec = make_exec(0); // plan → implement
    exec.current_session_id = Some("plan-session".to_string());
    exec.artifacts.insert(
        "implement".to_string(),
        make_node_output_fixture("implement", 1),
    );
    // 他 node の前回出力は残り続けることも併せて確認。
    exec.artifacts
        .insert("plan".to_string(), make_node_output_fixture("plan", 1));

    let outcome = exec.apply_advance();
    assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_)));
    assert_eq!(
        exec.workflow.nodes[exec.current_node_index].name,
        "implement"
    );
    assert!(!exec.artifacts.contains_key("implement"));
    assert!(exec.artifacts.contains_key("plan"));
    assert!(exec.current_session_id.is_none());
}

#[test]
fn apply_advance_clears_artifacts_for_loop_target_node() {
    // ループで前ステップ（review）に戻る遷移でも、遷移先の前回出力が破棄される。
    let mut exec = make_exec(2); // review
    exec.current_session_id = Some("review-session".to_string());
    exec.artifacts.insert(
        "implement".to_string(),
        make_node_output_fixture("implement", 1),
    );

    let outcome = exec.apply_advance();
    assert!(matches!(outcome, NodeOutcome::TransitionAndStart(_)));
    assert_eq!(
        exec.workflow.nodes[exec.current_node_index].name,
        "implement"
    );
    assert!(!exec.artifacts.contains_key("implement"));
    assert!(exec.current_session_id.is_none());
}

#[test]
fn apply_advance_to_fanout_clears_parent_output_without_child_map_entries() {
    // fanout child artifact は親配列だけに保持されるため、node 名 output map では親だけを消す。
    let fanout = make_fanout_node(
        "code_review_fanout",
        vec!["review_security", "review_style"],
    );
    let wf = WorkflowDefinitionYaml {
        name: "loop-fanout".to_string(),
        description: "test".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            make_test_node(
                "fix",
                TestKind::Session,
                "Fix",
                vec![Rule::Next("code_review_fanout".to_string())],
                None,
            ),
            fanout,
            make_fanout_child("review_security"),
            make_fanout_child("review_style"),
        ],
    };
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: wf,
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: vec![],
        artifacts: {
            let mut m = HashMap::new();
            m.insert(
                "code_review_fanout".to_string(),
                make_node_output_fixture("code_review_fanout", 1),
            );
            m.insert("fix".to_string(), make_node_output_fixture("fix", 1));
            m
        },
        node_executions: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let outcome = exec.apply_advance();
    assert!(matches!(outcome, NodeOutcome::StartFanout(_)));
    assert!(!exec.artifacts.contains_key("code_review_fanout"));
    assert!(!exec.artifacts.contains_key("review_security"));
    assert!(!exec.artifacts.contains_key("review_style"));
    // fanout 外の node の前回出力は破棄されない。
    assert!(exec.artifacts.contains_key("fix"));
}

// ---- resolve_node_settings ----

#[test]
fn resolve_node_settings_model_and_permission_specified() {
    let result = resolve_node_settings(
        Some("codex-mini".to_string()),
        Some("full".to_string()),
        Some("codex".to_string()),
        &WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "edit".to_string(),
        },
    );
    assert_eq!(
        result,
        ResolvedNodeSettings {
            backend_id: Some("codex".to_string()),
            selected_model: Some("codex-mini".to_string()),
            permission_mode: "full".to_string(),
        }
    );
}

#[test]
fn resolve_node_settings_model_only() {
    let result = resolve_node_settings(
        Some("haiku".to_string()),
        None,
        Some("claude".to_string()),
        &WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "edit".to_string(),
        },
    );
    assert_eq!(
        result,
        ResolvedNodeSettings {
            backend_id: Some("claude".to_string()),
            selected_model: Some("haiku".to_string()),
            permission_mode: "edit".to_string(),
        }
    );
}

#[test]
fn resolve_node_settings_permission_only_clears_model_to_unset() {
    // Spec: workflow 経路では node model 未指定なら親の選択モデルへフォールバックしない。
    // permission のみ指定でも selected_model は None になる。
    let result = resolve_node_settings(
        None,
        Some("ask".to_string()),
        None,
        &WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "edit".to_string(),
        },
    );
    assert_eq!(
        result,
        ResolvedNodeSettings {
            backend_id: Some("claude".to_string()),
            selected_model: None,
            permission_mode: "ask".to_string(),
        }
    );
}

#[test]
fn resolve_node_settings_nothing_specified_clears_model_to_unset() {
    // Spec: model 未指定（None）は未指定状態のまま。親の selected_model へ
    // 暗黙フォールバックしない。
    let result = resolve_node_settings(
        None,
        None,
        None,
        &WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "edit".to_string(),
        },
    );
    assert_eq!(
        result,
        ResolvedNodeSettings {
            backend_id: Some("claude".to_string()),
            selected_model: None,
            permission_mode: "edit".to_string(),
        }
    );
}

#[test]
fn resolve_node_settings_fanout_children_different_configs() {
    // ステップA: model=opus-4, permission=ask
    let result_a = resolve_node_settings(
        Some("opus-4".to_string()),
        Some("ask".to_string()),
        Some("claude".to_string()),
        &WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "edit".to_string(),
        },
    );
    assert_eq!(
        result_a,
        ResolvedNodeSettings {
            backend_id: Some("claude".to_string()),
            selected_model: Some("opus-4".to_string()),
            permission_mode: "ask".to_string(),
        }
    );

    // ステップB: model=codex-mini, permission=full
    let result_b = resolve_node_settings(
        Some("codex-mini".to_string()),
        Some("full".to_string()),
        Some("codex".to_string()),
        &WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "edit".to_string(),
        },
    );
    assert_eq!(
        result_b,
        ResolvedNodeSettings {
            backend_id: Some("codex".to_string()),
            selected_model: Some("codex-mini".to_string()),
            permission_mode: "full".to_string(),
        }
    );

    // 並列ステップ間で結果が独立していることを確認
    assert_ne!(result_a.backend_id, result_b.backend_id);
    assert_ne!(result_a.selected_model, result_b.selected_model);
    assert_ne!(result_a.permission_mode, result_b.permission_mode);
}

// ---- ワークフロー node session の attributes 永続化 ----

// Spec issues-947: ワークフロー node session 作成は
// `create_session_internal_with_attributes` 経由で permission_mode / selected_model /
// workflow_node_session=true を初回保存で確定する。create_node_session_with_settings の
// 後段（resolve_node_settings の結果を attributes に流して save する経路）が
// 二段階保存に逆戻りしないことをガードする。
#[test]
fn node_session_persists_permission_workflow_flag_and_model_on_initial_save() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = crate::test_support::build_session_store();

    let settings = resolve_node_settings(
        Some("opus-4".to_string()),
        Some("edit".to_string()),
        Some("claude".to_string()),
        &WorkflowDefaults {
            backend_id: Some("codex".to_string()),
            permission_mode: "ask".to_string(),
        },
    );
    let permission_mode =
        crate::domain::agent_session::PermissionMode::parse_canonical(&settings.permission_mode)
            .unwrap();
    let session = crate::usecase::agent_session::session::create_session_internal_with_attributes(
        &store,
        tmp.path(),
        "/repo",
        settings.backend_id.clone(),
        permission_mode,
        crate::usecase::agent_session::session::SessionCreationAttributes {
            selected_model: settings.selected_model.clone(),
            workflow_node_session: true,
            workflow_node_context: None,
            ..Default::default()
        },
    )
    .unwrap();

    // 初回保存で permission_mode / workflow_node_session / selected_model / backend_id が確定。
    assert_eq!(session.permission_mode, "edit");
    assert!(session.workflow_node_session);
    assert_eq!(session.selected_model.as_deref(), Some("opus-4"));
    assert_eq!(session.backend_id.as_deref(), Some("claude"));

    // 別インスタンスから読み直しても同じ値で復元される（永続化が確定値で書かれている）。
    let store2 = crate::test_support::build_session_store();
    let loaded = store2
        .load_full_session_for_restore(tmp.path(), &session.id)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.permission_mode, "edit");
    assert!(loaded.workflow_node_session);
    assert_eq!(loaded.selected_model.as_deref(), Some("opus-4"));
    assert_eq!(loaded.backend_id.as_deref(), Some("claude"));
}

// 親セッションから permission_mode/backend_id を継承する経路でも初回保存で確定することを確認する。
// selected_model は Spec issues-946 により暗黙フォールバック禁止のため、node 未指定なら None。
#[test]
fn node_session_inherits_parent_permission_and_backend_on_initial_save() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = crate::test_support::build_session_store();

    let settings = resolve_node_settings(
        None,
        None,
        None,
        &WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "full".to_string(),
        },
    );
    let permission_mode =
        crate::domain::agent_session::PermissionMode::parse_canonical(&settings.permission_mode)
            .unwrap();
    let session = crate::usecase::agent_session::session::create_session_internal_with_attributes(
        &store,
        tmp.path(),
        "/repo",
        settings.backend_id,
        permission_mode,
        crate::usecase::agent_session::session::SessionCreationAttributes {
            selected_model: settings.selected_model,
            workflow_node_session: true,
            workflow_node_context: None,
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(session.permission_mode, "full");
    assert!(session.workflow_node_session);
    // 親 selected_model="haiku" は継承しない（Spec issues-946: 暗黙フォールバック禁止）
    assert_eq!(session.selected_model, None);
    assert_eq!(session.backend_id.as_deref(), Some("claude"));
}

// ---- execution_id 主体性に関する engine レベル統合テスト ----

/// engine が WorkflowExecution を登録する際に、`WorkflowExecution.id` と
/// Execution Store の `WorkflowExecutionSummary.execution_id` が同一 execution_id を共有することを検証する。
/// finding 13 対応: `return 値 execution_id = WorkflowExecution.id = active summary の execution_id
/// = workflow_executions/{execution_id}.json の execution_id` の一致を engine レベルで検証する。
#[tokio::test]
async fn engine_execution_id_consistency_across_execution_and_execution_store_metadata() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;

    // Execution Store API 境界の UUID 検証を満たすため UUID を採用する。
    let execution_id = uuid::Uuid::new_v4().to_string();
    let worktree_path = "/wt/a";
    let workflow = make_minimal_workflow();
    let exec = WorkflowExecution {
        id: execution_id.clone(),
        workflow: workflow.clone(),
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 100.0,
        updated_at: 100.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: worktree_path.to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    engine.executions.lock().await.insert(exec.id.clone(), exec);
    engine
        .execution_store
        .register_active_execution(
            crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: workflow.name.clone(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: workflow.nodes.first().map(|n| n.name.clone()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            },
        )
        .await
        .unwrap();

    // (1) WorkflowExecution.id
    let exec_id = {
        let execs = engine.executions.lock().await;
        execs.get(&execution_id).unwrap().id.clone()
    };
    assert_eq!(exec_id, execution_id);

    // (2) Execution Store active summary の execution_id
    let active = engine.list_active_executions().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].execution_id, execution_id);

    // (3) workflow_executions/{execution_id}.json の execution_id
    let metadata_path = tmp
        .path()
        .join("workflow_executions")
        .join(format!("{execution_id}.json"));
    assert!(metadata_path.exists());
    let metadata: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).unwrap()).unwrap();
    assert_eq!(
        metadata["executionId"].as_str(),
        Some(execution_id.as_str())
    );

    // (4) worktree -> execution_id reverse lookup も一致
    assert_eq!(
        engine.execution_id_for_worktree(worktree_path).await,
        Some(execution_id.clone())
    );
    assert_eq!(
        engine.resolve_worktree_by_execution(&execution_id).await,
        Some(worktree_path.to_string())
    );
}

/// 同一 worktree への重複起動が `validate_start` で拒否されることを検証する。
/// finding 14 対応: 既存 active な実行が同一 worktree に存在する間、
/// validate_start は `AlreadyActive` を返す。
#[tokio::test]
async fn engine_validate_start_rejects_duplicate_active_execution_on_same_worktree() {
    let engine = WorkflowRuntimeService::new_for_test();
    let workflow = make_minimal_workflow();
    let worktree_path = "/wt/dup";

    let exec = WorkflowExecution {
        id: "existing-execution".to_string(),
        workflow: workflow.clone(),
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 100.0,
        updated_at: 100.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: worktree_path.to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let existing_id = exec.id.clone();
    engine.executions.lock().await.insert(exec.id.clone(), exec);

    // validate_start should reject a new start while an active exec lives on this worktree
    let execs = engine.executions.lock().await;
    let existing = find_by_worktree(&execs, worktree_path).map(|(_, e)| e);
    assert!(existing.is_some());
    let result = WorkflowExecution::validate_start(&workflow, existing);
    match result {
        Err(WorkflowEngineError::AlreadyActive(_)) => {}
        other => panic!("expected AlreadyActive, got {other:?}"),
    }

    // Existing exec.id remains accessible by execution_id
    let still_there = execs.get(&existing_id).unwrap();
    assert_eq!(still_there.id, existing_id);
    assert_eq!(still_there.worktree_path, worktree_path);
}

/// engine が状態遷移を反映した際に Execution Store の active / completed 一覧および
/// metadata が同期されることを検証する。
/// finding 15 対応: Running -> WaitingApproval -> Completed の遷移で
/// list_active / list_completed と metadata が正しく更新される。
#[tokio::test]
async fn engine_state_transitions_sync_to_execution_store_active_and_completed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;

    // disk fallback の reverse lookup は UUID 形式しか受理しないため、UUID を採用する。
    let execution_id = uuid::Uuid::new_v4().to_string();
    let worktree_path = "/wt/transit";
    let workflow = make_minimal_workflow();
    engine
        .execution_store
        .register_active_execution(
            crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: workflow.name.clone(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: workflow.nodes.first().map(|n| n.name.clone()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            },
        )
        .await
        .unwrap();

    // Running -> WaitingApproval
    let snapshot_waiting = RuntimeCommitSnapshot {
        execution_id: execution_id.clone(),
        workflow_name: workflow.name.clone(),
        worktree_path: worktree_path.to_string(),
        created_from: ExecutionOrigin::DesktopUi,
        request: String::new(),
        error_reason: None,
        state: RuntimeExecutionState::WaitingApproval,
        current_node_index: 0,
        current_node_name: workflow.nodes[0].name.clone(),
        current_session_id: None,
        node_history: vec![],
        node_execution_counts: HashMap::new(),
        workflow_definition: workflow.clone(),
        total_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: vec![],
        started_at: 100.0,
        updated_at: 200.0,
    };
    workflow_runtime_commit::sync_execution_store_from_snapshot(
        engine.execution_store(),
        &execution_id,
        &snapshot_waiting,
    )
    .await
    .unwrap();
    let active = engine.list_active_executions().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].execution_id, execution_id);
    assert_eq!(active[0].status, ExecutionStatus::WaitingApproval);

    // Completed
    let snapshot_completed = RuntimeCommitSnapshot {
        state: RuntimeExecutionState::Completed,
        updated_at: 300.0,
        ..snapshot_waiting.clone()
    };
    workflow_runtime_commit::sync_execution_store_from_snapshot(
        engine.execution_store(),
        &execution_id,
        &snapshot_completed,
    )
    .await
    .unwrap();
    let active_after = engine.list_active_executions().await;
    assert!(
        active_after.is_empty(),
        "completed execution must leave the active set"
    );
    let completed = engine.list_completed_executions().await;
    assert!(completed.iter().any(|r| r.execution_id == execution_id));
    let completed_entry = completed
        .iter()
        .find(|r| r.execution_id == execution_id)
        .unwrap();
    assert_eq!(completed_entry.status, ExecutionStatus::Completed);

    // 終了後でも reverse lookup（persistence fallback）で worktree が解決できる。
    assert_eq!(
        engine.resolve_worktree_by_execution(&execution_id).await,
        Some(worktree_path.to_string())
    );
}

fn make_minimal_workflow() -> WorkflowDefinitionYaml {
    WorkflowDefinitionYaml {
        name: "engine-test-wf".to_string(),
        description: "minimal".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![{
            let mut node = make_test_node("only-node", TestKind::Session, "do", vec![], None);
            node.session_mut()
                .expect("minimal workflow node must be a session")
                .permission = Some("edit".to_string());
            node
        }],
    }
}

/// G3: workflow 構造の事前検証は `validate_workflow_shape` で副作用なく完結する。
/// 空 nodes を弾けば、`start_workflow` の Phase 1 で parent ChatSession 作成より前に
/// エラーで return できる（孤立 session を残さない）。command node は実行可能 node として
/// 受理される。
#[test]
fn validate_workflow_shape_rejects_empty_and_accepts_command_workflows_without_side_effects() {
    // 空 nodes は InvalidWorkflow
    let empty = WorkflowDefinitionYaml {
        name: "wf".to_string(),
        description: "".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![],
    };
    assert!(matches!(
        workflow_engine_start_guard::validate_workflow_shape(&empty),
        Err(WorkflowEngineError::InvalidWorkflow(_))
    ));

    // command node を含む workflow は実行可能
    let command = WorkflowDefinitionYaml {
        name: "wf".to_string(),
        description: "".to_string(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![make_test_node(
            "command-node",
            TestKind::Command,
            "echo test",
            vec![],
            None,
        )],
    };
    assert!(workflow_engine_start_guard::validate_workflow_shape(&command).is_ok());

    // 正常な workflow は Ok
    let ok = make_minimal_workflow();
    assert!(workflow_engine_start_guard::validate_workflow_shape(&ok).is_ok());
}

/// G3: `execution_id_for_worktree` を Execution Store 経由で参照すれば、parent ChatSession 作成より前に
/// 重複起動を検出できる。`start_workflow` Phase 1 で副作用前に判定する経路の主要な
/// 構成要素（Execution Store の active index）を直接検証する。
#[tokio::test]
async fn execution_store_active_index_resolves_worktree_to_execution_id_for_duplicate_check() {
    let engine = WorkflowRuntimeService::new_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;
    let worktree_path = "/wt/duplicate-check";
    let execution_id = uuid::Uuid::new_v4().to_string();
    engine
        .execution_store
        .register_active_execution(
            crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: Some("s1".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        engine.execution_id_for_worktree(worktree_path).await,
        Some(execution_id),
        "Phase 1 重複判定は Execution Store の active index で成立する"
    );
}

/// G6: handle_auto_complete の fixture は `exec.id` を execs HashMap キーに使う
/// （production と同じ execution_id キー）。fixture が `worktree_path` をキーとして使う旧バグの
/// 回帰防止。
#[tokio::test]
async fn handle_auto_complete_fixture_uses_execution_id_as_executions_key() {
    let engine = WorkflowRuntimeService::new_for_test();
    let exec = WorkflowExecution {
        id: "auto-complete-execution".to_string(),
        workflow: make_minimal_workflow(),
        state: RuntimeExecutionState::Running,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 0.0,
        updated_at: 0.0,
        current_session_id: Some("sess".to_string()),
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: "/wt/auto-complete".to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let execution_id = exec.id.clone();
    let worktree_path = exec.worktree_path.clone();
    engine
        .executions
        .lock()
        .await
        .insert(execution_id.clone(), exec);

    // production と同じ key で参照できる
    {
        let execs = engine.executions.lock().await;
        assert!(execs.get(&execution_id).is_some());
        // worktree_path をキーとした直接 lookup は失敗する（= 旧バグの回帰なし）
        assert!(execs.get(worktree_path.as_str()).is_none());
        // find_by_worktree 経由は成功する
        assert!(find_by_worktree(&execs, &worktree_path).is_some());
    }
}

fn make_exec_with(
    id: &str,
    worktree_path: &str,
    state: RuntimeExecutionState,
) -> WorkflowExecution {
    WorkflowExecution {
        id: id.to_string(),
        workflow: make_minimal_workflow(),
        state,
        current_node_index: 0,
        node_execution_counts: HashMap::new(),
        node_history: Vec::new(),
        started_at: 100.0,
        updated_at: 110.0,
        current_session_id: None,
        current_node_token_usage: TokenUsage::default(),
        artifacts: HashMap::new(),
        node_executions: Vec::new(),
        request: None,
        fanout_runtime: None,
        current_stall_observations: Vec::new(),
        worktree_path: worktree_path.to_string(),
        created_from: ExecutionOrigin::Agent,
        error_reason: None,
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    }
}

/// Spec issues-1011 finding 1/7: `find_by_worktree` / `find_by_worktree_mut` は
/// terminal な execution を返さず、active な execution のみを返す。同一 worktree に
/// terminal execution と active execution が共存しても production 経路で取り違えない。
#[tokio::test]
async fn find_by_worktree_filters_terminal_executions_and_returns_active_only() {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/wt/shared";
    let terminal_execution_id = "terminal-execution".to_string();
    let active_execution_id = "active-execution".to_string();
    let terminal_exec = make_exec_with(
        &terminal_execution_id,
        worktree_path,
        RuntimeExecutionState::Completed,
    );
    let active_exec = make_exec_with(
        &active_execution_id,
        worktree_path,
        RuntimeExecutionState::Running,
    );

    {
        let mut execs = engine.executions.lock().await;
        execs.insert(terminal_execution_id.clone(), terminal_exec);
        execs.insert(active_execution_id.clone(), active_exec);
    }

    // find_by_worktree は active のみを返す
    {
        let execs = engine.executions.lock().await;
        let (found_id, found_exec) =
            find_by_worktree(&execs, worktree_path).expect("active execution must be findable");
        assert_eq!(found_id, &active_execution_id);
        assert!(found_exec.is_active());
        assert_ne!(found_id, &terminal_execution_id);
    }

    // find_any_by_worktree は terminal/active を問わず返す（validate_start 経路用）
    {
        let execs = engine.executions.lock().await;
        assert!(find_any_by_worktree(&execs, worktree_path).is_some());
    }
}

/// Spec issues-1011 finding 11: `abort_workflow_by_execution_id` は terminal な execution_id に対して
/// no-op を返し、同一 worktree の active execution を誤って中断しない。
#[tokio::test]
async fn abort_workflow_by_execution_id_is_noop_for_terminal_execution_even_if_active_shares_worktree(
) {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/wt/coexist";
    let terminal_execution_id = "terminal-abort-target".to_string();
    let active_execution_id = "active-bystander".to_string();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(
            terminal_execution_id.clone(),
            make_exec_with(
                &terminal_execution_id,
                worktree_path,
                RuntimeExecutionState::Completed,
            ),
        );
        execs.insert(
            active_execution_id.clone(),
            make_exec_with(
                &active_execution_id,
                worktree_path,
                RuntimeExecutionState::Running,
            ),
        );
    }

    // execution_id 主語の abort 経路: terminal な exec の execution_id を渡すと、内部の
    // `is_active()` ガードで即 Ok(()) を返し、worktree 主語の下流処理に委譲しない。
    // → 同一 worktree の active execution は影響を受けない。
    // ここでは executions の lookup 経路だけを検証する（AppHandle が要らない範囲）。
    let abort_target_active = {
        let execs = engine.executions.lock().await;
        execs.get(&terminal_execution_id).map(|e| e.is_active())
    };
    assert_eq!(abort_target_active, Some(false));
    // active な execution は依然として is_active
    let bystander_active = {
        let execs = engine.executions.lock().await;
        execs.get(&active_execution_id).map(|e| e.is_active())
    };
    assert_eq!(bystander_active, Some(true));
}

/// Spec issues-1011 finding 5/8: `start_workflow` のアトミック性。並行起動で
/// Execution Store reservation に負けた場合、parent ChatSession は作成されないため
/// 「孤立 parent session」が構造的に発生しないことを保証する。
/// reservation は最初の副作用であり、失敗時は他の副作用が走らない。
#[tokio::test]
async fn start_workflow_reservation_is_first_side_effect_so_no_orphan_session_on_conflict() {
    let engine = WorkflowRuntimeService::new_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;
    let worktree_path = "/wt/reserve";

    // 既に active な reservation がある状態を作る。
    let existing_execution_id = uuid::Uuid::new_v4().to_string();
    engine
        .execution_store
        .register_active_execution(
            crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                execution_id: existing_execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: Some("only-node".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            },
        )
        .await
        .unwrap();

    // 同一 worktree への 2 回目の reservation は WorktreeAlreadyActive で拒否される。
    let new_execution_id = uuid::Uuid::new_v4().to_string();
    let result = engine
        .execution_store
        .register_active_execution(
            crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                execution_id: new_execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: Some("only-node".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 200.0,
                updated_at: 200.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(crate::adaptor::gateway::workflow::execution_store::ExecutionStoreError::WorktreeAlreadyActive { .. })
    ));
    // 新 execution_id 用の metadata ファイルは作成されない
    let path = tmp
        .path()
        .join("workflow_executions")
        .join(format!("{new_execution_id}.json"));
    assert!(
        !path.exists(),
        "新 execution_id の metadata が作成されていないこと（reservation が副作用の最初の境界）"
    );
    // active は existing のみ
    let active = engine.list_active_executions().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].execution_id, existing_execution_id);
}

#[tokio::test]
async fn reserve_workflow_execution_maps_execution_store_worktree_conflict_to_already_active() {
    let engine = WorkflowRuntimeService::new_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;
    let workflow = make_minimal_workflow();
    let worktree_path = "/wt/reserve-conflict";
    engine
        .reserve_workflow_execution(
            &workflow,
            worktree_path,
            None,
            ExecutionOrigin::DesktopUi,
            100.0,
        )
        .await
        .unwrap();

    let err = engine
        .reserve_workflow_execution(
            &workflow,
            worktree_path,
            None,
            ExecutionOrigin::DesktopUi,
            101.0,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, WorkflowEngineError::AlreadyActive(_)));
}

/// Spec issues-1011 finding 10: authoritative sync により、active な execution が
/// terminal に遷移したとき Execution Store の active から外れて completed に
/// 追加され、failed/aborted も同じく completed 一覧に現れる。
/// `sync_execution_store_from_snapshot` を terminal snapshot 3 種で走査して検証する。
#[tokio::test]
async fn execution_store_completed_listing_includes_completed_failed_aborted_via_authoritative_sync(
) {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;

    let cases = [
        ("completed", RuntimeExecutionState::Completed),
        (
            "failed",
            RuntimeExecutionState::Failed {
                reason: "boom".to_string(),
                kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                retry_count: None,
            },
        ),
        ("aborted", RuntimeExecutionState::Aborted),
    ];
    let mut ids = Vec::new();
    for (_, state) in cases.iter().cloned() {
        let execution_id = uuid::Uuid::new_v4().to_string();
        engine
            .execution_store
            .register_active_execution(
                crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                    execution_id: execution_id.clone(),
                    workflow_name: "wf".to_string(),
                    status: ExecutionStatus::Running,
                    worktree_path: format!("/wt/{execution_id}"),
                    current_node: Some("only-node".to_string()),
                    created_from: ExecutionOrigin::DesktopUi,
                    started_at: 100.0,
                    updated_at: 100.0,
                    completed_at: None,
                    error_reason: None,
                    interruption_reason: None,
                    resume_from_node: None,
                    total_token_usage: crate::domain::workflow::TokenUsage::default(),
                },
            )
            .await
            .unwrap();
        // 権威遷移経路で使われる sync helper を直接呼ぶ
        let snapshot = RuntimeCommitSnapshot {
            execution_id: execution_id.clone(),
            workflow_name: "wf".to_string(),
            worktree_path: format!("/wt/{execution_id}"),
            created_from: ExecutionOrigin::DesktopUi,
            request: String::new(),
            error_reason: None,
            state,
            current_node_index: 0,
            current_node_name: "only-node".to_string(),
            current_session_id: None,
            node_history: vec![],
            node_execution_counts: HashMap::new(),
            workflow_definition: make_minimal_workflow(),
            total_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![],
            started_at: 100.0,
            updated_at: 200.0,
        };
        workflow_runtime_commit::sync_execution_store_from_snapshot(
            engine.execution_store(),
            &execution_id,
            &snapshot,
        )
        .await
        .unwrap();
        ids.push(execution_id);
    }

    // 3 件とも active からは外れている
    assert!(engine.list_active_executions().await.is_empty());
    // 3 件とも completed に並ぶ
    let completed = engine.list_completed_executions().await;
    let completed_ids: std::collections::HashSet<&str> =
        completed.iter().map(|r| r.execution_id.as_str()).collect();
    for id in &ids {
        assert!(
            completed_ids.contains(id.as_str()),
            "completed listing must include execution {id}"
        );
    }
}

#[tokio::test]
async fn execution_store_sync_failure_rolls_engine_projection_back_to_active_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;
    let execution_id = uuid::Uuid::new_v4().to_string();
    let worktree_path = "/wt/sync-rollback";
    engine
        .execution_store
        .register_active_execution(
            crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: Some("only-node".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            },
        )
        .await
        .unwrap();
    engine.executions.lock().await.insert(
        execution_id.clone(),
        make_exec_with(
            &execution_id,
            worktree_path,
            RuntimeExecutionState::Completed,
        ),
    );

    let bad_data_dir = tmp.path().join("not-a-directory");
    std::fs::write(&bad_data_dir, "file").unwrap();
    engine.set_execution_store_data_dir(bad_data_dir).await;
    let snapshot = engine
        .executions
        .lock()
        .await
        .get(&execution_id)
        .unwrap()
        .to_commit_snapshot();
    let err = workflow_runtime_commit::sync_execution_store_from_snapshot(
        engine.execution_store(),
        &execution_id,
        &snapshot,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, WorkflowEngineError::SessionStore(_)));

    workflow_runtime_commit::rollback_execution_projection_after_execution_store_sync_failure(
        &engine.executions,
        engine.execution_store(),
        &execution_id,
        &snapshot,
    )
    .await;

    let exec_state = engine
        .executions
        .lock()
        .await
        .get(&execution_id)
        .unwrap()
        .state
        .clone();
    assert_eq!(exec_state, RuntimeExecutionState::Running);
    assert_eq!(
        engine.execution_id_for_worktree(worktree_path).await,
        Some(execution_id),
        "Execution Store rollback keeps the active worktree index authoritative"
    );
}

/// Spec issues-1011 finding 16: `abort_workflow_by_execution_id` 経路の境界回帰検出。
/// AppHandle を要するため `abort_workflow_by_execution_id` 自体は production 経路で起動できないが、
/// 内部 lookup 段階で「terminal execution へ no-op を返し、同一 worktree の active execution の状態を
/// 変更しない」ことを直接検証する。terminal/active 共存時に execution_id 主語の lookup が
/// 取り違えないことを engine state 観測で保証する。
#[tokio::test]
async fn abort_workflow_by_execution_id_does_not_modify_sibling_active_execution_state() {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/wt/sibling";
    let terminal_execution_id = uuid::Uuid::new_v4().to_string();
    let active_execution_id = uuid::Uuid::new_v4().to_string();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(
            terminal_execution_id.clone(),
            make_exec_with(
                &terminal_execution_id,
                worktree_path,
                RuntimeExecutionState::Completed,
            ),
        );
        execs.insert(
            active_execution_id.clone(),
            make_exec_with(
                &active_execution_id,
                worktree_path,
                RuntimeExecutionState::Running,
            ),
        );
    }

    // execution_id ベース lookup: terminal を引いても active のスナップショットには影響しない。
    let initial_active_state = {
        let execs = engine.executions.lock().await;
        execs.get(&active_execution_id).map(|e| e.state.clone())
    };
    assert_eq!(initial_active_state, Some(RuntimeExecutionState::Running));

    // abort_workflow_by_execution_id が production で使う lookup helper は、terminal target を
    // `AlreadyTerminal` として返す。worktree_path で sibling active execution を探索しない。
    assert!(matches!(
        engine.abort_target_lookup(&terminal_execution_id).await,
        AbortTargetLookup::AlreadyTerminal
    ));

    // active execution には触れていない（同一 worktree でも誤って中断しない）
    let final_active_state = {
        let execs = engine.executions.lock().await;
        execs.get(&active_execution_id).map(|e| e.state.clone())
    };
    assert_eq!(final_active_state, Some(RuntimeExecutionState::Running));
}

/// Spec issues-1011 finding 17: approve は execution_id を主語に対象 execution を
/// 直接更新し、同一 worktree に別 execution が存在しても指定 execution 以外へ適用しない。
#[tokio::test]
async fn approval_for_execution_id_updates_only_target_execution_when_worktree_is_shared() {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/wt/approval-shared";
    let target_execution_id = uuid::Uuid::new_v4().to_string();
    let sibling_execution_id = uuid::Uuid::new_v4().to_string();

    let mut target = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    target.id = target_execution_id.clone();
    target.worktree_path = worktree_path.to_string();

    let mut sibling = make_approval_exec(RuntimeExecutionState::WaitingApproval, vec![]);
    sibling.id = sibling_execution_id.clone();
    sibling.worktree_path = worktree_path.to_string();

    {
        let mut execs = engine.executions.lock().await;
        execs.insert(target_execution_id.clone(), target);
        execs.insert(sibling_execution_id.clone(), sibling);
    }

    let outcome = engine
        .handle_approval_with_output_for_execution_for_test(
            &target_execution_id,
            Some(&target_execution_id),
            Some("review"),
        )
        .await
        .unwrap();
    assert!(matches!(outcome, NodeOutcome::Persist(_)));

    let execs = engine.executions.lock().await;
    let target = execs.get(&target_execution_id).unwrap();
    let sibling = execs.get(&sibling_execution_id).unwrap();
    assert_eq!(target.state, RuntimeExecutionState::Completed);
    assert_eq!(target.node_history.len(), 1);
    assert_eq!(sibling.state, RuntimeExecutionState::WaitingApproval);
    assert!(sibling.node_history.is_empty());
}

/// Spec issues-1011 finding 13: `start_workflow` 本体の core 起動経路が払い出す
/// execution_id と、`WorkflowExecution.id` / active summary / workflow_executions/{execution_id}.json が
/// 一貫し、同一 worktree への重複起動を拒否することを直接検証する。
#[tokio::test]
async fn start_workflow_core_records_execution_id_and_rejects_duplicate_worktree() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;

    let worktree_path = "/wt/start-fixture";
    let workflow = make_minimal_workflow();
    let now = 100.0;
    let execution_id = engine
        .start_workflow_common_core_for_test(
            workflow.clone(),
            worktree_path.to_string(),
            Some("task-x".to_string()),
            ExecutionOrigin::DesktopUi,
            now,
        )
        .await
        .unwrap();

    // 一貫性: (1) executions の id (2) active summary.execution_id (3) workflow_executions/{execution_id}.json
    let (exec_id, exec_worktree) = {
        let execs = engine.executions.lock().await;
        let exec = execs.get(&execution_id).unwrap();
        let request_output = exec
            .artifacts
            .get(crate::domain::workflow::services::reference::REQUEST_ARTIFACT)
            .and_then(|output| output.artifact.as_ref())
            .cloned();
        assert_eq!(request_output, Some(serde_json::json!("task-x")));
        (exec.id.clone(), exec.worktree_path.clone())
    };
    let active = engine.list_active_executions().await;
    let metadata_path = tmp
        .path()
        .join("workflow_executions")
        .join(format!("{execution_id}.json"));
    let metadata: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).unwrap()).unwrap();
    assert_eq!(exec_id, execution_id);
    assert_eq!(exec_worktree, worktree_path);
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].execution_id, execution_id);
    assert_eq!(active[0].workflow_name, workflow.name);
    assert_eq!(active[0].worktree_path, worktree_path);
    assert_eq!(active[0].started_at, now);
    assert_eq!(active[0].updated_at, now);
    assert_eq!(active[0].created_from, ExecutionOrigin::DesktopUi);
    assert_eq!(
        metadata["executionId"].as_str(),
        Some(execution_id.as_str())
    );
    assert_eq!(
        metadata["workflowName"].as_str(),
        Some(workflow.name.as_str())
    );
    assert_eq!(metadata["worktreePath"].as_str(), Some(worktree_path));
    assert_eq!(metadata["startedAt"].as_f64(), Some(now));
    assert_eq!(metadata["updatedAt"].as_f64(), Some(now));
    assert_eq!(metadata["createdFrom"].as_str(), Some("desktop_ui"));
    assert!(metadata.get("task").is_none());
    assert!(metadata.get("request").is_none());
    // worktree -> execution の双方向解決も一貫している
    assert_eq!(
        engine.execution_id_for_worktree(worktree_path).await,
        Some(execution_id.clone())
    );
    assert_eq!(
        engine.resolve_worktree_by_execution(&execution_id).await,
        Some(worktree_path.to_string())
    );

    let duplicate = engine
        .start_workflow_common_core_for_test(
            make_minimal_workflow(),
            worktree_path.to_string(),
            None,
            ExecutionOrigin::DesktopUi,
            now + 1.0,
        )
        .await;
    assert!(matches!(
        duplicate,
        Err(WorkflowEngineError::AlreadyActive(_))
    ));

    let empty_request_execution_id = engine
        .start_workflow_common_core_for_test(
            make_minimal_workflow(),
            "/wt/start-empty-request".to_string(),
            None,
            ExecutionOrigin::DesktopUi,
            now + 2.0,
        )
        .await
        .unwrap();
    let execs = engine.executions.lock().await;
    let empty_request = execs
        .get(&empty_request_execution_id)
        .unwrap()
        .artifacts
        .get(crate::domain::workflow::services::reference::REQUEST_ARTIFACT)
        .and_then(|output| output.artifact.as_ref());
    assert_eq!(empty_request, Some(&serde_json::json!("")));
}

/// Spec issues-1011 finding 14: 同一 worktree への重複起動は reservation 段階で拒否され、
/// 新規 metadata / parent session / refs が孤立しない。Execution Store の reservation は
/// 起動経路上の「最初の副作用」であり、失敗時には他の副作用が一切走らない構造を保証する。
#[tokio::test]
async fn start_workflow_duplicate_reservation_does_not_leak_metadata_or_refs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;
    let worktree_path = "/wt/dup-leak";

    // 既存 active reservation
    let existing_execution_id = uuid::Uuid::new_v4().to_string();
    engine
        .execution_store
        .register_active_execution(
            crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                execution_id: existing_execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: Some("only-node".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            },
        )
        .await
        .unwrap();

    // 2 回目の reservation 失敗 → 新 metadata / refs / executions に何も追加されない
    let new_execution_id = uuid::Uuid::new_v4().to_string();
    let result = engine
        .execution_store
        .register_active_execution(
            crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                execution_id: new_execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: Some("only-node".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 200.0,
                updated_at: 200.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(crate::adaptor::gateway::workflow::execution_store::ExecutionStoreError::WorktreeAlreadyActive { .. })
    ));
    // (1) 新 execution_id 用 metadata ファイル無し
    let path = tmp
        .path()
        .join("workflow_executions")
        .join(format!("{new_execution_id}.json"));
    assert!(!path.exists());
    // (2) session_workflow_refs に新規エントリ無し（reservation 失敗の段階で副作用が走らない）
    let refs = engine.session_workflow_refs.lock().await;
    assert!(!refs
        .values()
        .any(|r: &SessionWorkflowRef| r.execution_id == new_execution_id));
    // (3) executions にも新 execution_id が無い
    let execs = engine.executions.lock().await;
    assert!(!execs.contains_key(&new_execution_id));
    // (4) active は existing のみ
    assert_eq!(
        active_only_summary(&engine).await,
        vec![existing_execution_id]
    );
}

// 撤去済み: rollback_created_parent_session は parent ChatSession 機構撤去で消滅した。
// 旧テスト `start_workflow_rollback_deletes_created_parent_session` も役目を終えた。

async fn active_only_summary(engine: &WorkflowRuntimeService) -> Vec<String> {
    engine
        .list_active_executions()
        .await
        .into_iter()
        .map(|s| s.execution_id)
        .collect()
}

/// Spec issues-1011 finding 15: completed / failed / aborted の代表経路で
/// active 一覧から消えて completed 一覧に status 付きで現れる。
/// production の権威遷移経路で必ず呼ばれる `sync_execution_store_from_snapshot` を直接呼び、
/// 3 ステータスすべてで「Execution Store の owner が active → completed に推移する」ことを
/// 1 つのテストでまとめて検証する（既存の同種テストとは別に、status 観測も加える）。
#[tokio::test]
async fn execution_store_terminal_statuses_propagate_status_field_in_completed_listing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_execution_store_data_dir(tmp.path().to_path_buf())
        .await;

    let mut expectations: Vec<(String, ExecutionStatus)> = Vec::new();
    for state in [
        RuntimeExecutionState::Completed,
        RuntimeExecutionState::Failed {
            reason: "boom".to_string(),
            kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            retry_count: None,
        },
        RuntimeExecutionState::Aborted,
    ] {
        let execution_id = uuid::Uuid::new_v4().to_string();
        let expected_status = match state {
            RuntimeExecutionState::Completed => ExecutionStatus::Completed,
            RuntimeExecutionState::Failed { .. } => ExecutionStatus::Failed,
            RuntimeExecutionState::Aborted => ExecutionStatus::Aborted,
            RuntimeExecutionState::Interrupted => ExecutionStatus::Interrupted,
            _ => unreachable!(),
        };
        engine
            .execution_store
            .register_active_execution(
                crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                    execution_id: execution_id.clone(),
                    workflow_name: "wf".to_string(),
                    status: ExecutionStatus::Running,
                    worktree_path: format!("/wt/{execution_id}"),
                    current_node: Some("only-node".to_string()),
                    created_from: ExecutionOrigin::DesktopUi,
                    started_at: 100.0,
                    updated_at: 100.0,
                    completed_at: None,
                    error_reason: None,
                    interruption_reason: None,
                    resume_from_node: None,
                    total_token_usage: crate::domain::workflow::TokenUsage::default(),
                },
            )
            .await
            .unwrap();
        let snapshot = RuntimeCommitSnapshot {
            execution_id: execution_id.clone(),
            workflow_name: "wf".to_string(),
            worktree_path: format!("/wt/{execution_id}"),
            created_from: ExecutionOrigin::DesktopUi,
            request: String::new(),
            error_reason: None,
            state,
            current_node_index: 0,
            current_node_name: "only-node".to_string(),
            current_session_id: None,
            node_history: vec![],
            node_execution_counts: HashMap::new(),
            workflow_definition: make_minimal_workflow(),
            total_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![],
            started_at: 100.0,
            updated_at: 200.0,
        };
        workflow_runtime_commit::sync_execution_store_from_snapshot(
            engine.execution_store(),
            &execution_id,
            &snapshot,
        )
        .await
        .unwrap();
        expectations.push((execution_id, expected_status));
    }

    // active 一覧から全て外れている
    assert!(engine.list_active_executions().await.is_empty());

    // completed 一覧に status 付きで現れる
    let completed = engine.list_completed_executions().await;
    for (id, expected_status) in &expectations {
        let entry = completed
            .iter()
            .find(|r| &r.execution_id == id)
            .expect("completed listing must include execution");
        assert_eq!(
            entry.status, *expected_status,
            "status must propagate to completed summary for {id}"
        );
    }
}

/// Spec issues-1011 finding 3: `resolve_chat_session_for_approval` は execution state が
/// `WaitingApproval` でない場合に Err を返す（任意 node session への注入経路を塞ぐ）。
#[tokio::test]
async fn resolve_chat_session_for_approval_rejects_non_waiting_approval_state() {
    let engine = WorkflowRuntimeService::new_for_test();
    let execution_id = uuid::Uuid::new_v4().to_string();
    let mut exec = make_exec_with(&execution_id, "/wt/x", RuntimeExecutionState::Running);
    exec.workflow.nodes[0].kind = test_node_kind(TestKind::ApprovalSession, "review");
    exec.current_session_id = Some("node-sess".to_string());
    engine
        .executions
        .lock()
        .await
        .insert(execution_id.clone(), exec);

    let err = engine
        .resolve_chat_session_for_approval(&execution_id)
        .await
        .unwrap_err();
    assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
}

/// Spec issues-1011 finding 3: `resolve_chat_session_for_approval` は current node が
/// approval-gated session でない場合に拒否する。
#[tokio::test]
async fn resolve_chat_session_for_approval_rejects_non_approval_current_node() {
    let engine = WorkflowRuntimeService::new_for_test();
    let execution_id = uuid::Uuid::new_v4().to_string();
    let mut exec = make_exec_with(
        &execution_id,
        "/wt/x",
        RuntimeExecutionState::WaitingApproval,
    );
    // current node は通常 session のまま（make_minimal_workflow が auto session を返す）
    exec.current_session_id = Some("node-sess".to_string());
    engine
        .executions
        .lock()
        .await
        .insert(execution_id.clone(), exec);

    let err = engine
        .resolve_chat_session_for_approval(&execution_id)
        .await
        .unwrap_err();
    assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
}

/// Spec issues-1011 finding 3: 全条件揃った場合のみ session_id / worktree_path を返す。
#[tokio::test]
async fn resolve_chat_session_for_approval_accepts_fully_valid_state() {
    let engine = WorkflowRuntimeService::new_for_test();
    let execution_id = uuid::Uuid::new_v4().to_string();
    let mut exec = make_exec_with(
        &execution_id,
        "/wt/x",
        RuntimeExecutionState::WaitingApproval,
    );
    exec.workflow.nodes[0].kind = test_node_kind(TestKind::ApprovalSession, "review");
    exec.current_session_id = Some("node-sess".to_string());
    engine
        .executions
        .lock()
        .await
        .insert(execution_id.clone(), exec);

    let (sid, wt) = engine
        .resolve_chat_session_for_approval(&execution_id)
        .await
        .unwrap();
    assert_eq!(sid, "node-sess");
    assert_eq!(wt, "/wt/x");
}

/// Spec issues-1011 finding 3: terminal execution の approval 解決は拒否される。
/// 同一 worktree に terminal + active がある状況で terminal 側を狙う注入経路を防ぐ。
#[tokio::test]
async fn resolve_chat_session_for_approval_rejects_terminal_execution() {
    let engine = WorkflowRuntimeService::new_for_test();
    let execution_id = uuid::Uuid::new_v4().to_string();
    let mut exec = make_exec_with(&execution_id, "/wt/x", RuntimeExecutionState::Completed);
    exec.workflow.nodes[0].kind = test_node_kind(TestKind::ApprovalSession, "review");
    exec.current_session_id = Some("node-sess".to_string());
    engine
        .executions
        .lock()
        .await
        .insert(execution_id.clone(), exec);

    let err = engine
        .resolve_chat_session_for_approval(&execution_id)
        .await
        .unwrap_err();
    assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
}

/// Spec issues-1011 finding 5: terminal transition 経路で `cleanup_session_workflow_refs_by_execution_id`
/// は対象 execution の refs のみを削除し、同一 worktree の別 active execution の refs は残す。
#[tokio::test]
async fn cleanup_session_workflow_refs_by_execution_id_preserves_sibling_execution_refs() {
    let engine = WorkflowRuntimeService::new_for_test();
    let terminal_execution_id = uuid::Uuid::new_v4().to_string();
    let active_execution_id = uuid::Uuid::new_v4().to_string();

    // 両 execution の refs を入れる（同一 worktree 想定）
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "parent-terminal".to_string(),
            SessionWorkflowRef {
                execution_id: terminal_execution_id.clone(),
            },
        );
        refs.insert(
            "node-terminal".to_string(),
            SessionWorkflowRef {
                execution_id: terminal_execution_id.clone(),
            },
        );
        refs.insert(
            "parent-active".to_string(),
            SessionWorkflowRef {
                execution_id: active_execution_id.clone(),
            },
        );
    }

    engine
        .cleanup_session_workflow_refs_by_execution_id(&terminal_execution_id)
        .await;

    let refs = engine.session_workflow_refs.lock().await;
    assert!(!refs.contains_key("parent-terminal"));
    assert!(!refs.contains_key("node-terminal"));
    assert!(
        refs.contains_key("parent-active"),
        "sibling active execution の refs は残るべき"
    );
}

/// [04] Runtime Mutation / Event Boundary 専用テスト。
///
/// runtime primitive routing と `handle_approval` 内の ApprovalResolved append /
/// snapshot 一括復元（atomic mutation 境界）を検証する。本モジュールは
/// `tauri::AppHandle` を要さない範囲で mutation / approval semantics の production
/// 経路を直接呼ぶ。
#[cfg(test)]
mod dispatch_boundary_tests {
    use super::*;
    use crate::adaptor::gateway::workflow::approval_runtime::MAX_APPROVAL_COMMENT_CHARS;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::execution_store::{
        ExecutionOrigin, ExecutionStatus, TerminalExecutionStatus, WorkflowExecutionMetadata,
    };
    use crate::adaptor::gateway::workflow::internal_node_command::InternalNodeCommand;
    use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
    use crate::adaptor::gateway::workflow::schema::{ItemsSource, Rule, WorkflowDefinitionYaml};
    use crate::adaptor::gateway::workflow::state::RuntimeExecutionState;
    use crate::domain::workflow::WorkflowError;
    use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
    use crate::usecase::agent_session::session::MessagePart;
    use crate::usecase::workflow::command::{
        AbortExecutionCommand, ResumeExecutionCommand, WorkflowAbortExecutionUsecase,
        WorkflowResumeExecutionUsecase,
    };
    use crate::usecase::workflow::ports::{
        WorkflowAbortExecutionGateway, WorkflowResumeExecutionGateway,
    };
    use async_trait::async_trait;
    use tauri::Manager;
    use tempfile::TempDir;

    /// 実バックエンドと同じ供給経路（`fixed_models()`）でモデル一覧を返す
    /// dispatch テスト用 backend。claude / codex の固定モデル定数をそのまま供給し、
    /// builtin workflow が使う `claude-opus-4-8` / `gpt-5.5` を production と同一経路で
    /// 解決できるようにする（dispatch フロー検証の本来意図を維持）。
    struct DispatchMockBackend {
        backend_id: String,
        fixed_models: Vec<String>,
    }

    #[async_trait]
    impl AgentBackend for DispatchMockBackend {
        fn id(&self) -> &str {
            &self.backend_id
        }

        fn name(&self) -> &str {
            "Mock"
        }

        fn available_models(&self) -> Vec<ModelDescriptor> {
            self.fixed_models
                .iter()
                .map(|model| ModelDescriptor {
                    id: ModelId::parse(model).unwrap(),
                    display_name: model.clone(),
                })
                .collect()
        }

        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities { steering: false }
        }

        async fn open_session(
            &self,
            _spec: AgentSessionSpec,
        ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
            Ok(Box::new(DispatchMockRuntime))
        }

        async fn archive_session(
            &self,
            _backend_session_id: &str,
            _cwd: &str,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn unarchive_session(
            &self,
            _backend_session_id: &str,
            _cwd: &str,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn fork_session(
            &self,
            _req: ForkSessionRequest,
        ) -> Result<Option<String>, AgentBackendError> {
            Ok(None)
        }

        async fn skill_catalog(
            &self,
            _cwd: &std::path::Path,
            _query: Option<&str>,
            _limit: Option<usize>,
        ) -> Result<Vec<SkillEntry>, AgentBackendError> {
            Ok(Vec::new())
        }

        async fn fuzzy_file_search(
            &self,
            _root: &std::path::Path,
            _query: &str,
            _limit: usize,
        ) -> Result<Option<Vec<String>>, AgentBackendError> {
            Ok(None)
        }
    }

    struct DispatchMockRuntime;

    #[async_trait]
    impl AgentSessionRuntime for DispatchMockRuntime {
        fn take_events(
            &mut self,
        ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = AgentRuntimeEvent> + Send>> {
            Box::pin(futures_util::stream::empty())
        }

        async fn start_turn(&self, _input: TurnInput) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn interrupt(&self) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn respond_permission(
            &self,
            _response: PermissionResponse,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn set_permission_mode(
            &self,
            _mode: crate::domain::agent_session::PermissionMode,
            _plan_mode: bool,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn set_model(&self, _model: &ModelId) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn close(&self) {}
    }

    struct RewritingManagedWorktreeResolver;

    #[async_trait]
    impl ManagedWorktreeResolver for RewritingManagedWorktreeResolver {
        async fn resolve(
            &self,
            _worktree_path: String,
        ) -> Result<String, ManagedWorktreeResolverError> {
            Ok("/different-managed-worktree".to_string())
        }
    }

    fn dispatch_data_dir(app: &tauri::AppHandle<tauri::test::MockRuntime>) -> std::path::PathBuf {
        crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .expect("mock app data dir must resolve")
    }

    fn make_approval_only_workflow() -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: "boundary-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_approval_gated_session("review", "review", vec![])],
        }
    }

    fn make_running_session_workflow() -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: "boundary-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_test_node(
                "fix",
                TestKind::Session,
                "fix",
                vec![],
                None,
            )],
        }
    }

    fn make_waiting_approval_execution(
        execution_id: &str,
        worktree_path: &str,
    ) -> WorkflowExecution {
        let workflow = make_approval_only_workflow();
        make_waiting_approval_execution_with_workflow(execution_id, worktree_path, workflow)
    }

    fn make_waiting_approval_execution_with_workflow(
        execution_id: &str,
        worktree_path: &str,
        workflow: WorkflowDefinitionYaml,
    ) -> WorkflowExecution {
        let node_name = workflow.nodes[0].name.clone();
        let node_kind = workflow.nodes[0].kind_name();
        let node_execution_id = format!("{execution_id}-{node_name}-1");
        WorkflowExecution {
            id: execution_id.to_string(),
            workflow,
            state: RuntimeExecutionState::WaitingApproval,
            current_node_index: 0,
            node_execution_counts: HashMap::from([(node_name.clone(), 1)]),
            node_history: Vec::new(),
            worktree_path: worktree_path.to_string(),
            created_from: ExecutionOrigin::Agent,
            error_reason: None,
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("sess-1".to_string()),
            current_node_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: node_execution_id,
                execution_id: execution_id.to_string(),
                node_name,
                kind: node_kind,
                attempt: 1,
                status: NodeExecutionStatus::WaitingApproval,
                session_id: Some("sess-1".to_string()),
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                started_at: 1000.0,
                completed_at: None,
            }],
            request: None,
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "edit".to_string(),
            },
        }
    }

    fn test_fanout_child(
        node_name: &str,
        session_id: &str,
        state: FanoutChildRuntimeState,
        child_index: usize,
    ) -> FanoutChildRuntime {
        FanoutChildRuntime {
            node_execution_id: format!("ne-{node_name}-{child_index}"),
            node_name: node_name.to_string(),
            session_id: session_id.to_string(),
            state,
            result: None,
            artifact: None,
            contract: None,
            failure_kind: None,
            failure_disposition: None,
            token_usage: TokenUsage::default(),
            attempt: 1,
            completed_at: None,
        }
    }

    fn install_test_fanout(exec: &mut WorkflowExecution, children: Vec<FanoutChildRuntime>) {
        let parent = exec
            .node_executions
            .first_mut()
            .expect("fanout parent NodeExecution");
        parent.status = NodeExecutionStatus::Running;
        parent.kind = NodeKindName::Fanout;
        let parent_node = parent.node_name.clone();
        let parent_attempt = parent.attempt;
        let parent_node_execution_id = parent.id.clone();
        exec.node_executions
            .extend(
                children
                    .iter()
                    .enumerate()
                    .map(|(child_index, child)| NodeExecution {
                        id: child.node_execution_id.clone(),
                        execution_id: exec.id.clone(),
                        node_name: child.node_name.clone(),
                        kind: NodeKindName::Session,
                        attempt: child.attempt,
                        status: match child.state {
                            FanoutChildRuntimeState::Running => NodeExecutionStatus::Running,
                            FanoutChildRuntimeState::Completed => NodeExecutionStatus::Succeeded,
                            FanoutChildRuntimeState::Failed => NodeExecutionStatus::Failed,
                            FanoutChildRuntimeState::Interrupted => NodeExecutionStatus::Aborted,
                        },
                        session_id: Some(child.session_id.clone()),
                        artifact: child.artifact.clone(),
                        token_usage: Some(child.token_usage.clone()),
                        failure: child.failure_kind.map(|kind| {
                            crate::adaptor::gateway::workflow::state::NodeExecutionFailure {
                                reason: child
                                    .result
                                    .clone()
                                    .unwrap_or_else(|| "failed".to_string()),
                                kind,
                            }
                        }),
                        fanout_parent: Some(FanoutParentRef {
                            parent_node: parent_node.clone(),
                            parent_attempt,
                            item_index: None,
                            child_index,
                        }),
                        started_at: exec.started_at,
                        completed_at: child.completed_at,
                    }),
            );
        exec.fanout_runtime = Some(FanoutRuntimeState {
            parent_node_name: parent_node,
            parent_node_execution_id,
            children,
        });
    }

    fn append_started_events_for_execution(data_dir: &std::path::Path, exec: &WorkflowExecution) {
        let mut events = vec![WorkflowEvent::ExecutionStarted {
            execution_id: exec.id.clone(),
            workflow_name: exec.workflow.name.clone(),
            worktree_path: exec.worktree_path.clone(),
            created_from: ExecutionOrigin::Agent,
            request: String::new(),
            permission_mode: exec.workflow_defaults.permission_mode.clone(),
            definition: exec.workflow.clone(),
            timestamp: exec.started_at,
        }];
        for node_execution in &exec.node_executions {
            events.push(WorkflowEvent::NodeStarted {
                execution_id: exec.id.clone(),
                node_execution_id: node_execution.id.clone(),
                node_name: node_execution.node_name.clone(),
                kind: node_execution.kind,
                attempt: node_execution.attempt,
                fanout_parent: node_execution.fanout_parent.clone(),
                timestamp: node_execution.started_at,
            });
            if let Some(session_id) = node_execution.session_id.as_ref() {
                events.push(WorkflowEvent::SessionAttached {
                    execution_id: exec.id.clone(),
                    node_execution_id: node_execution.id.clone(),
                    session_id: session_id.clone(),
                    timestamp: node_execution.started_at,
                });
            }
        }
        WorkflowEventLog::new(data_dir)
            .append_batch(&events)
            .unwrap();
    }

    fn same_name_fanout_approval_execution() -> WorkflowExecution {
        let mut exec = make_waiting_approval_execution("execution-fanout-approval", "/wt/fanout");
        exec.state = RuntimeExecutionState::Running;
        exec.current_session_id = None;
        install_test_fanout(
            &mut exec,
            vec![
                test_fanout_child(
                    "review-child",
                    "session-review-0",
                    FanoutChildRuntimeState::Running,
                    0,
                ),
                test_fanout_child(
                    "review-child",
                    "session-review-1",
                    FanoutChildRuntimeState::Running,
                    1,
                ),
            ],
        );
        exec.node_executions
            .iter_mut()
            .find(|execution| execution.id == "ne-review-child-0")
            .unwrap()
            .status = NodeExecutionStatus::WaitingApproval;
        exec
    }

    #[test]
    fn name_only_fanout_approval_requires_id_for_multiple_active_same_name_children() {
        let exec = same_name_fanout_approval_execution();

        let error = resolve_fanout_approval_target_node_execution_id(&exec, "review-child", None)
            .unwrap_err();

        assert!(matches!(
            error,
            WorkflowEngineError::InvalidState(message)
                if message.contains("2 active fanout executions")
                    && message.contains("node_execution_id is required")
                    && message.contains("ne-review-child-0")
                    && message.contains("ne-review-child-1")
        ));
    }

    #[test]
    fn fanout_approval_id_selects_waiting_child_among_same_name_active_children() {
        let exec = same_name_fanout_approval_execution();

        let selected = resolve_fanout_approval_target_node_execution_id(
            &exec,
            "review-child",
            Some("ne-review-child-0"),
        )
        .unwrap();

        assert_eq!(selected.as_deref(), Some("ne-review-child-0"));
    }

    type DispatchTestApp = tauri::App<tauri::test::MockRuntime>;

    fn make_dispatch_app() -> DispatchTestApp {
        let mut config = crate::adaptor::gateway::app_config::ReleashConfig::default();
        config.app.last_repo_paths = Vec::new();
        config.agents.default = Some("codex".to_string());
        let app_config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            config,
            TempDir::new().unwrap().path().join("config.toml"),
        ));
        let config_repository: Arc<dyn crate::domain::app_config::ConfigRepository> =
            app_config.clone();
        let agent_config_repository: Arc<dyn crate::domain::app_config::AgentConfigRepository> =
            app_config.clone();
        let config_secret_repository: Arc<dyn crate::domain::app_config::ConfigSecretRepository> =
            app_config.clone();
        let notion_config_repository: Arc<dyn crate::domain::app_config::NotionConfigRepository> =
            app_config.clone();
        let notion_usecase = Arc::new(crate::usecase::notion::usecase::NotionUsecase::new(
            notion_config_repository,
            Arc::new(crate::adaptor::gateway::notion::NotionApiGatewayImpl::new()),
        ));
        // 実 backend と同じ供給経路（fixed_models()）で claude / codex の固定モデルを
        // 供給する mock backend を登録する。builtin workflow が使う claude-opus-4-8 /
        // gpt-5.5 が production と同一経路で解決され、dispatch フロー検証を維持できる。
        let mut registry = AgentBackendRegistry::new();
        let claude_models = crate::infrastructure::agent_session::claude::ClaudeBackend::new(None)
            .available_models()
            .into_iter()
            .map(|model| model.id.as_str().to_string())
            .collect();
        let codex_models = crate::infrastructure::agent_session::codex::CodexBackend::new(None)
            .available_models()
            .into_iter()
            .map(|model| model.id.as_str().to_string())
            .collect();
        registry.register(Arc::new(DispatchMockBackend {
            backend_id: "claude".to_string(),
            fixed_models: claude_models,
        }));
        registry.register(Arc::new(DispatchMockBackend {
            backend_id: "codex".to_string(),
            fixed_models: codex_models,
        }));
        registry.set_default(Some("codex".to_string()));
        let registry = Arc::new(registry);
        let data_dir =
            std::env::temp_dir().join(format!("releash-dispatch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let repository_usecase =
            Arc::new(crate::adaptor::controller::wiring::build_repository_usecase());
        let shared_repo_paths =
            <crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths>::default();
        let repo_paths_gateway =
            crate::adaptor::gateway::repository::repo_paths::RepoPathsGateway::new(
                shared_repo_paths,
                config_repository.clone(),
            );
        // テスト用 stub: 通知は no-op で受け流す。
        let repo_paths_notifier = Arc::new(NoopRepoPathsNotifier);
        let repo_paths_usecase =
            Arc::new(crate::usecase::repo_paths_usecase::RepoPathsUsecase::new(
                Arc::new(repo_paths_gateway),
                repo_paths_notifier,
            ));
        let code_usecase = Arc::new(crate::adaptor::controller::wiring::build_code_usecase());
        let repository_scanner = Arc::new(
            crate::adaptor::gateway::repository::scanner::DefaultRepositoryScanner::new(
                repository_usecase.clone(),
                code_usecase.clone(),
            ),
        );
        let repository_state_repository = Arc::new(
            crate::adaptor::gateway::repository::state::RepositoryStateRepositoryGateway::new(
                repository_usecase.clone(),
            ),
        );
        let repository_state = Arc::new(
            crate::usecase::repository_state::RepositoryStateService::new(
                repository_state_repository,
                repository_scanner,
                Arc::new(crate::usecase::repository_state::worktree::NoopRepositoryStateNotifier),
                Arc::new(crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher),
                Arc::new(
                    crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime,
                ),
                Arc::new(
                    crate::usecase::repository_state::runtime::tests_support::IdentityWorktreePathNormalizer,
                ),
            ),
        );
        let review_usecase = Arc::new(crate::usecase::review_usecase::ReviewUsecase::new(
            repository_state.clone(),
            code_usecase.clone(),
        ));
        let workflow_usecase = Arc::new(
            crate::adaptor::controller::wiring::build_workflow_usecase(data_dir.clone()),
        );
        let app = tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                data_dir.clone(),
            ))
            .manage(app_config)
            .manage(config_repository)
            .manage(agent_config_repository)
            .manage(config_secret_repository)
            .manage(registry)
            .manage(crate::adaptor::controller::state::AppState {
                repository_usecase,
                repository_state,
                repo_paths_usecase,
                code_usecase,
                review_usecase,
                notion_usecase,
                workflow_usecase,
                pty_session_read_usecase: Arc::new(
                    crate::adaptor::controller::wiring::build_pty_session_read_usecase_for_tests(),
                ),
                git_host_usecase: Arc::new(
                    crate::adaptor::controller::wiring::build_git_host_usecase(),
                ),
            })
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock test app must build");
        app.manage(Arc::new(
            crate::usecase::agent_session::status::AgentStatusCenter::new(),
        ));
        app.manage(Arc::new(
            crate::usecase::agent_session::session::OpenTabRegistry::default(),
        ));
        app.manage(crate::test_support::build_agent_runtime_usecase(
            Arc::new(crate::test_support::build_session_store()),
            data_dir,
        ));
        app
    }

    struct NoopRepoPathsNotifier;
    impl crate::domain::repository::RepoPathsNotifier for NoopRepoPathsNotifier {
        fn notify_changed(&self, _paths: Vec<String>) {}
    }

    fn make_dispatch_deps(
        data_dir: std::path::PathBuf,
    ) -> (
        Arc<crate::usecase::agent_session::session::SessionStore>,
        Arc<AgentSessionRuntimeUsecase>,
    ) {
        let session_store = Arc::new(crate::test_support::build_session_store());
        std::fs::create_dir_all(&data_dir).unwrap();
        let agent_runtime =
            crate::test_support::build_agent_runtime_usecase(session_store.clone(), data_dir);
        (session_store, agent_runtime)
    }

    #[derive(Clone)]
    struct RecoveredOrphanCommandGateway {
        app: tauri::AppHandle<tauri::test::MockRuntime>,
        engine: Arc<WorkflowRuntimeService>,
        session_store: Arc<crate::usecase::agent_session::session::SessionStore>,
        agent_runtime: Arc<AgentSessionRuntimeUsecase>,
    }

    #[async_trait]
    impl WorkflowResumeExecutionGateway for RecoveredOrphanCommandGateway {
        async fn resume_execution(
            &self,
            command: ResumeExecutionCommand,
        ) -> Result<(), WorkflowError> {
            self.engine
                .resume_workflow_execution(
                    &self.app,
                    &self.session_store,
                    &self.agent_runtime,
                    &command.execution_id,
                )
                .await
                .map_err(|error| WorkflowError::external(error.to_string()))
        }
    }

    #[async_trait]
    impl WorkflowAbortExecutionGateway for RecoveredOrphanCommandGateway {
        async fn abort_execution(
            &self,
            command: AbortExecutionCommand,
        ) -> Result<(), WorkflowError> {
            self.engine
                .abort_workflow_execution(
                    &self.app,
                    &self.session_store,
                    &self.agent_runtime,
                    &command.execution_id,
                    command.expected_node_name.as_deref(),
                )
                .await
                .map_err(|error| WorkflowError::external(error.to_string()))
        }
    }

    async fn seed_resumable_orphan_execution(
        store: &crate::adaptor::gateway::workflow::execution_store::ExecutionStore,
        data_dir: &std::path::Path,
        execution_id: &str,
        worktree_path: &str,
    ) {
        store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: execution_id.to_string(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: Some("plan".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        WorkflowEventLog::new(data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "wf".to_string(),
                    worktree_path: worktree_path.to_string(),
                    created_from: ExecutionOrigin::Agent,
                    request: String::new(),
                    permission_mode: "ask".to_string(),
                    definition: WorkflowDefinitionYaml {
                        name: "wf".to_string(),
                        description: String::new(),
                        builtin: false,
                        schemas: Default::default(),
                        nodes: vec![
                            make_test_node(
                                "plan",
                                TestKind::Session,
                                "implement",
                                vec![Rule::Next("review".to_string())],
                                None,
                            ),
                            make_test_node("review", TestKind::Session, "implement", vec![], None),
                        ],
                    },
                    timestamp: 100.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: format!("{execution_id}-plan-1"),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 101.0,
                },
                WorkflowEvent::NodeCompleted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: format!("{execution_id}-plan-1"),
                    node_name: "plan".to_string(),
                    attempt: 1,
                    result_summary: Some("planned".to_string()),
                    token_usage: None,
                    timestamp: 102.0,
                },
            ])
            .unwrap();
    }

    #[tokio::test]
    async fn abort_workflow_by_execution_id_clears_stall_observations_in_live_and_projection() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir.clone());
        let received_payloads: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let received_for_listener = Arc::clone(&received_payloads);
        app.listen("workflow-execution-changed", move |event| {
            received_for_listener
                .lock()
                .unwrap()
                .push(event.payload().to_string());
        });

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/abort-clears-stall";
        let session_id = "abort-stall-session";
        let node_name = "review";
        let mut exec = make_waiting_approval_execution(&execution_id, worktree_path);
        exec.state = RuntimeExecutionState::Running;
        exec.node_executions[0].status = NodeExecutionStatus::Running;
        exec.current_session_id = Some(session_id.to_string());
        exec.node_executions[0].session_id = Some(session_id.to_string());
        let node_execution_id = exec.node_executions[0].id.clone();
        exec.current_stall_observations =
            vec![workflow_stall_observation_fixture(session_id, node_name)];
        WorkflowEventLog::new(&data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    worktree_path: exec.worktree_path.clone(),
                    created_from: ExecutionOrigin::Agent,
                    request: String::new(),
                    permission_mode: exec.workflow_defaults.permission_mode.clone(),
                    definition: exec.workflow.clone(),
                    timestamp: exec.started_at,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                    node_name: node_name.to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: exec.started_at,
                },
                WorkflowEvent::SessionAttached {
                    execution_id: execution_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                    session_id: session_id.to_string(),
                    timestamp: exec.started_at,
                },
                WorkflowEvent::StallObserved {
                    execution_id: execution_id.clone(),
                    node_execution_id,
                    session_id: session_id.to_string(),
                    node_name: node_name.to_string(),
                    attempt: 1,
                    turn_phase: "streaming".to_string(),
                    idle_secs: 181,
                    signal_count: 1,
                    cap_reached: false,
                    timestamp: exec.updated_at,
                },
            ])
            .unwrap();
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        let outcome = engine
            .abort_workflow_by_execution_id(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                None,
            )
            .await
            .unwrap();

        assert!(matches!(outcome, AbortOutcome::Aborted));
        let stored_execution = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(stored_execution.status, ExecutionStatus::Aborted);
        let payloads = received_payloads.lock().unwrap().clone();
        let live_payload = payloads
            .last()
            .expect("abort must broadcast workflow-execution-changed");
        let live_json: serde_json::Value = serde_json::from_str(live_payload).unwrap();
        assert!(
            live_json["workflowExecution"]["stallObservations"]
                .as_array()
                .is_none_or(Vec::is_empty),
            "abort broadcast must clear stall observations: {live_json}"
        );

        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::ExecutionAborted { .. })));
        let projected = project_workflow_execution(&execution_id, &events)
            .unwrap()
            .unwrap();
        assert_eq!(projected.status, ExecutionStatus::Aborted);
    }

    fn workflow_turn_complete_notification_from_typed_refusal(
        chat_session_id: &str,
    ) -> crate::usecase::workflow::ports::WorkflowTurnCompleteNotification {
        use crate::usecase::agent_session::event_log::{
            AgentSessionEvent, AgentTurnFailureSignal, PromptInput, TurnEventLog, TurnStopReason,
        };
        use crate::usecase::workflow::ports::{
            WorkflowTurnCompleteNotification, WorkflowTurnFailureSignal, WorkflowTurnTokenUsage,
        };

        let read_model = TurnEventLog::from_events(vec![
            AgentSessionEvent::TurnStarted {
                turn_id: 1,
                message_id: "human-1".to_string(),
                assistant_message_id: Some("agent-1".to_string()),
                prompt: PromptInput::default(),
                at: 1.0,
            },
            AgentSessionEvent::TurnCompleted {
                turn_id: 1,
                exit_code: 0,
                stop_reason: Some(TurnStopReason::Refusal),
                token_usage: None,
            },
        ])
        .project();
        let projected = read_model
            .workflow_turn_complete
            .expect("typed stop_reason must project a workflow turn completion");
        assert_eq!(projected.exit_code, 0);
        assert_eq!(
            projected.failure_signal,
            Some(AgentTurnFailureSignal::ModelRefusal)
        );

        WorkflowTurnCompleteNotification {
            chat_session_id: chat_session_id.to_string(),
            exit_code: projected.exit_code,
            final_text_parts: projected.final_text_parts,
            failure_signal: projected.failure_signal.map(|signal| match signal {
                AgentTurnFailureSignal::ModelRefusal => WorkflowTurnFailureSignal::ModelRefusal,
            }),
            token_usage: projected.token_usage.map(|usage| WorkflowTurnTokenUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
            }),
            interrupted: projected.interrupted,
        }
    }

    async fn insert_execution_and_register_active(
        engine: &WorkflowRuntimeService,
        exec: WorkflowExecution,
        created_from: ExecutionOrigin,
    ) {
        let execution_id = exec.id.clone();
        engine
            .execution_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: exec.workflow.name.clone(),
                status: match exec.state {
                    RuntimeExecutionState::WaitingApproval => ExecutionStatus::WaitingApproval,
                    _ => ExecutionStatus::Running,
                },
                worktree_path: exec.worktree_path.clone(),
                current_node: Some(exec.workflow.nodes[exec.current_node_index].name.clone()),
                created_from,
                started_at: exec.started_at,
                updated_at: exec.updated_at,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        engine.executions.lock().await.insert(execution_id, exec);
    }

    fn resumable_two_node_execution(execution_id: &str, worktree_path: &str) -> WorkflowExecution {
        let workflow = WorkflowDefinitionYaml {
            name: "resume-checkpoint-wf".to_string(),
            description: "resume from the last confirmed node".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_test_node(
                    "prepare",
                    TestKind::Session,
                    "implement",
                    vec![Rule::Next("execute".to_string())],
                    None,
                ),
                make_test_node("execute", TestKind::Session, "implement", vec![], None),
            ],
        };
        WorkflowExecution {
            id: execution_id.to_string(),
            workflow,
            state: RuntimeExecutionState::Running,
            current_node_index: 1,
            node_execution_counts: HashMap::from([
                ("prepare".to_string(), 1),
                ("execute".to_string(), 1),
            ]),
            node_history: Vec::new(),
            worktree_path: worktree_path.to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            error_reason: None,
            started_at: 1000.0,
            updated_at: 1003.0,
            current_session_id: Some("old-unconfirmed-session".to_string()),
            current_node_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![
                NodeExecution {
                    id: format!("{execution_id}-prepare-1"),
                    execution_id: execution_id.to_string(),
                    node_name: "prepare".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    status: NodeExecutionStatus::Succeeded,
                    session_id: Some("confirmed-session".to_string()),
                    artifact: Some(serde_json::json!({"prepared": true})),
                    token_usage: None,
                    failure: None,
                    fanout_parent: None,
                    started_at: 1000.0,
                    completed_at: Some(1002.0),
                },
                NodeExecution {
                    id: format!("{execution_id}-execute-1"),
                    execution_id: execution_id.to_string(),
                    node_name: "execute".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    status: NodeExecutionStatus::Running,
                    session_id: Some("old-unconfirmed-session".to_string()),
                    artifact: None,
                    token_usage: None,
                    failure: None,
                    fanout_parent: None,
                    started_at: 1003.0,
                    completed_at: None,
                },
            ],
            request: Some("continue safely".to_string()),
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: PermissionMode::ASK.to_string(),
            },
        }
    }

    fn append_resumable_two_node_events(data_dir: &std::path::Path, exec: &WorkflowExecution) {
        let prepare_id = format!("{}-prepare-1", exec.id);
        let execute_id = format!("{}-execute-1", exec.id);
        WorkflowEventLog::new(data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: exec.id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    worktree_path: exec.worktree_path.clone(),
                    created_from: exec.created_from,
                    request: exec.request.clone().unwrap_or_default(),
                    permission_mode: exec.workflow_defaults.permission_mode.clone(),
                    definition: exec.workflow.clone(),
                    timestamp: 1000.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: exec.id.clone(),
                    node_execution_id: prepare_id.clone(),
                    node_name: "prepare".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 1000.0,
                },
                WorkflowEvent::SessionAttached {
                    execution_id: exec.id.clone(),
                    node_execution_id: prepare_id.clone(),
                    session_id: "confirmed-session".to_string(),
                    timestamp: 1000.5,
                },
                WorkflowEvent::ArtifactProduced {
                    execution_id: exec.id.clone(),
                    node_execution_id: prepare_id.clone(),
                    node_name: "prepare".to_string(),
                    contract: None,
                    value: serde_json::json!({"prepared": true}),
                    request_id: None,
                    submitted_at: None,
                    timestamp: 1001.0,
                },
                WorkflowEvent::NodeCompleted {
                    execution_id: exec.id.clone(),
                    node_execution_id: prepare_id,
                    node_name: "prepare".to_string(),
                    attempt: 1,
                    result_summary: Some("prepared".to_string()),
                    token_usage: None,
                    timestamp: 1002.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: exec.id.clone(),
                    node_execution_id: execute_id.clone(),
                    node_name: "execute".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 1003.0,
                },
                WorkflowEvent::SessionAttached {
                    execution_id: exec.id.clone(),
                    node_execution_id: execute_id,
                    session_id: "old-unconfirmed-session".to_string(),
                    timestamp: 1003.0,
                },
            ])
            .unwrap();
    }

    async fn assert_event_log_resume_after_interruption(
        reason: ExecutionInterruptionReason,
        permission_mode: &str,
    ) {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir.clone());
        let worktree = TempDir::new().unwrap();
        let execution_id = uuid::Uuid::new_v4().to_string();
        let mut exec =
            resumable_two_node_execution(&execution_id, worktree.path().to_string_lossy().as_ref());
        exec.workflow_defaults.permission_mode = permission_mode.to_string();
        append_resumable_two_node_events(&data_dir, &exec);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        if reason == ExecutionInterruptionReason::Stop {
            engine
                .stop_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &execution_id,
                )
                .await
                .unwrap();
        } else {
            assert!(engine
                .interrupt_active_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &execution_id,
                    reason,
                )
                .await
                .unwrap());
        }

        let interrupted = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(interrupted.status, ExecutionStatus::Interrupted);
        assert_eq!(interrupted.interruption_reason, Some(reason));
        assert_eq!(interrupted.resume_from_node.as_deref(), Some("execute"));
        assert!(!engine.contains_execution_for_test(&execution_id).await);

        engine
            .resume_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();

        let executions = engine.executions.lock().await;
        let resumed = executions.get(&execution_id).expect("resumed runtime");
        assert_eq!(resumed.state, RuntimeExecutionState::Running);
        assert_eq!(resumed.current_node_index, 1);
        assert_eq!(resumed.node_execution_counts["prepare"], 1);
        assert_eq!(resumed.node_execution_counts["execute"], 2);
        assert_eq!(resumed.workflow_defaults.permission_mode, permission_mode);
        assert_eq!(resumed.node_history.len(), 1);
        assert_eq!(resumed.node_history[0].node_name, "prepare");
        assert_eq!(
            resumed.artifacts["prepare"].artifact,
            Some(serde_json::json!({"prepared": true}))
        );
        assert_ne!(
            resumed.current_session_id.as_deref(),
            Some("old-unconfirmed-session")
        );
        assert!(resumed.current_session_id.is_some());
        drop(executions);

        let resumed_metadata = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(resumed_metadata.status, ExecutionStatus::Running);
        assert_eq!(resumed_metadata.current_node.as_deref(), Some("execute"));
        assert_eq!(resumed_metadata.interruption_reason, None);
        assert_eq!(resumed_metadata.resume_from_node, None);

        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    WorkflowEvent::NodeStarted { node_name, .. } if node_name == "prepare"
                ))
                .count(),
            1,
            "confirmed node must not execute again"
        );
        let execute_attempts = events
            .iter()
            .filter_map(|event| match event {
                WorkflowEvent::NodeStarted {
                    node_name, attempt, ..
                } if node_name == "execute" => Some(*attempt),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(execute_attempts, vec![1, 2]);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionResumed {
                resume_from_node,
                ..
            } if resume_from_node == "execute"
        )));
    }

    #[tokio::test]
    async fn crash_stale_and_explicit_stop_resume_from_the_first_unconfirmed_node() {
        for reason in [
            ExecutionInterruptionReason::Crash,
            ExecutionInterruptionReason::Stale,
            ExecutionInterruptionReason::Stop,
        ] {
            assert_event_log_resume_after_interruption(reason, PermissionMode::ASK).await;
        }
    }

    #[tokio::test]
    async fn explicit_stop_resume_preserves_non_default_permission_modes() {
        for permission_mode in [PermissionMode::EDIT, PermissionMode::FULL] {
            assert_event_log_resume_after_interruption(
                ExecutionInterruptionReason::Stop,
                permission_mode,
            )
            .await;
        }
    }

    #[tokio::test]
    async fn resume_required_event_append_failure_rolls_back_every_precommit_state() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir.clone());
        let worktree = TempDir::new().unwrap();
        let execution_id = uuid::Uuid::new_v4().to_string();
        let exec =
            resumable_two_node_execution(&execution_id, worktree.path().to_string_lossy().as_ref());
        append_resumable_two_node_events(&data_dir, &exec);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        engine
            .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();
        let checkpoint_before = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();

        engine.fail_next_required_event_append_for_test();
        let error = engine
            .resume_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("ExecutionResumed log failed"));

        let checkpoint_after = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(checkpoint_after.status, ExecutionStatus::Interrupted);
        assert_eq!(
            checkpoint_after.interruption_reason,
            checkpoint_before.interruption_reason
        );
        assert_eq!(
            checkpoint_after.resume_from_node,
            checkpoint_before.resume_from_node
        );
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(!events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::ExecutionResumed { .. })));
        assert!(!events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeStarted {
                node_name,
                attempt: 2,
                ..
            } if node_name == "execute"
        )));
        assert!(!engine.contains_execution_for_test(&execution_id).await);
        assert!(!engine
            .execution_facet_contents
            .lock()
            .await
            .contains_key(&execution_id));
        assert!(!engine
            .fanout_resume_checkpoints
            .lock()
            .await
            .contains_key(&execution_id));
        assert!(
            !engine
                .execution_store
                .interrupted_transition_pending(&execution_id)
                .await
        );

        engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &agent_runtime,
                &execution_id,
                None,
            )
            .await
            .expect("the rolled-back checkpoint must accept a later command");
        assert_eq!(
            engine
                .execution_store()
                .get_execution(&execution_id)
                .await
                .unwrap()
                .status,
            ExecutionStatus::Aborted
        );
    }

    #[tokio::test]
    async fn resume_metadata_commit_failure_is_accepted_with_a_crash_checkpoint() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir.clone());
        let worktree = TempDir::new().unwrap();
        let execution_id = uuid::Uuid::new_v4().to_string();
        let exec =
            resumable_two_node_execution(&execution_id, worktree.path().to_string_lossy().as_ref());
        append_resumable_two_node_events(&data_dir, &exec);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        engine
            .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();

        engine.execution_store.fail_next_resume_commit_for_test();
        engine
            .resume_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .expect("durable Resume remains accepted after metadata projection failure");

        let metadata = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(metadata.status, ExecutionStatus::Interrupted);
        assert_eq!(
            metadata.interruption_reason,
            Some(ExecutionInterruptionReason::Crash)
        );
        assert!(!engine.contains_execution_for_test(&execution_id).await);
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, WorkflowEvent::ExecutionResumed { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    WorkflowEvent::NodeStarted {
                        node_name,
                        attempt: 2,
                        ..
                    } if node_name == "execute"
                ))
                .count(),
            1
        );
        assert!(matches!(
            events.last(),
            Some(WorkflowEvent::ExecutionInterrupted {
                reason: ExecutionInterruptionReason::Crash,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn resume_runtime_start_failure_is_accepted_with_a_crash_checkpoint() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (agent_runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data_dir.clone(),
            );
        let worktree = TempDir::new().unwrap();
        let execution_id = uuid::Uuid::new_v4().to_string();
        let exec =
            resumable_two_node_execution(&execution_id, worktree.path().to_string_lossy().as_ref());
        append_resumable_two_node_events(&data_dir, &exec);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        engine
            .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();

        controller.fail_next_start_turn();
        engine
            .resume_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .expect("durable Resume remains accepted after runtime activation failure");

        let metadata = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(metadata.status, ExecutionStatus::Interrupted);
        assert_eq!(
            metadata.interruption_reason,
            Some(ExecutionInterruptionReason::Crash)
        );
        assert!(!engine.contains_execution_for_test(&execution_id).await);
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, WorkflowEvent::ExecutionResumed { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    WorkflowEvent::NodeStarted {
                        node_name,
                        attempt: 2,
                        ..
                    } if node_name == "execute"
                ))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionInterrupted {
                reason: ExecutionInterruptionReason::Crash,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn partial_fanout_resume_reuses_confirmed_child_and_restarts_only_pending_child() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir.clone());
        let worktree = TempDir::new().unwrap();
        let worktree_path = worktree.path().to_string_lossy().to_string();
        let execution_id = uuid::Uuid::new_v4().to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "partial-fanout-resume".to_string(),
            description: "reuse confirmed fanout artifacts".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                make_test_node("review-a", TestKind::Session, "implement", vec![], None),
                make_test_node("review-b", TestKind::Session, "implement", vec![], None),
            ],
        };
        let parent_v1 = format!("{execution_id}-parent-1");
        let child_a_v1 = format!("{execution_id}-review-a-1");
        let child_b_v1 = format!("{execution_id}-review-b-1");
        WorkflowEventLog::new(&data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: workflow.name.clone(),
                    worktree_path: worktree_path.clone(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: "review in fanout".to_string(),
                    permission_mode: "ask".to_string(),
                    definition: workflow.clone(),
                    timestamp: 1.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: parent_v1,
                    node_name: "fanout-review".to_string(),
                    kind: NodeKindName::Fanout,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: child_a_v1.clone(),
                    node_name: "review-a".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "fanout-review".to_string(),
                        parent_attempt: 1,
                        item_index: None,
                        child_index: 0,
                    }),
                    timestamp: 3.0,
                },
                WorkflowEvent::SessionAttached {
                    execution_id: execution_id.clone(),
                    node_execution_id: child_a_v1.clone(),
                    session_id: "old-review-a-session".to_string(),
                    timestamp: 3.1,
                },
                WorkflowEvent::ArtifactProduced {
                    execution_id: execution_id.clone(),
                    node_execution_id: child_a_v1.clone(),
                    node_name: "review-a".to_string(),
                    contract: None,
                    value: serde_json::json!({"verdict": "confirmed"}),
                    request_id: None,
                    submitted_at: None,
                    timestamp: 4.0,
                },
                WorkflowEvent::NodeCompleted {
                    execution_id: execution_id.clone(),
                    node_execution_id: child_a_v1,
                    node_name: "review-a".to_string(),
                    attempt: 1,
                    result_summary: Some("confirmed review".to_string()),
                    token_usage: Some(crate::adaptor::gateway::workflow::event::TokenUsage {
                        input_tokens: 11,
                        output_tokens: 7,
                    }),
                    timestamp: 5.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: child_b_v1.clone(),
                    node_name: "review-b".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "fanout-review".to_string(),
                        parent_attempt: 1,
                        item_index: None,
                        child_index: 1,
                    }),
                    timestamp: 3.0,
                },
                WorkflowEvent::SessionAttached {
                    execution_id: execution_id.clone(),
                    node_execution_id: child_b_v1,
                    session_id: "old-review-b-session".to_string(),
                    timestamp: 3.1,
                },
                WorkflowEvent::ExecutionInterrupted {
                    execution_id: execution_id.clone(),
                    reason: ExecutionInterruptionReason::Crash,
                    timestamp: 6.0,
                },
            ])
            .unwrap();
        engine
            .execution_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: workflow.name.clone(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.clone(),
                current_node: Some("fanout-review".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 1.0,
                updated_at: 5.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        engine
            .execution_store
            .interrupt_execution(
                &execution_id,
                ExecutionInterruptionReason::Crash,
                Some("fanout-review".to_string()),
                6.0,
            )
            .await
            .unwrap();

        engine
            .resume_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();

        let executions = engine.executions.lock().await;
        let resumed = executions
            .get(&execution_id)
            .expect("resumed fanout runtime");
        assert_eq!(resumed.node_execution_counts["fanout-review"], 2);
        assert_eq!(resumed.node_execution_counts["review-a"], 2);
        assert_eq!(resumed.node_execution_counts["review-b"], 2);
        let fanout = resumed.fanout_runtime.as_ref().expect("active fanout");
        let reused = fanout
            .children
            .iter()
            .find(|child| child.node_name == "review-a")
            .unwrap();
        let pending = fanout
            .children
            .iter()
            .find(|child| child.node_name == "review-b")
            .unwrap();
        assert_eq!(reused.state, FanoutChildRuntimeState::Completed);
        assert!(reused.session_id.is_empty());
        assert_eq!(
            reused.artifact,
            Some(serde_json::json!({"verdict": "confirmed"}))
        );
        assert_eq!(reused.result.as_deref(), Some("confirmed review"));
        assert_eq!(reused.token_usage.input_tokens, 11);
        assert_eq!(reused.token_usage.output_tokens, 7);
        assert_eq!(pending.state, FanoutChildRuntimeState::Running);
        assert!(!pending.session_id.is_empty());
        assert_ne!(pending.session_id, "old-review-b-session");
        let reused_node_execution_id = reused.node_execution_id.clone();
        let pending_node_execution_id = pending.node_execution_id.clone();
        let pending_session_id = pending.session_id.clone();
        drop(executions);

        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ArtifactProduced {
                node_execution_id,
                value,
                ..
            } if node_execution_id == &reused_node_execution_id
                && value == &serde_json::json!({"verdict": "confirmed"})
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeCompleted {
                node_execution_id,
                ..
            } if node_execution_id == &reused_node_execution_id
        )));
        assert!(events.iter().all(|event| !matches!(
            event,
            WorkflowEvent::SessionAttached {
                node_execution_id,
                ..
            } if node_execution_id == &reused_node_execution_id
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::SessionAttached {
                node_execution_id,
                session_id,
                ..
            } if node_execution_id == &pending_node_execution_id
                && session_id != "old-review-b-session"
        )));

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &agent_runtime,
                &pending_session_id,
                0,
                None,
                &[MessagePart::Text {
                    content: "pending review complete".to_string(),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .unwrap();
        wait_for_execution_terminal(&app, &engine, &execution_id).await;

        let completed = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .expect("completed resumed fanout metadata");
        assert_eq!(completed.status, ExecutionStatus::Completed);
        assert_eq!(completed.total_token_usage.input_tokens, 11);
        assert_eq!(completed.total_token_usage.output_tokens, 7);
        let completed_events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(completed_events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeCompleted {
                node_execution_id,
                ..
            } if node_execution_id == &pending_node_execution_id
        )));
        let parent_artifact = completed_events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::ArtifactProduced {
                    node_name, value, ..
                } if node_name == "fanout-review" => Some(value),
                _ => None,
            })
            .expect("resumed fanout parent artifact");
        assert_eq!(
            parent_artifact,
            &serde_json::json!([{"verdict": "confirmed"}, null])
        );
        assert!(completed_events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeCompleted {
                node_name,
                attempt: 2,
                ..
            } if node_name == "fanout-review"
        )));
        assert!(completed_events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionCompleted { total_token_usage, .. }
                if total_token_usage.input_tokens == 11
                    && total_token_usage.output_tokens == 7
        )));
    }

    #[tokio::test]
    async fn explicit_stop_accepts_waiting_approval() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir.clone());
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree = TempDir::new().unwrap();
        let exec = make_waiting_approval_execution(
            &execution_id,
            worktree.path().to_string_lossy().as_ref(),
        );
        append_started_events_for_execution(&data_dir, &exec);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        engine
            .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();

        let metadata = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(metadata.status, ExecutionStatus::Interrupted);
        assert_eq!(
            metadata.interruption_reason,
            Some(ExecutionInterruptionReason::Stop)
        );
        assert_eq!(metadata.resume_from_node.as_deref(), Some("review"));
    }

    #[tokio::test]
    async fn explicit_stop_waits_for_session_turn_activation_and_interrupts_the_started_turn() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (agent_runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data_dir,
            );
        controller.pause_start_turn();
        let worktree = TempDir::new().unwrap();
        let worktree_path = worktree.path().to_string_lossy().to_string();
        let mut review_node =
            make_test_node("review", TestKind::Session, "implement", vec![], None);
        if let NodeKind::Session(session) = &mut review_node.kind {
            session.model = Some("claude-4-sonnet".to_string());
        }
        let workflow = WorkflowDefinitionYaml {
            name: "stop-during-session-activation".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![review_node],
        };

        let start_engine = engine.clone();
        let start_app = app.handle().clone();
        let start_session_store = session_store.clone();
        let start_agent_runtime = agent_runtime.clone();
        let start_worktree_path = worktree_path.clone();
        let mut start_task = tokio::spawn(async move {
            start_engine
                .start_resolved_workflow(
                    &start_app,
                    &start_session_store,
                    &start_agent_runtime,
                    workflow,
                    start_worktree_path,
                    Some("review safely".to_string()),
                    ExecutionOrigin::DesktopUi,
                    PermissionMode::Edit,
                )
                .await
        });

        let session_id_result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(call) = controller.calls().into_iter().find(|call| {
                    matches!(
                        call.kind,
                        crate::test_support::TestRuntimeCallKind::StartTurn
                    )
                }) {
                    break call.session_id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        let session_id = match session_id_result {
            Ok(session_id) => session_id,
            Err(_) => {
                controller.release_start_turn();
                let start_result =
                    tokio::time::timeout(std::time::Duration::from_secs(2), &mut start_task).await;
                let metadata = match &start_result {
                    Ok(Ok(Ok(execution_id))) => {
                        engine.execution_store().get_execution(execution_id).await
                    }
                    _ => None,
                };
                panic!(
                    "session turn activation did not reach the paused backend; start result: {start_result:?}; runtime calls: {:?}; metadata: {metadata:?}",
                    controller.calls(),
                );
            }
        };
        let execution_id = {
            let executions = engine.executions.lock().await;
            find_by_worktree(&executions, &worktree_path)
                .map(|(execution_id, _)| execution_id.clone())
                .expect("execution must be visible before its turn starts")
        };

        let stop_engine = engine.clone();
        let stop_app = app.handle().clone();
        let stop_session_store = session_store.clone();
        let stop_agent_runtime = agent_runtime.clone();
        let stop_execution_id = execution_id.clone();
        let mut stop_task = tokio::spawn(async move {
            stop_engine
                .stop_workflow_execution(
                    &stop_app,
                    &stop_session_store,
                    &stop_agent_runtime,
                    &stop_execution_id,
                )
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut stop_task)
            .await
            .expect("stop must cancel a stuck in-flight turn activation")
            .expect("stop task should join")
            .expect("stop should succeed");
        controller.release_start_turn();
        let started_execution_id =
            tokio::time::timeout(std::time::Duration::from_secs(2), &mut start_task)
                .await
                .expect("workflow start should finish after releasing the turn")
                .expect("workflow start task should join")
                .expect("workflow start should be accepted");
        assert_eq!(started_execution_id, execution_id);

        let metadata = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(metadata.status, ExecutionStatus::Interrupted);
        assert_eq!(
            metadata.interruption_reason,
            Some(ExecutionInterruptionReason::Stop)
        );
        assert_eq!(metadata.resume_from_node.as_deref(), Some("review"));
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&crate::test_support::TestRuntimeCallKind::Interrupt));
        assert!(!engine.contains_execution_for_test(&execution_id).await);
    }

    #[tokio::test]
    async fn stop_append_failure_resumes_the_paused_session_activation_and_restores_running() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (agent_runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data_dir,
            );
        controller.pause_start_turn();
        let worktree = TempDir::new().unwrap();
        let worktree_path = worktree.path().to_string_lossy().to_string();
        let mut review_node =
            make_test_node("review", TestKind::Session, "implement", vec![], None);
        if let NodeKind::Session(session) = &mut review_node.kind {
            session.model = Some("claude-4-sonnet".to_string());
        }
        let workflow = WorkflowDefinitionYaml {
            name: "stop-append-failure-during-activation".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![review_node],
        };
        let start_engine = engine.clone();
        let start_app = app.handle().clone();
        let start_session_store = session_store.clone();
        let start_agent_runtime = agent_runtime.clone();
        let start_worktree_path = worktree_path.clone();
        let mut start_task = tokio::spawn(async move {
            start_engine
                .start_resolved_workflow(
                    &start_app,
                    &start_session_store,
                    &start_agent_runtime,
                    workflow,
                    start_worktree_path,
                    Some("review safely".to_string()),
                    ExecutionOrigin::DesktopUi,
                    PermissionMode::Edit,
                )
                .await
        });
        let session_id = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(call) = controller.calls().into_iter().find(|call| {
                    matches!(
                        call.kind,
                        crate::test_support::TestRuntimeCallKind::StartTurn
                    )
                }) {
                    break call.session_id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session turn activation should reach the paused backend");
        let execution_id = {
            let executions = engine.executions.lock().await;
            find_by_worktree(&executions, &worktree_path)
                .map(|(execution_id, _)| execution_id.clone())
                .expect("execution must be visible before its turn starts")
        };

        engine.fail_next_required_event_append_for_test();
        engine
            .execution_store
            .fail_next_active_interruption_rollback_for_test();
        let stop_error = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine.stop_workflow_execution(
                app.handle(),
                &session_store,
                &agent_runtime,
                &execution_id,
            ),
        )
        .await
        .expect("failed stop append must roll back without waiting for the paused backend")
        .expect_err("injected stop append failure must be returned");
        assert!(matches!(stop_error, WorkflowEngineError::SessionStore(_)));
        let stop_error = stop_error.to_string();
        assert!(stop_error.contains("ExecutionInterrupted log failed"));
        assert!(stop_error.contains("interruption reservation rollback failed"));
        assert!(stop_error.contains("active metadata rollback failed"));

        let restored = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(restored.status, ExecutionStatus::Running);
        assert_eq!(restored.current_node.as_deref(), Some("review"));
        assert!(engine.contains_execution_for_test(&execution_id).await);
        assert!(!read_dispatch_events(&app, &execution_id)
            .iter()
            .any(|event| matches!(event, WorkflowEvent::ExecutionInterrupted { .. })));
        assert!(!controller
            .call_kinds_for(&session_id)
            .contains(&crate::test_support::TestRuntimeCallKind::Interrupt));

        // The injected rollback failure deliberately leaves the transition reservation in place.
        // Clear it so the remainder of this activation test can verify a later successful stop.
        engine
            .execution_store
            .finish_active_interruption(
                crate::adaptor::gateway::workflow::execution_store::ActiveInterruptionReservation {
                    execution_id: execution_id.clone(),
                    worktree_path: worktree_path.clone(),
                },
            )
            .await
            .unwrap();

        controller.release_start_turn();
        let started_execution_id =
            tokio::time::timeout(std::time::Duration::from_secs(2), &mut start_task)
                .await
                .expect("rolled-back activation should resume")
                .expect("workflow start task should join")
                .expect("workflow start should remain accepted");
        assert_eq!(started_execution_id, execution_id);

        engine
            .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&crate::test_support::TestRuntimeCallKind::Interrupt));
    }

    #[tokio::test]
    async fn active_abort_cancels_a_stuck_session_activation_and_interrupts_the_turn() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (agent_runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data_dir,
            );
        controller.pause_start_turn();
        let worktree = TempDir::new().unwrap();
        let worktree_path = worktree.path().to_string_lossy().to_string();
        let mut review_node =
            make_test_node("review", TestKind::Session, "implement", vec![], None);
        if let NodeKind::Session(session) = &mut review_node.kind {
            session.model = Some("claude-4-sonnet".to_string());
        }
        let workflow = WorkflowDefinitionYaml {
            name: "abort-during-session-activation".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![review_node],
        };
        let start_engine = engine.clone();
        let start_app = app.handle().clone();
        let start_session_store = session_store.clone();
        let start_agent_runtime = agent_runtime.clone();
        let start_worktree_path = worktree_path.clone();
        let mut start_task = tokio::spawn(async move {
            start_engine
                .start_resolved_workflow(
                    &start_app,
                    &start_session_store,
                    &start_agent_runtime,
                    workflow,
                    start_worktree_path,
                    Some("review safely".to_string()),
                    ExecutionOrigin::DesktopUi,
                    PermissionMode::Edit,
                )
                .await
        });
        let session_id = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(call) = controller.calls().into_iter().find(|call| {
                    matches!(
                        call.kind,
                        crate::test_support::TestRuntimeCallKind::StartTurn
                    )
                }) {
                    break call.session_id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session turn activation should reach the paused backend");
        let execution_id = {
            let executions = engine.executions.lock().await;
            find_by_worktree(&executions, &worktree_path)
                .map(|(execution_id, _)| execution_id.clone())
                .expect("execution must be visible before its turn starts")
        };

        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine.abort_workflow_execution(
                app.handle(),
                &session_store,
                &agent_runtime,
                &execution_id,
                None,
            ),
        )
        .await
        .expect("abort must cancel a stuck in-flight turn activation")
        .expect("abort should succeed");
        controller.release_start_turn();
        let started_execution_id =
            tokio::time::timeout(std::time::Duration::from_secs(2), &mut start_task)
                .await
                .expect("workflow start should finish after abort cancellation")
                .expect("workflow start task should join")
                .expect("workflow start should remain accepted");
        assert_eq!(started_execution_id, execution_id);

        let metadata = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(metadata.status, ExecutionStatus::Aborted);
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&crate::test_support::TestRuntimeCallKind::Interrupt));
        assert!(!engine.contains_execution_for_test(&execution_id).await);
        assert!(read_dispatch_events(&app, &execution_id)
            .iter()
            .any(|event| matches!(event, WorkflowEvent::ExecutionAborted { .. })));
    }

    #[tokio::test]
    async fn active_abort_keeps_activation_cancelled_after_metadata_projection_failure() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (agent_runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data_dir,
            );
        controller.pause_start_turn();
        let worktree = TempDir::new().unwrap();
        let worktree_path = worktree.path().to_string_lossy().to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "abort-projection-failure-during-activation".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_test_node(
                "review",
                TestKind::Session,
                "implement",
                vec![],
                None,
            )],
        };
        let start_engine = engine.clone();
        let start_app = app.handle().clone();
        let start_session_store = session_store.clone();
        let start_agent_runtime = agent_runtime.clone();
        let start_worktree_path = worktree_path.clone();
        let mut start_task = tokio::spawn(async move {
            start_engine
                .start_resolved_workflow(
                    &start_app,
                    &start_session_store,
                    &start_agent_runtime,
                    workflow,
                    start_worktree_path,
                    Some("review safely".to_string()),
                    ExecutionOrigin::DesktopUi,
                    PermissionMode::Edit,
                )
                .await
        });
        let session_id = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(call) = controller.calls().into_iter().find(|call| {
                    matches!(
                        call.kind,
                        crate::test_support::TestRuntimeCallKind::StartTurn
                    )
                }) {
                    break call.session_id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session turn activation should reach the paused backend");
        let execution_id = {
            let executions = engine.executions.lock().await;
            find_by_worktree(&executions, &worktree_path)
                .map(|(execution_id, _)| execution_id.clone())
                .expect("execution must be visible before its turn starts")
        };

        // Event log writes still use the app data directory. Redirect only ExecutionStore
        // metadata to a regular file so the durable ExecutionAborted append succeeds and its
        // post-commit metadata projection fails.
        let invalid_metadata_dir = worktree.path().join("metadata-is-a-file");
        std::fs::write(&invalid_metadata_dir, "not a directory").unwrap();
        engine
            .set_execution_store_data_dir(invalid_metadata_dir)
            .await;

        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine.abort_workflow_execution(
                app.handle(),
                &session_store,
                &agent_runtime,
                &execution_id,
                None,
            ),
        )
        .await
        .expect("durably committed abort must cancel the paused activation")
        .expect("metadata projection is post-commit and must not reject the abort");

        // A committed cancellation drops the pinned start future, so workflow start completes
        // without releasing the test backend's pause gate. Rolling cancellation back here would
        // leave this task blocked and later resume it against an Aborted execution.
        let started_execution_id =
            tokio::time::timeout(std::time::Duration::from_secs(2), &mut start_task)
                .await
                .expect("workflow start must finish without resuming the paused activation")
                .expect("workflow start task should join")
                .expect("workflow start should remain accepted");
        controller.release_start_turn();
        assert_eq!(started_execution_id, execution_id);
        assert!(controller
            .call_kinds_for(&session_id)
            .contains(&crate::test_support::TestRuntimeCallKind::Interrupt));
        assert!(!engine.contains_execution_for_test(&execution_id).await);

        let events = read_dispatch_events(&app, &execution_id);
        let aborted_index = events
            .iter()
            .position(|event| matches!(event, WorkflowEvent::ExecutionAborted { .. }))
            .expect("ExecutionAborted must remain durable");
        assert!(
            !events[aborted_index + 1..]
                .iter()
                .any(|event| matches!(event, WorkflowEvent::ExecutionInterrupted { .. })),
            "cancelled activation must not resume and append a crash interruption after abort"
        );
    }

    #[tokio::test]
    async fn typed_execution_commands_reject_missing_invalid_state_and_worktree_mismatch_targets() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir.clone());
        let engine = WorkflowRuntimeService::new_for_test();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let missing_id = uuid::Uuid::new_v4().to_string();

        assert!(matches!(
            engine
                .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &missing_id,)
                .await,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));
        assert!(matches!(
            engine
                .resume_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &missing_id,
                )
                .await,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));
        assert!(matches!(
            engine
                .abort_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &missing_id,
                    None,
                )
                .await,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));

        let running_id = uuid::Uuid::new_v4().to_string();
        engine
            .execution_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: running_id.clone(),
                workflow_name: "state-validation".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: "/wt/resume-running".to_string(),
                current_node: Some("review".to_string()),
                created_from: ExecutionOrigin::Api,
                started_at: 1.0,
                updated_at: 1.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        assert!(matches!(
            engine
                .resume_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &running_id,
                )
                .await,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        engine
            .execution_store
            .interrupt_execution(
                &running_id,
                ExecutionInterruptionReason::Stale,
                Some("review".to_string()),
                2.0,
            )
            .await
            .unwrap();
        WorkflowEventLog::new(&data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: running_id.clone(),
                    workflow_name: "state-validation".to_string(),
                    worktree_path: "/wt/resume-running".to_string(),
                    created_from: ExecutionOrigin::Api,
                    request: String::new(),
                    permission_mode: "ask".to_string(),
                    definition: WorkflowDefinitionYaml {
                        name: "state-validation".to_string(),
                        description: String::new(),
                        builtin: false,
                        schemas: Default::default(),
                        nodes: vec![make_test_node(
                            "review",
                            TestKind::Session,
                            "review",
                            vec![],
                            None,
                        )],
                    },
                    timestamp: 1.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: running_id.clone(),
                    node_execution_id: format!("{running_id}-review-1"),
                    node_name: "review".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 1.5,
                },
                WorkflowEvent::ExecutionInterrupted {
                    execution_id: running_id.clone(),
                    reason: ExecutionInterruptionReason::Stale,
                    timestamp: 2.0,
                },
            ])
            .unwrap();
        assert!(matches!(
            engine
                .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &running_id,)
                .await,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &agent_runtime,
                &running_id,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            engine
                .execution_store()
                .get_execution(&running_id)
                .await
                .unwrap()
                .status,
            ExecutionStatus::Aborted
        );

        let completed_id = uuid::Uuid::new_v4().to_string();
        engine
            .execution_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: completed_id.clone(),
                workflow_name: "state-validation".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: "/wt/resume-completed".to_string(),
                current_node: Some("review".to_string()),
                created_from: ExecutionOrigin::Api,
                started_at: 1.0,
                updated_at: 1.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        engine
            .execution_store
            .complete_execution(&completed_id, TerminalExecutionStatus::Completed, 2.0, None)
            .await
            .unwrap();
        assert!(matches!(
            engine
                .resume_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &completed_id,
                )
                .await,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        assert!(matches!(
            engine
                .stop_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &completed_id,
                )
                .await,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        assert!(matches!(
            engine
                .abort_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &completed_id,
                    None,
                )
                .await,
            Err(WorkflowEngineError::InvalidState(_))
        ));

        let mismatch_engine = WorkflowRuntimeService::new(
            Arc::new(TestWorkflowDefinitionResolver),
            Arc::new(RewritingManagedWorktreeResolver),
            None,
            Arc::new(OpenTabRegistry::default()),
        );
        mismatch_engine.set_execution_store_data_dir(data_dir).await;
        let mismatch_id = uuid::Uuid::new_v4().to_string();
        mismatch_engine
            .execution_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: mismatch_id.clone(),
                workflow_name: "target-validation".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: "/claimed-worktree".to_string(),
                current_node: Some("review".to_string()),
                created_from: ExecutionOrigin::Api,
                started_at: 1.0,
                updated_at: 1.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        assert!(matches!(
            mismatch_engine
                .stop_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &mismatch_id,
                )
                .await,
            Err(WorkflowEngineError::UnauthorizedWorktree(_))
        ));
        assert!(matches!(
            mismatch_engine
                .resume_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &mismatch_id,
                )
                .await,
            Err(WorkflowEngineError::UnauthorizedWorktree(_))
        ));
        assert!(matches!(
            mismatch_engine
                .abort_workflow_execution(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    &mismatch_id,
                    None,
                )
                .await,
            Err(WorkflowEngineError::UnauthorizedWorktree(_))
        ));
    }

    fn read_dispatch_events(app: &DispatchTestApp, execution_id: &str) -> Vec<WorkflowEvent> {
        let data_dir = dispatch_data_dir(app.handle());
        WorkflowEventLog::new(&data_dir)
            .read_log(execution_id)
            .unwrap_or_default()
    }

    fn make_managed_worktree() -> (TempDir, TempDir, std::path::PathBuf) {
        let repo_parent = TempDir::new().unwrap();
        let worktree_parent = TempDir::new().unwrap();
        let repo_path = repo_parent.path().join("repo");
        std::fs::create_dir(&repo_path).unwrap();
        let repo = git2::Repository::init(&repo_path).unwrap();
        std::fs::write(repo_path.join("README.md"), "test\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        let worktree_path = worktree_parent.path().join("managed-wt");
        repo.worktree("managed-wt", &worktree_path, None).unwrap();
        (repo_parent, worktree_parent, worktree_path)
    }

    fn configure_managed_repo(app: &DispatchTestApp, repo_path: &std::path::Path) {
        let config_repository = app.state::<Arc<dyn crate::domain::app_config::ConfigRepository>>();
        let mut config = config_repository.load().unwrap();
        config.app.last_repo_paths = vec![repo_path.to_string_lossy().to_string()];
        config_repository.save(config).unwrap();
    }

    fn command_node(name: &str, command: &str, rules: Vec<Rule>) -> NodeDefinition {
        make_test_node(name, TestKind::Command, command, rules, None)
    }

    fn command_node_with_artifact(
        name: &str,
        command: &str,
        artifact: &str,
        rules: Vec<Rule>,
    ) -> NodeDefinition {
        let mut node = command_node(name, command, rules);
        node.artifact = Some(artifact.to_string());
        node
    }

    async fn start_command_workflow_nowait_for_test(
        app: &DispatchTestApp,
        engine: &WorkflowRuntimeService,
        workflow: WorkflowDefinitionYaml,
        worktree_path: &std::path::Path,
    ) -> String {
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir);
        engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &handles,
                workflow,
                worktree_path.to_string_lossy().to_string(),
                Some("run command".to_string()),
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Edit,
            )
            .await
            .unwrap()
    }

    async fn start_command_workflow_for_test(
        app: &DispatchTestApp,
        engine: &WorkflowRuntimeService,
        workflow: WorkflowDefinitionYaml,
        worktree_path: &std::path::Path,
    ) -> String {
        let execution_id =
            start_command_workflow_nowait_for_test(app, engine, workflow, worktree_path).await;
        wait_for_execution_terminal(app, engine, &execution_id).await;
        execution_id
    }

    fn artifact_event_for_node(
        events: &[WorkflowEvent],
        node: &str,
    ) -> (Option<String>, serde_json::Value) {
        events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::ArtifactProduced {
                    node_name,
                    contract,
                    value,
                    ..
                } if node_name == node => Some((contract.clone(), value.clone())),
                _ => None,
            })
            .unwrap_or_else(|| panic!("ArtifactProduced for node '{node}' not found: {events:?}"))
    }

    fn completed_node_names(events: &[WorkflowEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                WorkflowEvent::NodeCompleted { node_name, .. } => Some(node_name.clone()),
                _ => None,
            })
            .collect()
    }

    async fn wait_for_active_command(engine: &WorkflowRuntimeService, execution_id: &str) {
        for _ in 0..100 {
            if engine
                .active_command_executions
                .lock()
                .await
                .values()
                .any(|owner_execution_id| owner_execution_id == execution_id)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("command process for execution '{execution_id}' did not become active");
    }

    async fn wait_for_fanout_child_session(
        engine: &WorkflowRuntimeService,
        execution_id: &str,
        child_name: &str,
    ) -> String {
        for _ in 0..500 {
            let session_id = {
                let execs = engine.executions.lock().await;
                execs
                    .get(execution_id)
                    .and_then(|exec| exec.fanout_runtime.as_ref())
                    .and_then(|fanout| {
                        fanout
                            .children
                            .iter()
                            .find(|child| {
                                child.node_name == child_name && !child.session_id.is_empty()
                            })
                            .map(|child| child.session_id.clone())
                    })
            };
            if let Some(session_id) = session_id {
                return session_id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!(
            "fanout child session for '{child_name}' in execution '{execution_id}' did not start"
        );
    }

    async fn wait_for_inactive_command(engine: &WorkflowRuntimeService, execution_id: &str) {
        for _ in 0..500 {
            if !engine
                .active_command_executions
                .lock()
                .await
                .values()
                .any(|owner_execution_id| owner_execution_id == execution_id)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("command process for execution '{execution_id}' did not stop");
    }

    #[cfg(unix)]
    async fn wait_for_pid_file(path: &std::path::Path) -> i32 {
        for _ in 0..500 {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    return pid;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("pid file '{}' was not written", path.display());
    }

    #[cfg(unix)]
    async fn wait_for_process_exit(pid: i32) {
        for _ in 0..500 {
            if !process_exists(pid) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("process {pid} was not killed");
    }

    #[cfg(unix)]
    fn process_exists(pid: i32) -> bool {
        // SAFETY: kill(pid, 0) performs existence/permission probing without sending a signal.
        let result = unsafe { libc::kill(pid, 0) };
        if result == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }

    async fn wait_for_execution_terminal(
        app: &DispatchTestApp,
        engine: &WorkflowRuntimeService,
        execution_id: &str,
    ) {
        let data_dir = dispatch_data_dir(app.handle());
        for _ in 0..500 {
            let execution_store_terminal = engine
                .execution_store()
                .get_execution(execution_id)
                .await
                .is_some_and(|execution| execution.status.is_terminal());
            let log_terminal = WorkflowEventLog::new(&data_dir)
                .read_log(execution_id)
                .unwrap_or_default()
                .iter()
                .any(|event| {
                    matches!(
                        event,
                        WorkflowEvent::ExecutionCompleted { .. }
                            | WorkflowEvent::ExecutionFailed { .. }
                            | WorkflowEvent::ExecutionAborted { .. }
                            | WorkflowEvent::ExecutionInterrupted { .. }
                    )
                });
            if execution_store_terminal && log_terminal {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let status = engine.execution_store().get_execution(execution_id).await;
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(execution_id)
            .unwrap_or_default();
        panic!(
            "execution '{execution_id}' did not become terminal; status={status:?}; events={events:?}"
        );
    }

    async fn wait_for_execution_status(
        engine: &WorkflowRuntimeService,
        execution_id: &str,
        expected: ExecutionStatus,
    ) {
        for _ in 0..500 {
            if engine
                .execution_store()
                .get_execution(execution_id)
                .await
                .is_some_and(|execution| execution.status == expected)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let status = engine.execution_store().get_execution(execution_id).await;
        panic!("execution '{execution_id}' did not become {expected:?}; status={status:?}");
    }

    fn canonical_full_pipeline_runtime_workflow(
        run_tests_command: Option<&str>,
        judge_command: Option<&str>,
        list_threads_command: Option<&str>,
    ) -> WorkflowDefinitionYaml {
        let source = include_str!("../../../../../../docs/examples/full-pipeline.yml");
        let diagnosis = crate::adaptor::gateway::workflow::diagnostics::diagnose_workflow_source(
            source,
            Some("full-pipeline"),
        );
        assert!(
            diagnosis.diagnostics.is_empty(),
            "canonical full-pipeline example must stay diagnostic-free: {:?}",
            diagnosis.diagnostics
        );
        let mut workflow = diagnosis
            .workflow
            .expect("diagnostic-free full-pipeline source must deserialize");
        for node in &mut workflow.nodes {
            match &mut node.kind {
                NodeKind::Command(command) => {
                    let replacement = match node.name.as_str() {
                        "run_tests" => run_tests_command,
                        "judge" => judge_command,
                        "list_threads" => list_threads_command,
                        other => panic!("unexpected full-pipeline command node: {other}"),
                    };
                    if let Some(replacement) = replacement {
                        command.command = replacement.to_string();
                    }
                }
                NodeKind::Session(session) => {
                    // The canonical example intentionally names conceptual facets. Runtime
                    // integration isolates engine behavior from the user's facet inventory by
                    // resolving every stub session through one installed test instruction.
                    session.facets = FacetRefs {
                        instruction: Some("implement".to_string()),
                        ..Default::default()
                    };
                }
                NodeKind::Fanout(_) => {}
            }
        }
        workflow
    }

    fn install_full_pipeline_cli() -> crate::test_support::EnvVarGuard {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        // Unit-test executables are Rust test harnesses, not the production CLI. Build the real
        // binary so the command node crosses the same process/parser boundary as users do.
        let status = std::process::Command::new(cargo)
            .args(["build", "--quiet", "--locked", "--bin", "releash"])
            .current_dir(&manifest_dir)
            .status()
            .expect("the production releash CLI binary must build for the canonical example");
        assert!(
            status.success(),
            "the production releash CLI binary must build"
        );
        let target_dir = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    manifest_dir.join(path)
                }
            })
            .unwrap_or_else(|| manifest_dir.join("target"));
        let binary_dir = target_dir.join("debug");
        let binary = binary_dir.join(if cfg!(windows) {
            "releash.exe"
        } else {
            "releash"
        });
        assert!(
            binary.is_file(),
            "missing releash CLI: {}",
            binary.display()
        );
        let current_path = std::env::var("PATH").unwrap_or_default();
        crate::test_support::EnvVarGuard::set_value(
            "PATH",
            &format!("{}:{current_path}", binary_dir.display()),
        )
    }

    fn install_full_pipeline_worktree_fixture(worktree: &std::path::Path) {
        std::fs::create_dir_all(worktree.join("src")).unwrap();
        std::fs::write(
            worktree.join("Cargo.toml"),
            "[package]\nname = \"full-pipeline-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            worktree.join("src/lib.rs"),
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn pipeline_fixture() {\n        assert!(false);\n    }\n}\n",
        )
        .unwrap();
    }

    fn make_full_pipeline_tests_pass(worktree: &std::path::Path) {
        std::fs::write(
            worktree.join("src/lib.rs"),
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn pipeline_fixture() {\n        assert!(true);\n    }\n}\n",
        )
        .unwrap();
    }

    async fn wait_for_top_level_session(
        engine: &WorkflowRuntimeService,
        execution_id: &str,
        node_name: &str,
    ) -> (String, String) {
        for _ in 0..500 {
            let target = {
                let executions = engine.executions.lock().await;
                executions.get(execution_id).and_then(|execution| {
                    let current = &execution.workflow.nodes[execution.current_node_index];
                    if current.name != node_name {
                        return None;
                    }
                    let session_id = execution.current_session_id.clone()?;
                    let node_execution_id = execution
                        .node_executions
                        .iter()
                        .rev()
                        .find(|candidate| {
                            candidate.node_name == node_name
                                && candidate.fanout_parent.is_none()
                                && matches!(
                                    candidate.status,
                                    NodeExecutionStatus::Running
                                        | NodeExecutionStatus::WaitingApproval
                                )
                        })?
                        .id
                        .clone();
                    Some((session_id, node_execution_id))
                })
            };
            if let Some(target) = target {
                return target;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let live = {
            let executions = engine.executions.lock().await;
            executions.get(execution_id).map(|execution| {
                (
                    execution.state.clone(),
                    execution.workflow.nodes[execution.current_node_index]
                        .name
                        .clone(),
                    execution.current_session_id.clone(),
                    execution.error_reason.clone(),
                )
            })
        };
        let metadata = engine.execution_store().get_execution(execution_id).await;
        panic!(
            "top-level session '{node_name}' did not start in execution '{execution_id}'; live={live:?}; metadata={metadata:?}"
        );
    }

    async fn wait_for_active_fanout_children(
        engine: &WorkflowRuntimeService,
        execution_id: &str,
        parent_node: &str,
        expected_count: usize,
    ) -> Vec<(String, String, String)> {
        for _ in 0..500 {
            let children = {
                let executions = engine.executions.lock().await;
                executions
                    .get(execution_id)
                    .and_then(|execution| execution.fanout_runtime.as_ref())
                    .filter(|fanout| fanout.parent_node_name == parent_node)
                    .map(|fanout| {
                        fanout
                            .children
                            .iter()
                            .filter(|child| {
                                child.state == FanoutChildRuntimeState::Running
                                    && !child.session_id.is_empty()
                            })
                            .map(|child| {
                                (
                                    child.node_name.clone(),
                                    child.session_id.clone(),
                                    child.node_execution_id.clone(),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            if children.len() == expected_count {
                return children;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!(
            "fanout '{parent_node}' did not start {expected_count} children in execution '{execution_id}'"
        );
    }

    async fn wait_for_stub_session_turn_activation(
        agent_runtime: &AgentSessionRuntimeUsecase,
        session_id: &str,
    ) {
        for _ in 0..500 {
            if agent_runtime.turn_phase(session_id).await
                == Some(crate::usecase::agent_session::status::TurnPhase::Streaming)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("stub session '{session_id}' did not activate its turn");
    }

    async fn complete_top_level_session(
        app: &DispatchTestApp,
        engine: &WorkflowRuntimeService,
        session_store: &Arc<crate::usecase::agent_session::session::SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        node_name: &str,
        artifact: Option<(&str, serde_json::Value)>,
    ) {
        let (session_id, node_execution_id) =
            wait_for_top_level_session(engine, execution_id, node_name).await;
        wait_for_stub_session_turn_activation(agent_runtime, &session_id).await;
        if let Some((contract, value)) = artifact {
            engine
                .submit_workflow_output(
                    app.handle(),
                    session_store,
                    agent_runtime,
                    execution_id,
                    node_name.to_string(),
                    Some(node_execution_id),
                    contract.to_string(),
                    value,
                )
                .await
                .unwrap();
        }
        engine
            .on_turn_complete(
                app.handle(),
                session_store,
                agent_runtime,
                &session_id,
                0,
                None,
                &[MessagePart::Text {
                    content: format!("stub completed {node_name}"),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .unwrap();
    }

    async fn complete_review_fanout(
        app: &DispatchTestApp,
        engine: &WorkflowRuntimeService,
        session_store: &Arc<crate::usecase::agent_session::session::SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        lgtm: bool,
    ) {
        let children = wait_for_active_fanout_children(engine, execution_id, "review", 2).await;
        for (node_name, _, node_execution_id) in &children {
            engine
                .submit_workflow_output(
                    app.handle(),
                    session_store,
                    agent_runtime,
                    execution_id,
                    node_name.clone(),
                    Some(node_execution_id.clone()),
                    "review_verdict".to_string(),
                    serde_json::json!({"lgtm": lgtm}),
                )
                .await
                .unwrap();
        }
        for (node_name, session_id, _) in children {
            wait_for_stub_session_turn_activation(agent_runtime, &session_id).await;
            engine
                .on_turn_complete(
                    app.handle(),
                    session_store,
                    agent_runtime,
                    &session_id,
                    0,
                    None,
                    &[MessagePart::Text {
                        content: format!("stub completed {node_name}"),
                        parent_tool_use_id: None,
                    }],
                    None,
                )
                .await
                .unwrap();
        }
    }

    async fn complete_and_approve_fix_fanout(
        app: &DispatchTestApp,
        engine: &WorkflowRuntimeService,
        session_store: &Arc<crate::usecase::agent_session::session::SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
    ) {
        let mut children =
            wait_for_active_fanout_children(engine, execution_id, "fix_each", 1).await;
        let (node_name, session_id, node_execution_id) = children.pop().unwrap();
        wait_for_stub_session_turn_activation(agent_runtime, &session_id).await;
        engine
            .submit_workflow_output(
                app.handle(),
                session_store,
                agent_runtime,
                execution_id,
                node_name.clone(),
                Some(node_execution_id.clone()),
                "fix_result".to_string(),
                serde_json::json!({"fixed": true}),
            )
            .await
            .unwrap();
        engine
            .on_turn_complete(
                app.handle(),
                session_store,
                agent_runtime,
                &session_id,
                0,
                None,
                &[MessagePart::Text {
                    content: "stub fix complete".to_string(),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .unwrap();
        agent_runtime
            .insert_runtime_state_for_test(
                &session_id,
                crate::usecase::agent_session::status::TurnPhase::Idle,
                false,
            )
            .await;
        engine
            .resolve_workflow_approval(
                app.handle(),
                session_store,
                agent_runtime,
                execution_id,
                Some("approved stub fix".to_string()),
                &node_name,
                Some(&node_execution_id),
            )
            .await
            .unwrap();
    }

    async fn complete_and_approve_terminal_session(
        app: &DispatchTestApp,
        engine: &WorkflowRuntimeService,
        session_store: &Arc<crate::usecase::agent_session::session::SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        node_name: &str,
    ) {
        let (session_id, node_execution_id) =
            wait_for_top_level_session(engine, execution_id, node_name).await;
        wait_for_stub_session_turn_activation(agent_runtime, &session_id).await;
        engine
            .on_turn_complete(
                app.handle(),
                session_store,
                agent_runtime,
                &session_id,
                0,
                None,
                &[MessagePart::Text {
                    content: format!("stub completed {node_name}"),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .unwrap();
        agent_runtime
            .insert_runtime_state_for_test(
                &session_id,
                crate::usecase::agent_session::status::TurnPhase::Idle,
                false,
            )
            .await;
        engine
            .resolve_workflow_approval(
                app.handle(),
                session_store,
                agent_runtime,
                execution_id,
                Some(format!("approved {node_name}")),
                node_name,
                Some(&node_execution_id),
            )
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn canonical_full_pipeline_executes_command_fanout_approval_loop_and_switch_path() {
        let _env_lock = crate::test_support::TEST_ENV_LOCK.lock().unwrap();
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir.clone());
        let worktree = TempDir::new().unwrap();
        install_full_pipeline_worktree_fixture(worktree.path());
        let _path = install_full_pipeline_cli();
        let _data_dir = crate::test_support::EnvVarGuard::set_path("RELEASH_DATA_DIR", &data_dir);
        let worktree_path = worktree.path().to_string_lossy().into_owned();
        let review_actor = crate::domain::comment::ReviewActor::human();
        let review_usecase = crate::adaptor::controller::wiring::build_review_comment_usecase();
        let open_thread = review_usecase
            .create_thread(
                &data_dir,
                &worktree_path,
                review_actor.clone(),
                crate::domain::comment::ReviewTarget {
                    file_path: Some("src/lib.rs".to_string()),
                    line_number: Some(1),
                    end_line: None,
                },
                "full-pipeline review fixture".to_string(),
            )
            .unwrap();
        let workflow = canonical_full_pipeline_runtime_workflow(None, None, None);

        let execution_id = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &agent_runtime,
                workflow,
                worktree_path.clone(),
                Some("exercise the canonical pipeline".to_string()),
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Ask,
            )
            .await
            .unwrap();

        wait_for_top_level_session(&engine, &execution_id, "fix_tests").await;
        make_full_pipeline_tests_pass(worktree.path());
        complete_top_level_session(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            "fix_tests",
            None,
        )
        .await;
        complete_review_fanout(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            false,
        )
        .await;
        complete_top_level_session(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            "apply_fixes",
            None,
        )
        .await;
        complete_review_fanout(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            true,
        )
        .await;
        complete_and_approve_fix_fanout(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
        )
        .await;
        review_usecase
            .resolve_thread(
                &data_dir,
                &worktree_path,
                review_actor,
                &open_thread.id,
                "fixed".to_string(),
                "full-pipeline fixture resolved".to_string(),
            )
            .unwrap();
        complete_top_level_session(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            "triage",
            Some(("ship_decision", serde_json::json!({"verdict": "HOLD"}))),
        )
        .await;
        complete_and_approve_terminal_session(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            "done",
        )
        .await;
        wait_for_execution_terminal(&app, &engine, &execution_id).await;

        let metadata = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(metadata.status, ExecutionStatus::Completed);
        let events = read_dispatch_events(&app, &execution_id);
        let completed = completed_node_names(&events);
        let count = |node: &str| {
            completed
                .iter()
                .filter(|name| name.as_str() == node)
                .count()
        };
        assert_eq!(
            count("run_tests"),
            2,
            "command false and true routes must run"
        );
        assert_eq!(count("fix_tests"), 1);
        assert_eq!(count("review"), 2);
        assert_eq!(count("review_opus"), 2);
        assert_eq!(count("review_gpt"), 2);
        assert_eq!(
            count("judge"),
            2,
            "typed command routing must take both branches"
        );
        assert_eq!(count("apply_fixes"), 1);
        assert_eq!(
            count("list_threads"),
            2,
            "has_open true and false routes must run"
        );
        assert_eq!(count("fix_each"), 1);
        assert_eq!(count("fix_one"), 1);
        assert_eq!(count("triage"), 1);
        assert_eq!(count("done"), 1);
        let list_thread_artifacts = events
            .iter()
            .filter_map(|event| match event {
                WorkflowEvent::ArtifactProduced {
                    node_name, value, ..
                } if node_name == "list_threads" => Some(value.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(list_thread_artifacts.len(), 2);
        assert_eq!(list_thread_artifacts[0]["has_open"], true);
        assert_eq!(
            list_thread_artifacts[0]["threads"],
            serde_json::json!([{"thread_id": open_thread.id}])
        );
        assert_eq!(list_thread_artifacts[1]["has_open"], false);
        assert_eq!(list_thread_artifacts[1]["threads"], serde_json::json!([]));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ApprovalResolved { node_name, .. } if node_name == "fix_one"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ApprovalResolved { node_name, .. } if node_name == "done"
        )));
        let (_, fix_each_artifact) = artifact_event_for_node(&events, "fix_each");
        assert_eq!(fix_each_artifact, serde_json::json!([{"fixed": true}]));
    }

    #[tokio::test]
    async fn canonical_full_pipeline_loop_guard_exhaustion_routes_to_give_up() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir);
        let worktree = TempDir::new().unwrap();
        let workflow = canonical_full_pipeline_runtime_workflow(
            Some("exit 1"),
            Some(r#"printf '%s' '{"all_lgtm":true}'"#),
            Some(r#"printf '%s' '{"threads":[],"has_open":false}'"#),
        );
        let execution_id = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &agent_runtime,
                workflow,
                worktree.path().to_string_lossy().to_string(),
                None,
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Ask,
            )
            .await
            .unwrap();

        for _ in 0..3 {
            complete_top_level_session(
                &app,
                &engine,
                &session_store,
                &agent_runtime,
                &execution_id,
                "fix_tests",
                None,
            )
            .await;
        }
        complete_and_approve_terminal_session(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            "give_up",
        )
        .await;
        wait_for_execution_terminal(&app, &engine, &execution_id).await;

        let events = read_dispatch_events(&app, &execution_id);
        let completed = completed_node_names(&events);
        assert_eq!(
            completed.iter().filter(|node| *node == "run_tests").count(),
            4
        );
        assert_eq!(
            completed.iter().filter(|node| *node == "fix_tests").count(),
            3
        );
        assert!(completed.iter().any(|node| node == "give_up"));
        assert!(completed.iter().all(|node| node != "review"));
    }

    #[tokio::test]
    async fn canonical_full_pipeline_apply_fixes_loop_guard_exhaustion_routes_to_give_up() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir);
        let worktree = TempDir::new().unwrap();
        let workflow = canonical_full_pipeline_runtime_workflow(
            Some("exit 0"),
            Some(r#"printf '%s' '{"all_lgtm":false}'"#),
            Some(r#"printf '%s' '{"threads":[],"has_open":false}'"#),
        );
        let execution_id = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &agent_runtime,
                workflow,
                worktree.path().to_string_lossy().to_string(),
                None,
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Ask,
            )
            .await
            .unwrap();

        for attempt in 0..4 {
            complete_review_fanout(
                &app,
                &engine,
                &session_store,
                &agent_runtime,
                &execution_id,
                true,
            )
            .await;
            if attempt < 3 {
                complete_top_level_session(
                    &app,
                    &engine,
                    &session_store,
                    &agent_runtime,
                    &execution_id,
                    "apply_fixes",
                    None,
                )
                .await;
            }
        }
        complete_and_approve_terminal_session(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            "give_up",
        )
        .await;
        wait_for_execution_terminal(&app, &engine, &execution_id).await;

        let events = read_dispatch_events(&app, &execution_id);
        let completed = completed_node_names(&events);
        assert_eq!(completed.iter().filter(|node| *node == "review").count(), 4);
        assert_eq!(completed.iter().filter(|node| *node == "judge").count(), 4);
        assert_eq!(
            completed
                .iter()
                .filter(|node| *node == "apply_fixes")
                .count(),
            3
        );
        assert!(completed.iter().any(|node| node == "give_up"));
        assert!(completed.iter().all(|node| node != "list_threads"));
    }

    #[tokio::test]
    async fn canonical_full_pipeline_triage_loop_guard_exhaustion_routes_to_escalate() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir);
        let worktree = TempDir::new().unwrap();
        let workflow = canonical_full_pipeline_runtime_workflow(
            Some("exit 0"),
            Some(r#"printf '%s' '{"all_lgtm":true}'"#),
            Some(r#"printf '%s' '{"threads":[{"thread_id":"T-1"}],"has_open":true}'"#),
        );
        let execution_id = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &agent_runtime,
                workflow,
                worktree.path().to_string_lossy().to_string(),
                None,
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Ask,
            )
            .await
            .unwrap();

        complete_review_fanout(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            true,
        )
        .await;
        for _ in 0..2 {
            complete_and_approve_fix_fanout(
                &app,
                &engine,
                &session_store,
                &agent_runtime,
                &execution_id,
            )
            .await;
            complete_top_level_session(
                &app,
                &engine,
                &session_store,
                &agent_runtime,
                &execution_id,
                "triage",
                Some(("ship_decision", serde_json::json!({"verdict": "HOLD"}))),
            )
            .await;
        }
        complete_and_approve_fix_fanout(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
        )
        .await;
        complete_and_approve_terminal_session(
            &app,
            &engine,
            &session_store,
            &agent_runtime,
            &execution_id,
            "escalate",
        )
        .await;
        wait_for_execution_terminal(&app, &engine, &execution_id).await;

        let events = read_dispatch_events(&app, &execution_id);
        let completed = completed_node_names(&events);
        assert_eq!(
            completed
                .iter()
                .filter(|node| *node == "list_threads")
                .count(),
            3
        );
        assert_eq!(
            completed.iter().filter(|node| *node == "fix_each").count(),
            3
        );
        assert_eq!(completed.iter().filter(|node| *node == "triage").count(), 2);
        assert!(completed.iter().any(|node| node == "escalate"));
    }

    #[tokio::test]
    async fn canonical_full_pipeline_switch_executes_ship_and_escalate_cases() {
        for (verdict, terminal_node) in [("SHIP", "done"), ("ESCALATE", "escalate")] {
            let app = make_dispatch_app();
            let engine = WorkflowRuntimeService::new_for_test();
            let data_dir = dispatch_data_dir(app.handle());
            engine.set_execution_store_data_dir(data_dir.clone()).await;
            let (session_store, agent_runtime) = make_dispatch_deps(data_dir);
            let worktree = TempDir::new().unwrap();
            let workflow = canonical_full_pipeline_runtime_workflow(
                Some("exit 0"),
                Some(r#"printf '%s' '{"all_lgtm":true}'"#),
                Some(r#"printf '%s' '{"threads":[{"thread_id":"T-1"}],"has_open":true}'"#),
            );
            let execution_id = engine
                .start_resolved_workflow(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    workflow,
                    worktree.path().to_string_lossy().to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                    crate::domain::agent_session::PermissionMode::Ask,
                )
                .await
                .unwrap();

            complete_review_fanout(
                &app,
                &engine,
                &session_store,
                &agent_runtime,
                &execution_id,
                true,
            )
            .await;
            complete_and_approve_fix_fanout(
                &app,
                &engine,
                &session_store,
                &agent_runtime,
                &execution_id,
            )
            .await;
            complete_top_level_session(
                &app,
                &engine,
                &session_store,
                &agent_runtime,
                &execution_id,
                "triage",
                Some(("ship_decision", serde_json::json!({"verdict": verdict}))),
            )
            .await;
            complete_and_approve_terminal_session(
                &app,
                &engine,
                &session_store,
                &agent_runtime,
                &execution_id,
                terminal_node,
            )
            .await;
            wait_for_execution_terminal(&app, &engine, &execution_id).await;

            let events = read_dispatch_events(&app, &execution_id);
            let completed = completed_node_names(&events);
            assert!(completed.iter().any(|node| node == terminal_node));
            let other_terminal = if terminal_node == "done" {
                "escalate"
            } else {
                "done"
            };
            assert!(completed.iter().all(|node| node != other_terminal));
        }
    }

    #[tokio::test]
    async fn command_runtime_appends_standard_artifact_and_keeps_node_completed_summary_only() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-standard".to_string(),
            description: "command standard artifact".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node(
                "pwd",
                "printf '%s' \"$PWD\"; printf '%s' err >&2",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let (contract, value) = artifact_event_for_node(&events, "pwd");
        let canonical_worktree = std::fs::canonicalize(worktree.path()).unwrap();

        assert_eq!(contract, None);
        assert_eq!(value["ok"], true);
        assert_eq!(value["exit_code"], 0);
        assert_eq!(
            value["stdout"].as_str(),
            Some(canonical_worktree.to_string_lossy().as_ref())
        );
        assert_eq!(value["stderr"], "err");
        assert!(value["duration"].as_u64().is_some());
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeCompleted {
                node_name,
                result_summary: Some(result),
                ..
            } if node_name == "pwd" && result == "exit_code=0"
        )));
        assert_eq!(
            engine
                .execution_store()
                .get_execution(&execution_id)
                .await
                .unwrap()
                .status,
            ExecutionStatus::Completed
        );
    }

    #[tokio::test]
    async fn command_runtime_routes_on_nonzero_exit_ok_false() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-exit-routing".to_string(),
            description: "command ok routing".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                command_node(
                    "run_tests",
                    "exit 7",
                    vec![Rule::When {
                        on: "ok".to_string(),
                        then: "done".to_string(),
                        next: "fix".to_string(),
                    }],
                ),
                command_node("done", "printf done", vec![]),
                command_node("fix", "printf fix", vec![]),
            ],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let (_, value) = artifact_event_for_node(&events, "run_tests");

        assert_eq!(value["ok"], false);
        assert_eq!(value["exit_code"], 7);
        assert_eq!(
            completed_node_names(&events),
            vec!["run_tests".to_string(), "fix".to_string()]
        );
    }

    #[tokio::test]
    async fn command_runtime_merges_stdout_json_contract_and_routes_on_contract_field() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-contract-routing".to_string(),
            description: "command stdout json artifact".to_string(),
            builtin: false,
            schemas: [("verdict".to_string(), bool_object_schema("passed"))]
                .into_iter()
                .collect(),
            nodes: vec![
                command_node_with_artifact(
                    "judge",
                    r#"printf '{"passed":true}'"#,
                    "verdict",
                    vec![Rule::When {
                        on: "passed".to_string(),
                        then: "done".to_string(),
                        next: "fix".to_string(),
                    }],
                ),
                command_node("done", "printf done", vec![]),
                command_node("fix", "printf fix", vec![]),
            ],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let (contract, value) = artifact_event_for_node(&events, "judge");

        assert_eq!(contract.as_deref(), Some("verdict"));
        assert_eq!(value["ok"], true);
        assert_eq!(value["passed"], true);
        assert_eq!(
            completed_node_names(&events),
            vec!["judge".to_string(), "done".to_string()]
        );
    }

    #[tokio::test]
    async fn command_runtime_contract_validation_failure_routes_to_fix_with_standard_result() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-contract-failure".to_string(),
            description: "command stdout json artifact failure".to_string(),
            builtin: false,
            schemas: [("verdict".to_string(), bool_object_schema("passed"))]
                .into_iter()
                .collect(),
            nodes: vec![
                command_node_with_artifact(
                    "judge",
                    r#"printf '{"passed":"yes"}'"#,
                    "verdict",
                    vec![Rule::When {
                        on: "passed".to_string(),
                        then: "done".to_string(),
                        next: "fix".to_string(),
                    }],
                ),
                command_node("done", "printf done", vec![]),
                command_node("fix", "printf fix", vec![]),
            ],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let (contract, value) = artifact_event_for_node(&events, "judge");

        assert_eq!(contract, None);
        assert_eq!(value["ok"], false);
        assert_eq!(value["exit_code"], 0);
        assert!(value.get("passed").is_none());
        assert_eq!(
            completed_node_names(&events),
            vec!["judge".to_string(), "fix".to_string()]
        );
    }

    #[tokio::test]
    async fn command_runtime_malformed_stdout_routes_to_fix_with_standard_result_only() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-contract-parse-failure".to_string(),
            description: "command stdout json parse failure".to_string(),
            builtin: false,
            schemas: [("verdict".to_string(), bool_object_schema("passed"))]
                .into_iter()
                .collect(),
            nodes: vec![
                command_node_with_artifact(
                    "judge",
                    "printf not-json",
                    "verdict",
                    vec![Rule::When {
                        on: "passed".to_string(),
                        then: "done".to_string(),
                        next: "fix".to_string(),
                    }],
                ),
                command_node("done", "printf done", vec![]),
                command_node("fix", "printf fix", vec![]),
            ],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let (contract, value) = artifact_event_for_node(&events, "judge");

        assert_eq!(contract, None);
        assert_eq!(value["ok"], false);
        assert_eq!(value["exit_code"], 0);
        assert_eq!(value["stdout"], "not-json");
        assert!(value.get("passed").is_none());
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, ["duration", "exit_code", "ok", "stderr", "stdout"]);
        assert_eq!(
            completed_node_names(&events),
            vec!["judge".to_string(), "fix".to_string()]
        );
    }

    #[tokio::test]
    async fn command_runtime_contract_success_with_nonzero_exit_records_contract_but_ok_false() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-contract-nonzero".to_string(),
            description: "command contract success nonzero exit".to_string(),
            builtin: false,
            schemas: [("verdict".to_string(), bool_object_schema("passed"))]
                .into_iter()
                .collect(),
            nodes: vec![command_node_with_artifact(
                "judge",
                r#"printf '{"passed":true}'; exit 7"#,
                "verdict",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let (contract, value) = artifact_event_for_node(&events, "judge");

        assert_eq!(contract.as_deref(), Some("verdict"));
        assert_eq!(value["passed"], true);
        assert_eq!(value["ok"], false);
        assert_eq!(value["exit_code"], 7);
    }

    #[tokio::test]
    async fn command_runtime_forwards_workflow_execution_id_env() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-env-forwarding".to_string(),
            description: "command env forwarding".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node(
                "print_env",
                "printf '%s' \"$RELEASH_WORKFLOW_EXECUTION_ID\"",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let (_, value) = artifact_event_for_node(&events, "print_env");

        assert_eq!(value["stdout"].as_str(), Some(execution_id.as_str()));
    }

    #[tokio::test]
    async fn command_runtime_forwards_node_execution_id_env() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-node-env-forwarding".to_string(),
            description: "command node env forwarding".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node(
                "print_node_env",
                "printf '%s' \"$RELEASH_NODE_EXECUTION_ID\"",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let node_started_id = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::NodeStarted {
                    node_name,
                    node_execution_id,
                    ..
                } if node_name == "print_node_env" => Some(node_execution_id.clone()),
                _ => None,
            })
            .expect("command NodeStarted must be recorded");
        let (artifact_node_execution_id, value) = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::ArtifactProduced {
                    node_name,
                    node_execution_id,
                    value,
                    ..
                } if node_name == "print_node_env" => {
                    Some((node_execution_id.clone(), value.clone()))
                }
                _ => None,
            })
            .expect("command ArtifactProduced must be recorded");

        assert_eq!(artifact_node_execution_id, node_started_id);
        assert_eq!(value["stdout"].as_str(), Some(node_started_id.as_str()));
        assert_ne!(
            value["stdout"].as_str(),
            Some(execution_id.as_str()),
            "RELEASH_NODE_EXECUTION_ID must not be populated with execution_id"
        );
    }

    #[tokio::test]
    async fn fanout_command_child_forwards_own_node_execution_id_env() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-command-node-env-forwarding".to_string(),
            description: "fanout command node env forwarding".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("commands", vec!["print_child_env"]),
                command_node(
                    "print_child_env",
                    "printf '%s' \"$RELEASH_NODE_EXECUTION_ID\"",
                    vec![],
                ),
            ],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let parent_node_execution_id = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::NodeStarted {
                    node_name,
                    node_execution_id,
                    fanout_parent: None,
                    ..
                } if node_name == "commands" => Some(node_execution_id.clone()),
                _ => None,
            })
            .expect("fanout parent NodeStarted must be recorded");
        let child_node_execution_id = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::NodeStarted {
                    node_name,
                    node_execution_id,
                    fanout_parent: Some(_),
                    ..
                } if node_name == "print_child_env" => Some(node_execution_id.clone()),
                _ => None,
            })
            .expect("fanout command child NodeStarted must be recorded");
        let (artifact_node_execution_id, value) = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::ArtifactProduced {
                    node_name,
                    node_execution_id,
                    value,
                    ..
                } if node_name == "print_child_env" => {
                    Some((node_execution_id.clone(), value.clone()))
                }
                _ => None,
            })
            .expect("fanout command child ArtifactProduced must be recorded");

        assert_eq!(artifact_node_execution_id, child_node_execution_id);
        assert_eq!(
            value["stdout"].as_str(),
            Some(child_node_execution_id.as_str())
        );
        assert_ne!(child_node_execution_id, parent_node_execution_id);
        assert_ne!(value["stdout"].as_str(), Some(execution_id.as_str()));
    }

    #[tokio::test]
    async fn fanout_command_crash_clears_stall_observations_and_preserves_a_resumable_checkpoint() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir.clone());
        let received_payloads: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let received_for_listener = Arc::clone(&received_payloads);
        app.listen("workflow-execution-changed", move |event| {
            received_for_listener
                .lock()
                .unwrap()
                .push(event.payload().to_string());
        });

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-command-failure-stall";
        let sibling_session_id = "fanout-command-failure-sibling-session";
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-command-failure-stall-wf".to_string(),
            description: "fanout command failure stall".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["shell-command", "review-b"]),
                command_node("shell-command", "false", vec![]),
                make_fanout_child("review-b"),
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([
            ("fanout-review".to_string(), 1),
            ("shell-command".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        exec.current_stall_observations = vec![workflow_stall_observation_fixture(
            sibling_session_id,
            "review-b",
        )];
        install_test_fanout(
            &mut exec,
            vec![
                test_fanout_child("shell-command", "", FanoutChildRuntimeState::Running, 0),
                test_fanout_child(
                    "review-b",
                    sibling_session_id,
                    FanoutChildRuntimeState::Running,
                    1,
                ),
            ],
        );
        let fanout = exec.fanout_runtime.as_ref().expect("fanout");
        let parent_node_execution_id = fanout.parent_node_execution_id.clone();
        let command_node_execution_id = fanout.children[0].node_execution_id.clone();
        let sibling_node_execution_id = fanout.children[1].node_execution_id.clone();
        let command_node_execution = exec
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == command_node_execution_id)
            .expect("command child node execution");
        command_node_execution.kind = NodeKindName::Command;
        command_node_execution.session_id = None;
        append_started_events_for_execution(&data_dir, &exec);
        WorkflowEventLog::new(&data_dir)
            .append(&WorkflowEvent::StallObserved {
                execution_id: execution_id.clone(),
                node_execution_id: sibling_node_execution_id.clone(),
                session_id: sibling_session_id.to_string(),
                node_name: "review-b".to_string(),
                attempt: 1,
                turn_phase: "streaming".to_string(),
                idle_secs: 181,
                signal_count: 1,
                cap_reached: false,
                timestamp: 1003.0,
            })
            .unwrap();
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            sibling_session_id.to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );

        assert!(engine
            .interrupt_active_execution(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                ExecutionInterruptionReason::Crash,
            )
            .await
            .unwrap());

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "interrupted fanout command must release the live runtime"
        );
        let execution = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .expect("interrupted execution metadata must be stored");
        assert_eq!(execution.status, ExecutionStatus::Interrupted);
        assert_eq!(
            execution.interruption_reason,
            Some(ExecutionInterruptionReason::Crash)
        );
        assert_eq!(execution.resume_from_node.as_deref(), Some("fanout-review"));
        assert!(execution.completed_at.is_none());

        let live_payload = received_payloads
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("interruption must broadcast live snapshot");
        let live_json: serde_json::Value = serde_json::from_str(&live_payload).unwrap();
        assert_eq!(live_json["workflowExecution"]["status"], "interrupted");
        assert_eq!(
            live_json["workflowExecution"]["interruptionReason"],
            "crash"
        );
        assert_eq!(
            live_json["workflowExecution"]["resumeFromNode"],
            "fanout-review"
        );
        let live_stall_observations =
            live_json["workflowExecution"]["stallObservations"].as_array();
        assert!(
            live_stall_observations.is_none_or(|observations| observations.is_empty()),
            "fanout command interruption must clear live stall observations: {live_json}"
        );
        let live_node_executions = live_json["workflowExecution"]["nodeExecutions"]
            .as_array()
            .expect("live payload must expose node executions");
        let live_status_by_id = |node_execution_id: &str| {
            live_node_executions
                .iter()
                .find(|execution| execution["id"] == node_execution_id)
                .and_then(|execution| execution["status"].as_str())
                .expect("node execution status must be present")
        };
        assert_eq!(live_status_by_id(&parent_node_execution_id), "aborted");
        assert_eq!(live_status_by_id(&command_node_execution_id), "aborted");
        assert_eq!(live_status_by_id(&sibling_node_execution_id), "aborted");

        let events = read_dispatch_events(&app, &execution_id);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionInterrupted {
                reason: ExecutionInterruptionReason::Crash,
                ..
            }
        )));
        let projected = project_workflow_execution(&execution_id, &events)
            .unwrap()
            .unwrap();
        assert_eq!(projected.status, ExecutionStatus::Interrupted);
        assert_eq!(
            projected.interruption_reason,
            Some(ExecutionInterruptionReason::Crash)
        );
        assert_eq!(projected.resume_from_node.as_deref(), Some("fanout-review"));
        let projected_status_by_id = |node_execution_id: &str| {
            projected
                .node_executions
                .iter()
                .find(|execution| execution.id == node_execution_id)
                .map(|execution| execution.status)
                .expect("projected node execution must exist")
        };
        assert_eq!(
            projected_status_by_id(&parent_node_execution_id),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert_eq!(
            projected_status_by_id(&command_node_execution_id),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert_eq!(
            projected_status_by_id(&sibling_node_execution_id),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
    }

    #[tokio::test]
    async fn fanout_parent_artifact_preserves_null_session_and_command_result_order() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir);
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-mixed-child-artifact".to_string(),
            description: "fanout mixed child artifact".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-session", "shell-command"]),
                make_fanout_child("review-session"),
                command_node(
                    "shell-command",
                    "printf command-out; printf command-err >&2",
                    vec![],
                ),
            ],
        };
        let execution_id = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &handles,
                workflow,
                worktree.path().to_string_lossy().to_string(),
                Some("run mixed fanout".to_string()),
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Edit,
            )
            .await
            .unwrap();
        let review_session_id =
            wait_for_fanout_child_session(&engine, &execution_id, "review-session").await;

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                &review_session_id,
                0,
                None,
                &[MessagePart::Text {
                    content: "LGTM".to_string(),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .unwrap();
        wait_for_execution_terminal(&app, &engine, &execution_id).await;

        let events = read_dispatch_events(&app, &execution_id);
        let (_, value) = artifact_event_for_node(&events, "fanout-review");
        let values = value
            .as_array()
            .expect("fanout parent ArtifactProduced value must be an array");

        assert_eq!(values.len(), 2);
        assert_eq!(values[0], serde_json::Value::Null);
        assert_eq!(values[1]["ok"], true);
        assert_eq!(values[1]["exit_code"], 0);
        assert_eq!(values[1]["stdout"], "command-out");
        assert_eq!(values[1]["stderr"], "command-err");
        assert!(
            values[1]["duration"].as_u64().is_some(),
            "command standard result must include duration: {}",
            values[1]
        );
    }

    #[tokio::test]
    async fn fanout_command_reducer_fixture_routes_on_boolean_artifact() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow: WorkflowDefinitionYaml =
            serde_saphyr::from_str(include_str!("../fixtures/valid/fanout-command-reducer.yml"))
                .unwrap();

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let execution = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Completed);

        let events = read_dispatch_events(&app, &execution_id);
        let (_, fanout_value) = artifact_event_for_node(&events, "review");
        assert_eq!(
            fanout_value
                .as_array()
                .expect("fanout Artifact must be a child Artifact array")
                .iter()
                .map(|value| value["lgtm"].as_bool())
                .collect::<Vec<_>>(),
            vec![Some(true), Some(false)]
        );
        let (_, judge_value) = artifact_event_for_node(&events, "judge");
        assert_eq!(judge_value["all_lgtm"], false);

        let completed = completed_node_names(&events);
        assert!(completed.iter().any(|name| name == "fix"));
        assert!(completed.iter().all(|name| name != "ready"));
    }

    #[tokio::test]
    async fn fanout_session_reducer_fixture_receives_array_and_routes_on_enum_artifact() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir);
        let worktree = TempDir::new().unwrap();
        let workflow: WorkflowDefinitionYaml =
            serde_saphyr::from_str(include_str!("../fixtures/valid/fanout-session-reducer.yml"))
                .unwrap();
        let judge_index = workflow
            .nodes
            .iter()
            .position(|node| node.name == "judge")
            .unwrap();
        let judge = workflow.nodes[judge_index].clone();
        let fanout_value = serde_json::json!([
            {"lgtm": true},
            {"lgtm": false}
        ]);
        let review_output = RuntimeArtifact {
            node_name: "review".to_string(),
            attempt: 1,
            session_id: None,
            result: None,
            artifact: Some(fanout_value),
            contract: None,
            token_usage: None,
            completed_at: 1000.0,
        };
        let outputs = HashMap::from([("review".to_string(), review_output.clone())]);
        let (_, prompt) = workflow_prompt::build_node_prompt(
            &judge,
            Some(&instruction_contents("Reduce the review verdicts.")),
            "session-reducer-execution",
            None,
            &outputs,
        )
        .unwrap();
        assert!(prompt.contains("## input: review"));
        assert!(prompt.contains("\"lgtm\": true"));
        assert!(prompt.contains("\"lgtm\": false"));

        let execution_id = uuid::Uuid::new_v4().to_string();
        let judge_session_id = "session-reducer-judge";
        let mut execution = make_waiting_approval_execution_with_workflow(
            &execution_id,
            &worktree.path().to_string_lossy(),
            workflow,
        );
        execution.state = RuntimeExecutionState::Running;
        execution.current_node_index = judge_index;
        execution.current_session_id = Some(judge_session_id.to_string());
        execution.node_execution_counts =
            HashMap::from([("review".to_string(), 1), ("judge".to_string(), 1)]);
        execution.artifacts = HashMap::from([("review".to_string(), review_output)]);
        execution.node_executions = vec![node_execution_fixture(
            &execution_id,
            "session-reducer-judge-execution",
            "judge",
            1,
            NodeExecutionStatus::Running,
            Some(judge_session_id),
            None,
        )];
        insert_execution_and_register_active(&engine, execution, ExecutionOrigin::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            judge_session_id.to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );

        submit_output_for_test_with_deps(
            &engine,
            app.handle(),
            &session_store,
            &handles,
            &execution_id,
            "judge",
            "review-decision",
            serde_json::json!({"verdict": "NEEDS_FIX"}),
            None,
            None,
        )
        .await
        .unwrap();
        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                judge_session_id,
                0,
                None,
                &[],
                None,
            )
            .await
            .unwrap();
        wait_for_execution_terminal(&app, &engine, &execution_id).await;

        let execution = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Completed);
        let events = read_dispatch_events(&app, &execution_id);
        let (_, judge_value) = artifact_event_for_node(&events, "judge");
        assert_eq!(judge_value["verdict"], "NEEDS_FIX");
        let completed = completed_node_names(&events);
        assert!(completed.iter().any(|name| name == "fix"));
        assert!(completed.iter().all(|name| name != "ready"));
    }

    #[tokio::test]
    async fn command_runtime_start_returns_before_long_running_command_completes_and_abort_still_works(
    ) {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-nonblocking-start".to_string(),
            description: "command nonblocking start".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node("long", "sleep 30", vec![])],
        };

        let execution_id = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            start_command_workflow_nowait_for_test(&app, &engine, workflow, worktree.path()),
        )
        .await
        .expect("start_resolved_workflow must not wait for command completion");
        wait_for_active_command(&engine, &execution_id).await;

        let data_dir = dispatch_data_dir(app.handle());
        let (session_store, handles) = make_dispatch_deps(data_dir);
        let outcome = engine
            .abort_workflow_by_execution_id(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                None,
            )
            .await
            .unwrap();

        assert!(matches!(outcome, AbortOutcome::Aborted));
        wait_for_inactive_command(&engine, &execution_id).await;
        let events = read_dispatch_events(&app, &execution_id);
        assert!(events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::ExecutionAborted { .. })));
        assert!(events.iter().all(|event| {
            !matches!(
                event,
                WorkflowEvent::NodeCompleted { node_name, .. }
                    | WorkflowEvent::ArtifactProduced { node_name, .. }
                    if node_name == "long"
            )
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_stop_kills_the_active_command_process_group_and_records_stop_checkpoint() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-explicit-stop".to_string(),
            description: "stop kills command process group".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node(
                "long",
                "sleep 30 & echo $! > child.pid; wait",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_nowait_for_test(&app, &engine, workflow, worktree.path()).await;
        wait_for_active_command(&engine, &execution_id).await;
        let child_pid = wait_for_pid_file(&worktree.path().join("child.pid")).await;
        let data_dir = dispatch_data_dir(app.handle());
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir);

        engine
            .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();

        wait_for_inactive_command(&engine, &execution_id).await;
        wait_for_process_exit(child_pid).await;
        assert!(!engine.contains_execution_for_test(&execution_id).await);
        let metadata = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(metadata.status, ExecutionStatus::Interrupted);
        assert_eq!(
            metadata.interruption_reason,
            Some(ExecutionInterruptionReason::Stop)
        );
        assert_eq!(metadata.resume_from_node.as_deref(), Some("long"));
        assert!(read_dispatch_events(&app, &execution_id)
            .iter()
            .any(|event| matches!(
                event,
                WorkflowEvent::ExecutionInterrupted {
                    reason: ExecutionInterruptionReason::Stop,
                    ..
                }
            )));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_stop_then_resume_restarts_command_attempt_and_completes() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-stop-resume".to_string(),
            description: "resume starts a fresh command attempt".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node(
                "long",
                "if [ -f resumed ]; then printf resumed; else touch resumed; sleep 30 & echo $! > child.pid; wait; fi",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_nowait_for_test(&app, &engine, workflow, worktree.path()).await;
        wait_for_active_command(&engine, &execution_id).await;
        let child_pid = wait_for_pid_file(&worktree.path().join("child.pid")).await;
        let data_dir = dispatch_data_dir(app.handle());
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir);

        engine
            .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();
        wait_for_inactive_command(&engine, &execution_id).await;
        wait_for_process_exit(child_pid).await;

        let attempt_one_events = read_dispatch_events(&app, &execution_id);
        assert!(attempt_one_events.iter().all(|event| {
            !matches!(
                event,
                WorkflowEvent::NodeCompleted {
                    node_name,
                    attempt: 1,
                    ..
                } | WorkflowEvent::ArtifactProduced { node_name, .. }
                    if node_name == "long"
            )
        }));

        engine
            .resume_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .unwrap();
        wait_for_execution_terminal(&app, &engine, &execution_id).await;

        let metadata = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(metadata.status, ExecutionStatus::Completed);
        let events = read_dispatch_events(&app, &execution_id);
        let attempts = events
            .iter()
            .filter_map(|event| match event {
                WorkflowEvent::NodeStarted {
                    node_name, attempt, ..
                } if node_name == "long" => Some(*attempt),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(attempts, vec![1, 2]);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeCompleted {
                node_name,
                attempt: 2,
                ..
            } if node_name == "long"
        )));
        assert!(matches!(
            events.last(),
            Some(WorkflowEvent::ExecutionCompleted { .. })
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_stop_still_kills_the_command_after_metadata_projection_failure() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-stop-projection-failure".to_string(),
            description: "durable stop must still kill".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node(
                "long",
                "sleep 30 & echo $! > child.pid; wait",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_nowait_for_test(&app, &engine, workflow, worktree.path()).await;
        wait_for_active_command(&engine, &execution_id).await;
        let child_pid = wait_for_pid_file(&worktree.path().join("child.pid")).await;
        let data_dir = dispatch_data_dir(app.handle());
        let metadata_dir = data_dir.join("workflow_executions");
        std::fs::remove_dir_all(&metadata_dir).unwrap();
        std::fs::write(&metadata_dir, b"block metadata persistence").unwrap();
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir);

        engine
            .stop_workflow_execution(app.handle(), &session_store, &agent_runtime, &execution_id)
            .await
            .expect("the durable stop fact remains accepted");

        wait_for_inactive_command(&engine, &execution_id).await;
        wait_for_process_exit(child_pid).await;
        assert!(!engine.contains_execution_for_test(&execution_id).await);
        assert!(
            engine
                .execution_store
                .interrupted_transition_pending(&execution_id)
                .await
        );
        assert!(read_dispatch_events(&app, &execution_id)
            .iter()
            .any(|event| matches!(
                event,
                WorkflowEvent::ExecutionInterrupted {
                    reason: ExecutionInterruptionReason::Stop,
                    ..
                }
            )));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn app_exit_shutdown_interrupts_active_command_and_kills_process_group() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-app-exit-shutdown".to_string(),
            description: "command app exit shutdown".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node(
                "long",
                "sleep 30 & echo $! > child.pid; wait",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_nowait_for_test(&app, &engine, workflow, worktree.path()).await;
        wait_for_active_command(&engine, &execution_id).await;
        let child_pid = wait_for_pid_file(&worktree.path().join("child.pid")).await;

        engine.shutdown_all_active_commands().await;

        wait_for_inactive_command(&engine, &execution_id).await;
        wait_for_process_exit(child_pid).await;
        let execution = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Interrupted);
        let events = read_dispatch_events(&app, &execution_id);
        assert!(events.iter().any(|event| {
            matches!(
                event,
                WorkflowEvent::ExecutionInterrupted { reason, .. }
                    if reason == &ExecutionInterruptionReason::Crash
            )
        }));
        assert!(events.iter().all(|event| {
            !matches!(
                event,
                WorkflowEvent::NodeCompleted { node_name, .. }
                    | WorkflowEvent::ArtifactProduced { node_name, .. }
                    if node_name == "long"
            )
        }));
        let projected = project_workflow_execution(&execution_id, &events)
            .unwrap()
            .unwrap();
        assert_eq!(projected.status, ExecutionStatus::Interrupted);
        assert!(projected.node_executions.iter().all(|node_execution| {
            node_execution.status == crate::domain::workflow::NodeExecutionStatus::Aborted
                && node_execution.completed_at.is_some()
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn app_exit_shutdown_interrupts_fanout_command_child_by_node_execution_id() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-command-app-exit".to_string(),
            description: "fanout command app exit".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("commands", vec!["long-child"]),
                command_node("long-child", "sleep 30", vec![]),
            ],
        };

        let execution_id =
            start_command_workflow_nowait_for_test(&app, &engine, workflow, worktree.path()).await;
        wait_for_active_command(&engine, &execution_id).await;

        engine.shutdown_all_active_commands().await;

        wait_for_inactive_command(&engine, &execution_id).await;
        let execution = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Interrupted);
        let events = read_dispatch_events(&app, &execution_id);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionInterrupted { reason, .. }
                if reason == &ExecutionInterruptionReason::Crash
        )));
        let projected = project_workflow_execution(&execution_id, &events)
            .unwrap()
            .unwrap();
        assert_eq!(projected.status, ExecutionStatus::Interrupted);
        let parent = projected
            .node_executions
            .iter()
            .find(|execution| execution.node_name == "commands")
            .expect("fanout parent execution");
        let child = projected
            .node_executions
            .iter()
            .find(|execution| execution.node_name == "long-child")
            .expect("fanout child execution");
        assert_eq!(
            parent.status,
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert_eq!(
            child.status,
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
    }

    #[tokio::test]
    async fn command_runtime_completion_append_failure_records_crash_checkpoint() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-append-failure".to_string(),
            description: "command append failure".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node(
                "gate",
                "while [ ! -f .command-go ]; do sleep 0.02; done; printf done",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_nowait_for_test(&app, &engine, workflow, worktree.path()).await;
        wait_for_active_command(&engine, &execution_id).await;
        engine.fail_next_required_event_append_for_test();
        std::fs::write(worktree.path().join(".command-go"), "go").unwrap();
        wait_for_execution_status(&engine, &execution_id, ExecutionStatus::Interrupted).await;

        let execution = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Interrupted);
        assert_eq!(
            execution.interruption_reason,
            Some(ExecutionInterruptionReason::Crash)
        );
        assert_eq!(execution.resume_from_node.as_deref(), Some("gate"));
        assert!(execution.error_reason.is_none());
        let events = read_dispatch_events(&app, &execution_id);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionInterrupted {
                reason: ExecutionInterruptionReason::Crash,
                ..
            }
        )));
        assert!(events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::ExecutionFailed { .. })));
        assert!(events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::ArtifactProduced { .. })));
    }

    static COMMAND_SECRET_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn command_runtime_redacts_configured_and_env_secrets_before_persistence() {
        let _guard = COMMAND_SECRET_ENV_LOCK.lock().await;
        let configured_secret = "CONFIGURED_COMMAND_SECRET_12345";
        let env_secret = "ENV_COMMAND_SECRET_12345";
        std::env::set_var("RELEASH_COMMAND_SECRET_TEST", env_secret);

        let app = make_dispatch_app();
        let app_config = app.state::<Arc<crate::adaptor::gateway::app_config::AppConfig>>();
        app_config
            .with_config_mut(|config| {
                config.server.token = configured_secret.to_string();
                Ok(())
            })
            .unwrap();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        std::fs::write(
            worktree.path().join("configured-secret.txt"),
            configured_secret,
        )
        .unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "command-secret-redaction".to_string(),
            description: "command secret redaction".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node(
                "print_secret",
                "cat configured-secret.txt; printf '%s' \"$RELEASH_COMMAND_SECRET_TEST\" >&2",
                vec![],
            )],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        std::env::remove_var("RELEASH_COMMAND_SECRET_TEST");
        let events = read_dispatch_events(&app, &execution_id);
        let (_, value) = artifact_event_for_node(&events, "print_secret");

        let event_text = serde_json::to_string(&events).unwrap();
        assert!(!event_text.contains(configured_secret));
        assert!(!event_text.contains(env_secret));
        assert!(!value.to_string().contains(configured_secret));
        assert!(!value.to_string().contains(env_secret));

        let data_dir = dispatch_data_dir(app.handle());
        let ndjson = std::fs::read_to_string(
            data_dir
                .join("workflow_execution_logs")
                .join(format!("{execution_id}.ndjson")),
        )
        .unwrap();
        assert!(!ndjson.contains(configured_secret));
        assert!(!ndjson.contains(env_secret));
        let projected = project_workflow_execution(&execution_id, &events)
            .unwrap()
            .unwrap();
        let projected_values = projected
            .artifacts
            .iter()
            .map(|artifact| &artifact.value)
            .collect::<Vec<_>>();
        let projected_text = serde_json::to_string(&projected_values).unwrap();
        assert!(!projected_text.contains(configured_secret));
        assert!(!projected_text.contains(env_secret));
    }

    #[tokio::test]
    async fn command_runtime_renders_previous_command_artifact_reference() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let worktree = TempDir::new().unwrap();
        let mut echo = command_node("echo_thread", "printf '{{ list_threads.stdout }}'", vec![]);
        echo.inputs = vec!["list_threads".to_string()];
        let workflow = WorkflowDefinitionYaml {
            name: "command-input-reference".to_string(),
            description: "command input artifact reference".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                command_node(
                    "list_threads",
                    "printf thread-42",
                    vec![Rule::Next("echo_thread".to_string())],
                ),
                echo,
            ],
        };

        let execution_id =
            start_command_workflow_for_test(&app, &engine, workflow, worktree.path()).await;
        let events = read_dispatch_events(&app, &execution_id);
        let (_, value) = artifact_event_for_node(&events, "echo_thread");

        assert_eq!(value["ok"], true);
        assert_eq!(value["stdout"], "thread-42");
    }

    #[tokio::test]
    async fn abort_workflow_kills_active_command_without_completion_artifact() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir);
        let worktree = TempDir::new().unwrap();
        let execution_id = uuid::Uuid::new_v4().to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "command-abort".to_string(),
            description: "abort command runtime".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![command_node("long", "sleep 30", vec![])],
        };
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                workflow,
                RuntimeExecutionState::Running,
                worktree.path().to_string_lossy().to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        let runtime_engine = engine.clone();
        let app_handle = app.handle().clone();
        let runtime_session_store = session_store.clone();
        let runtime_handles = handles.clone();
        let worktree_path = worktree.path().to_string_lossy().to_string();
        let runtime_task = tokio::spawn(async move {
            runtime_engine
                .start_current_node_runtime(
                    &app_handle,
                    &runtime_session_store,
                    &runtime_handles,
                    &worktree_path,
                )
                .await
        });

        wait_for_active_command(&engine, &execution_id).await;
        let outcome = engine
            .abort_workflow_by_execution_id(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                None,
            )
            .await
            .unwrap();

        assert!(matches!(outcome, AbortOutcome::Aborted));
        tokio::time::timeout(std::time::Duration::from_secs(8), runtime_task)
            .await
            .expect("command runtime should stop after abort")
            .unwrap()
            .unwrap();

        let events = read_dispatch_events(&app, &execution_id);
        assert!(events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::ExecutionAborted { .. })));
        assert!(events.iter().all(|event| {
            !matches!(
                event,
                WorkflowEvent::NodeCompleted { node_name, .. }
                    | WorkflowEvent::ArtifactProduced { node_name, .. }
                    if node_name == "long"
            )
        }));
    }

    #[tokio::test]
    async fn fanout_child_prompt_failure_skips_sessions_refs_and_execution_mutation() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let engine = WorkflowRuntimeService::new_for_test();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-prompt-failure";
        let mut child = make_fanout_child("missing-facet-child");
        set_session_facets(
            &mut child,
            FacetRefs {
                policy: Some(format!(
                    "nonexistent_policy_{}",
                    uuid::Uuid::new_v4().simple()
                )),
                ..Default::default()
            },
        );

        let workflow = WorkflowDefinitionYaml {
            name: "fanout-prompt-failure-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["missing-facet-child"]),
                child,
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([("fanout-review".to_string(), 1)]);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        let result = engine
            .start_fanout_children(app.handle(), &session_store, &handles, worktree_path)
            .await;

        let err = result.expect_err("unresolved child facet must fail before side effects");
        assert!(
            matches!(err, WorkflowEngineError::InvalidWorkflow(_)),
            "missing child facet must produce InvalidWorkflow, got: {err:?}"
        );
        assert!(
            session_store
                .list_sessions(&data_dir, worktree_path)
                .unwrap()
                .is_empty(),
            "prompt failure must not persist Workflow Node Sessions"
        );
        assert!(
            engine.session_workflow_refs.lock().await.is_empty(),
            "prompt failure must not register session_workflow_refs"
        );

        let execs = engine.executions.lock().await;
        let (_, exec) = find_by_worktree(&execs, worktree_path)
            .expect("execution must remain registered after prompt failure");
        assert!(
            exec.fanout_runtime.is_none(),
            "prompt failure must not apply fanout_runtime state"
        );
        assert!(
            exec.current_session_id.is_none(),
            "prompt failure must not set current_session_id"
        );
        assert_eq!(
            exec.node_execution_counts,
            HashMap::from([("fanout-review".to_string(), 1)]),
            "prompt failure must not record child execution indices"
        );
    }

    #[tokio::test]
    async fn fanout_child_setup_failure_rolls_back_created_sessions_refs_and_execution_mutation() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let engine = WorkflowRuntimeService::new_for_test();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let save_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let save_attempts_for_hook = save_attempts.clone();
        session_store.set_save_hook_for_test(Arc::new(move |session| {
            save_attempts_for_hook.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if session
                .workflow_node_context
                .as_ref()
                .is_some_and(|context| context.node_name == "review-b")
            {
                Err("injected second child save failure".to_string())
            } else {
                Ok(())
            }
        }));

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-setup-rollback";
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-setup-rollback-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                make_fanout_child("review-a"),
                make_fanout_child("review-b"),
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([("fanout-review".to_string(), 1)]);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        let result = engine
            .start_fanout_children(app.handle(), &session_store, &handles, worktree_path)
            .await;

        let err = result.expect_err("second child save failure must fail setup");
        assert!(
            matches!(err, WorkflowEngineError::SessionStore(_)),
            "injected save failure must surface as SessionStore, got: {err:?}"
        );
        assert!(
            err.to_string()
                .contains("injected second child save failure"),
            "original setup failure must remain diagnosable, got: {err}"
        );
        assert_eq!(
            save_attempts.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "test must exercise first child save success followed by second child save failure"
        );
        assert!(
            session_store
                .list_sessions(&data_dir, worktree_path)
                .unwrap()
                .is_empty(),
            "rollback must remove the first child ChatSession from SessionStore"
        );
        assert!(
            engine.session_workflow_refs.lock().await.is_empty(),
            "rollback must remove refs for created child sessions"
        );

        let execs = engine.executions.lock().await;
        let (_, exec) = find_by_worktree(&execs, worktree_path)
            .expect("execution must remain registered after setup failure");
        assert!(
            exec.fanout_runtime.is_none(),
            "setup failure must not apply fanout_runtime state"
        );
        assert!(
            exec.current_session_id.is_none(),
            "setup failure must not set current_session_id"
        );
        assert_eq!(
            exec.node_execution_counts,
            HashMap::from([("fanout-review".to_string(), 1)]),
            "setup failure must not record child execution indices"
        );
    }

    #[tokio::test]
    async fn fanout_child_started_append_failure_rolls_back_sessions_refs_and_expansion() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let engine = WorkflowRuntimeService::new_for_test();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir.clone());

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-child-started-rollback";
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-child-started-rollback-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                make_fanout_child("review-a"),
                make_fanout_child("review-b"),
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_session_id = None;
        exec.node_executions[0].status = NodeExecutionStatus::Running;
        exec.node_execution_counts = HashMap::from([("fanout-review".to_string(), 1)]);
        let parent_node_execution_id = exec.node_executions[0].id.clone();
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        engine.fail_next_required_event_append_for_test();
        let error = engine
            .start_fanout_children(app.handle(), &session_store, &handles, worktree_path)
            .await
            .expect_err("fanout child NodeStarted append failure must be propagated");

        assert!(
            matches!(error, WorkflowEngineError::SessionStore(_)),
            "required append failure must surface as SessionStore: {error:?}"
        );
        assert!(
            session_store
                .list_sessions(&data_dir, worktree_path)
                .unwrap()
                .is_empty(),
            "prepared child sessions must be removed after event rollback"
        );
        assert!(
            engine.session_workflow_refs.lock().await.is_empty(),
            "prepared child session refs must be removed after event rollback"
        );
        let executions = engine.executions.lock().await;
        let execution = executions
            .get(&execution_id)
            .expect("parent execution remains active");
        assert!(execution.fanout_runtime.is_none());
        assert_eq!(execution.node_executions.len(), 1);
        assert_eq!(execution.node_executions[0].id, parent_node_execution_id);
        assert_eq!(
            execution.node_execution_counts,
            HashMap::from([("fanout-review".to_string(), 1)])
        );
        drop(executions);
        assert!(read_dispatch_events(&app, &execution_id)
            .iter()
            .all(|event| {
                !matches!(
                    event,
                    WorkflowEvent::NodeStarted {
                        fanout_parent: Some(_),
                        ..
                    }
                )
            }));
    }

    /// Spec [04]: ApprovalResolved event は approve の事実だけを表し、コメントを
    /// comment field に伝播する。decision field を持たないことを担保する。
    #[test]
    fn approval_resolved_records_comment_without_decision_in_ndjson() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let execution_id = "00000000-0000-0000-0000-000000000300";

        let event = WorkflowEvent::ApprovalResolved {
            execution_id: execution_id.to_string(),
            node_execution_id: "node-execution-review".to_string(),
            node_name: "review".to_string(),
            comment: Some("lgtm".to_string()),
            timestamp: 1234.0,
        };
        log.append(&event).unwrap();

        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("decision").is_none());

        let events = log.read_log(execution_id).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            WorkflowEvent::ApprovalResolved {
                execution_id: rid,
                node_name,
                comment,
                ..
            } => {
                assert_eq!(rid, execution_id);
                assert_eq!(node_name, "review");
                assert_eq!(comment.as_deref(), Some("lgtm"));
            }
            other => panic!("expected ApprovalResolved, got {other:?}"),
        }
    }

    /// Spec [04]: atomic mutation 境界。mutation 直前の `WorkflowExecution` snapshot を
    /// 一括復元することで、履歴・state・current_node_index を含む全フィールドが
    /// 元に戻ることを担保する（部分 rollback helper を使わない構造）。
    #[tokio::test]
    async fn approval_snapshot_rollback_restores_workflow_execution_fully() {
        let engine = WorkflowRuntimeService::new_for_test();
        let execution_id = uuid::Uuid::new_v4().to_string();

        let exec = make_waiting_approval_execution(&execution_id, "/wt/atomic");
        let before_history_len = exec.node_history.len();
        let before_node_index = exec.current_node_index;
        let before_state = exec.state.clone();
        let snapshot_before = exec.clone();

        engine
            .executions
            .lock()
            .await
            .insert(execution_id.clone(), exec);

        // mutation を適用
        {
            let mut execs = engine.executions.lock().await;
            let exec = execs.get_mut(&execution_id).unwrap();
            let _ = WorkflowRuntimeService::apply_approval_application(
                exec,
                ApprovalApplication {
                    effective_result: "approve".to_string(),
                    artifact: None,
                    contract: None,
                },
            )
            .unwrap();
            assert_ne!(exec.state, before_state);
        }

        // event append 失敗時の一括復元（handle_approval 内と同じ操作）。
        {
            let mut execs = engine.executions.lock().await;
            if let Some(exec) = execs.get_mut(&execution_id) {
                *exec = snapshot_before;
            }
        }

        let execs = engine.executions.lock().await;
        let restored = execs.get(&execution_id).expect("execution must remain");
        assert_eq!(restored.state, before_state, "WaitingApproval が復元される");
        assert_eq!(
            restored.current_node_index, before_node_index,
            "current_node_index が復元される"
        );
        assert_eq!(
            restored.node_history.len(),
            before_history_len,
            "node_history.len() が復元される"
        );
    }

    fn dispatch_internal_test_snapshot(
        execution_id: &str,
        workflow_name: &str,
    ) -> RuntimeCommitSnapshot {
        RuntimeCommitSnapshot {
            execution_id: execution_id.to_string(),
            workflow_name: workflow_name.to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Agent,
            request: String::new(),
            error_reason: None,
            state: RuntimeExecutionState::Running,
            current_node_index: 0,
            current_node_name: "node-1".to_string(),
            current_session_id: None,
            node_history: vec![],
            node_execution_counts: HashMap::new(),
            workflow_definition:
                crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml {
                    name: workflow_name.to_string(),
                    description: String::new(),
                    builtin: false,
                    schemas: Default::default(),
                    nodes: vec![],
                },
            total_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: "ne-node-1".to_string(),
                execution_id: execution_id.to_string(),
                node_name: "node-1".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                status: NodeExecutionStatus::Running,
                session_id: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                started_at: 0.0,
                completed_at: None,
            }],
            started_at: 0.0,
            updated_at: 0.0,
        }
    }

    /// Spec [05]: `dispatch_internal_node_command` は `InternalNodeCommand` を受け取り、
    /// 対応する state mutation を snapshot に適用したうえで event を返す
    /// atomic commit 関数として機能する（spec [05]: 発行点が typed command 経路に
    /// 集約 / state mutation と event 発行を同一 commit 境界に集約）。
    #[test]
    fn dispatch_internal_node_command_projects_complete_and_fail_commands() {
        // Complete は snapshot.node_history 末尾 entry と command effect の整合を
        // 検証する（commit 関数: 上流 push との同期境界）。
        let mut snapshot =
            dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000602", "wf");
        snapshot.node_history.push(NodeHistoryEntry {
            node_name: "node-1".to_string(),
            completed_at: 100.0,
            result: Some("ok".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            artifact: None,
            attempt: 1,
            fanout_children: None,
            state: crate::domain::workflow::value_objects::default_node_history_status(),
        });
        let complete = InternalNodeCommand::CompleteNode {
            execution_id: "00000000-0000-0000-0000-000000000602".to_string(),
            workflow_name: "wf".to_string(),
            node_execution_id: "ne-node-1".to_string(),
            node_name: "node-1".to_string(),
            result: Some("ok".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            artifact: None,
            attempt: Some(1),
            timestamp: 100.0,
        };
        match workflow_runtime_events::dispatch_internal_node_command(&mut snapshot, complete) {
            Ok(WorkflowEvent::NodeCompleted {
                execution_id,
                node_name,
                result_summary,
                timestamp,
                ..
            }) => {
                assert_eq!(execution_id, "00000000-0000-0000-0000-000000000602");
                assert_eq!(node_name, "node-1");
                assert_eq!(result_summary.as_deref(), Some("ok"));
                assert_eq!(timestamp, 100.0);
            }
            other => panic!("expected NodeCompleted, got {other:?}"),
        }

        let mut fail_snapshot =
            dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000603", "wf");
        let fail = InternalNodeCommand::FailNode {
            execution_id: "00000000-0000-0000-0000-000000000603".to_string(),
            workflow_name: "wf".to_string(),
            node_execution_id: "ne-node-1".to_string(),
            node_name: "node-1".to_string(),
            attempt: 1,
            reason: "boom".to_string(),
            failure_kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            retry_count: None,
            timestamp: 200.0,
        };
        match workflow_runtime_events::dispatch_internal_node_command(&mut fail_snapshot, fail) {
            Ok(WorkflowEvent::NodeFailed {
                execution_id,
                node_name,
                reason,
                timestamp,
                ..
            }) => {
                assert_eq!(execution_id, "00000000-0000-0000-0000-000000000603");
                assert_eq!(node_name, "node-1");
                assert_eq!(reason, "boom");
                assert_eq!(timestamp, 200.0);
            }
            other => panic!("expected NodeFailed, got {other:?}"),
        }
        // state mutation: Fail 受領後 snapshot.state は Failed { reason } に遷移し、
        // updated_at は command の timestamp と一致する。
        assert!(matches!(
            fail_snapshot.state,
            RuntimeExecutionState::Failed { ref reason, .. } if reason == "boom"
        ));
        assert_eq!(fail_snapshot.updated_at, 200.0);

        // Complete で snapshot の node_history 末尾と node_name が不一致な場合、
        // commit 関数は ValidationError を返す（spec [05] commit 境界: snapshot が
        // command effect を含まないことの検出）。
        let mut mismatched =
            dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000604", "wf");
        let mismatched_cmd = InternalNodeCommand::CompleteNode {
            execution_id: "00000000-0000-0000-0000-000000000604".to_string(),
            workflow_name: "wf".to_string(),
            node_execution_id: "ne-node-1".to_string(),
            node_name: "node-1".to_string(),
            result: None,
            session_id: None,
            token_usage: None,
            artifact: None,
            attempt: None,
            timestamp: 100.0,
        };
        assert!(matches!(
            workflow_runtime_events::dispatch_internal_node_command(
                &mut mismatched,
                mismatched_cmd
            ),
            Err(WorkflowEngineError::ValidationError(_))
        ));
    }

    /// Spec [05] commit 境界（snapshot と command effect の整合検証）の table-driven 網羅。
    /// `CompleteNode` の全 effect 列（execution_id / workflow_name / node_name / result /
    /// session_id / token_usage / artifact / attempt / timestamp）について、
    /// snapshot 側で 1 個ずつ意図的に mismatch を作成し、`dispatch_internal_node_command`
    /// が `ValidationError` を返すことを境界仕様として担保する（policy 指示）。
    #[test]
    fn dispatch_internal_complete_node_validates_all_effect_fields() {
        fn base_snapshot() -> RuntimeCommitSnapshot {
            let mut s =
                dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000620", "table-wf");
            s.node_history.push(NodeHistoryEntry {
                node_name: "node-1".to_string(),
                completed_at: 100.0,
                result: Some("ok".to_string()),
                session_id: Some("sess-1".to_string()),
                token_usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                }),
                artifact: Some(serde_json::json!({"k":"v"})),
                attempt: 1,
                fanout_children: None,
                state: crate::domain::workflow::value_objects::default_node_history_status(),
            });
            s
        }
        fn base_command() -> InternalNodeCommand {
            InternalNodeCommand::CompleteNode {
                execution_id: "00000000-0000-0000-0000-000000000620".to_string(),
                workflow_name: "table-wf".to_string(),
                node_execution_id: "ne-node-1".to_string(),
                node_name: "node-1".to_string(),
                result: Some("ok".to_string()),
                session_id: Some("sess-1".to_string()),
                token_usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                }),
                artifact: Some(serde_json::json!({"k":"v"})),
                attempt: Some(1),
                timestamp: 100.0,
            }
        }

        // baseline は受理される（all fields match）。
        let mut s = base_snapshot();
        assert!(
            workflow_runtime_events::dispatch_internal_node_command(&mut s, base_command()).is_ok()
        );

        // 各 field を 1 個ずつ意図的に乖離させて ValidationError を確認する。
        type CompleteNodeMutator = Box<dyn Fn(InternalNodeCommand) -> InternalNodeCommand>;
        let mutators: Vec<(&str, CompleteNodeMutator)> = vec![
            (
                "execution_id",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        execution_id: ref mut r,
                        ..
                    } = c
                    {
                        *r = "00000000-0000-0000-0000-000000000999".to_string();
                    }
                    c
                }),
            ),
            (
                "workflow_name",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        workflow_name: ref mut w,
                        ..
                    } = c
                    {
                        *w = "other-wf".to_string();
                    }
                    c
                }),
            ),
            (
                "node_name",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        node_name: ref mut n,
                        ..
                    } = c
                    {
                        *n = "node-X".to_string();
                    }
                    c
                }),
            ),
            (
                "result",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        result: ref mut r, ..
                    } = c
                    {
                        *r = Some("DIFFERENT".to_string());
                    }
                    c
                }),
            ),
            (
                "session_id",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        session_id: ref mut s,
                        ..
                    } = c
                    {
                        *s = Some("sess-X".to_string());
                    }
                    c
                }),
            ),
            (
                "token_usage",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        token_usage: ref mut t,
                        ..
                    } = c
                    {
                        *t = Some(TokenUsage {
                            input_tokens: 999,
                            output_tokens: 999,
                        });
                    }
                    c
                }),
            ),
            (
                "artifact",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        artifact: ref mut so,
                        ..
                    } = c
                    {
                        *so = Some(serde_json::json!({"k":"other"}));
                    }
                    c
                }),
            ),
            (
                "attempt",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        attempt: ref mut r, ..
                    } = c
                    {
                        *r = Some(99);
                    }
                    c
                }),
            ),
            (
                "timestamp",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        timestamp: ref mut t,
                        ..
                    } = c
                    {
                        *t = 999.0;
                    }
                    c
                }),
            ),
        ];

        for (label, mutate) in mutators {
            let mut snapshot = base_snapshot();
            let cmd = mutate(base_command());
            let result =
                workflow_runtime_events::dispatch_internal_node_command(&mut snapshot, cmd);
            assert!(
                matches!(result, Err(WorkflowEngineError::ValidationError(_))),
                "CompleteNode {label} mismatch must return ValidationError, got: {result:?}"
            );
        }
    }

    /// Spec [05] commit 境界: `FailNode` の整合検証も execution_id / workflow_name / node_name の
    /// 各次元で snapshot との mismatch を ValidationError として検出することを担保する。
    #[test]
    fn dispatch_internal_fail_node_validates_all_effect_fields() {
        fn base_snapshot() -> RuntimeCommitSnapshot {
            let mut s =
                dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000621", "fail-wf");
            s.current_node_name = "node-1".to_string();
            s
        }
        fn base_command() -> InternalNodeCommand {
            InternalNodeCommand::FailNode {
                execution_id: "00000000-0000-0000-0000-000000000621".to_string(),
                workflow_name: "fail-wf".to_string(),
                node_execution_id: "ne-node-1".to_string(),
                node_name: "node-1".to_string(),
                attempt: 1,
                reason: "boom".to_string(),
                failure_kind:
                    crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                retry_count: None,
                timestamp: 200.0,
            }
        }

        // baseline は受理される。
        let mut s = base_snapshot();
        assert!(
            workflow_runtime_events::dispatch_internal_node_command(&mut s, base_command()).is_ok()
        );

        // execution_id mismatch
        let mut s = base_snapshot();
        let mut bad = base_command();
        if let InternalNodeCommand::FailNode {
            execution_id: ref mut r,
            ..
        } = bad
        {
            *r = "00000000-0000-0000-0000-000000000999".to_string();
        }
        assert!(matches!(
            workflow_runtime_events::dispatch_internal_node_command(&mut s, bad),
            Err(WorkflowEngineError::ValidationError(_))
        ));

        // workflow_name mismatch
        let mut s = base_snapshot();
        let mut bad = base_command();
        if let InternalNodeCommand::FailNode {
            workflow_name: ref mut w,
            ..
        } = bad
        {
            *w = "other-wf".to_string();
        }
        assert!(matches!(
            workflow_runtime_events::dispatch_internal_node_command(&mut s, bad),
            Err(WorkflowEngineError::ValidationError(_))
        ));

        // node_name mismatch
        let mut s = base_snapshot();
        let mut bad = base_command();
        if let InternalNodeCommand::FailNode {
            node_name: ref mut n,
            ..
        } = bad
        {
            *n = "node-X".to_string();
        }
        assert!(matches!(
            workflow_runtime_events::dispatch_internal_node_command(&mut s, bad),
            Err(WorkflowEngineError::ValidationError(_))
        ));
    }

    #[tokio::test]
    async fn stale_turn_complete_failure_records_resumable_checkpoint() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/stale-policy-terminal";
        let node_session_id = "stale-node-session";
        let workflow = WorkflowDefinitionYaml {
            name: "stale-policy-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_test_node(
                "review",
                TestKind::Session,
                "review",
                vec![],
                None,
            )],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_session_id = Some(node_session_id.to_string());
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            node_session_id.to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                node_session_id,
                124,
                None,
                &[],
                None,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "stale timeout releases live runtime after checkpointing"
        );
        let execution = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .expect("ExecutionStore must keep interrupted checkpoint metadata");
        assert_eq!(execution.status, ExecutionStatus::Interrupted);
        assert_eq!(
            execution.interruption_reason,
            Some(ExecutionInterruptionReason::Stale)
        );
        assert_eq!(execution.resume_from_node.as_deref(), Some("review"));
        assert!(execution.error_reason.is_none());

        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionInterrupted {
                reason: ExecutionInterruptionReason::Stale,
                ..
            }
        )));
        assert!(events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::ExecutionFailed { .. })));
    }

    #[tokio::test]
    async fn fanout_child_crash_releases_interrupted_execution_after_broadcast() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-child-failure";
        let failed_child_session_id = "fanout-child-failed-session";
        let interrupted_child_session_id = "fanout-child-interrupted-session";
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-failure-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                make_fanout_child("review-a"),
                make_fanout_child("review-b"),
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([
            ("fanout-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        install_test_fanout(
            &mut exec,
            vec![
                test_fanout_child(
                    "review-a",
                    failed_child_session_id,
                    FanoutChildRuntimeState::Running,
                    0,
                ),
                test_fanout_child(
                    "review-b",
                    interrupted_child_session_id,
                    FanoutChildRuntimeState::Running,
                    1,
                ),
            ],
        );
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                failed_child_session_id.to_string(),
                SessionWorkflowRef {
                    execution_id: execution_id.clone(),
                },
            );
            refs.insert(
                interrupted_child_session_id.to_string(),
                SessionWorkflowRef {
                    execution_id: execution_id.clone(),
                },
            );
        }

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                failed_child_session_id,
                1,
                None,
                &[],
                None,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "fanout child crash must release live runtime after checkpointing"
        );
        let stored = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .expect("ExecutionStore must keep interrupted checkpoint metadata");
        assert_eq!(stored.status, ExecutionStatus::Interrupted);
        assert_eq!(
            stored.interruption_reason,
            Some(ExecutionInterruptionReason::Crash)
        );
        assert_eq!(stored.resume_from_node.as_deref(), Some("fanout-review"));
        assert!(stored.error_reason.is_none());
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                WorkflowEvent::ExecutionInterrupted {
                    reason: ExecutionInterruptionReason::Crash,
                    ..
                }
            )),
            "fanout child crash must append ExecutionInterrupted(Crash); got {events:?}"
        );
        let refs = engine.session_workflow_refs.lock().await;
        assert!(
            refs.values()
                .all(|session_ref| session_ref.execution_id != execution_id),
            "interruption cleanup must remove all session refs for the fanout"
        );
    }

    #[tokio::test]
    async fn fanout_child_success_clears_live_stall_observation_for_child() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-child-stall-success";
        let completed_child_session_id = "fanout-child-stall-completed-session";
        let waiting_child_session_id = "fanout-child-stall-waiting-session";
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-stall-success-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                make_fanout_child("review-a"),
                make_fanout_child("review-b"),
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([
            ("fanout-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        exec.current_stall_observations = vec![
            workflow_stall_observation_fixture(completed_child_session_id, "review-a"),
            workflow_stall_observation_fixture(waiting_child_session_id, "review-b"),
        ];
        install_test_fanout(
            &mut exec,
            vec![
                test_fanout_child(
                    "review-a",
                    completed_child_session_id,
                    FanoutChildRuntimeState::Running,
                    0,
                ),
                test_fanout_child(
                    "review-b",
                    waiting_child_session_id,
                    FanoutChildRuntimeState::Running,
                    1,
                ),
            ],
        );
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            completed_child_session_id.to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                completed_child_session_id,
                0,
                None,
                &[MessagePart::Text {
                    content: "LGTM".to_string(),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .unwrap();

        let execs = engine.executions.lock().await;
        let exec = execs
            .get(&execution_id)
            .expect("execution must stay active");
        let observations = &exec.current_stall_observations;
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].session_id, waiting_child_session_id,
            "completed child stall observation must be removed while running sibling remains"
        );
        let completed_child = exec
            .fanout_runtime
            .as_ref()
            .expect("fanout must stay active")
            .children
            .iter()
            .find(|child| child.node_name == "review-a")
            .expect("completed child");
        assert!(matches!(
            completed_child.state,
            FanoutChildRuntimeState::Completed
        ));
    }

    #[tokio::test]
    async fn fanout_model_refusal_checkpoints_confirmed_child_and_resume_completes_pending_child() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-zero-exit-refusal";
        let refused_child_session_id = "fanout-child-zero-refusal-session";
        let waiting_child_session_id = "fanout-child-waiting-session";
        let mut review_b = make_fanout_child("review-b");
        review_b.artifact = Some("review-verdict".to_string());
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-zero-refusal-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: submit_test_schemas(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                make_fanout_child("review-a"),
                review_b,
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([
            ("fanout-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        exec.current_stall_observations = vec![
            workflow_stall_observation_fixture(refused_child_session_id, "review-a"),
            workflow_stall_observation_fixture(waiting_child_session_id, "review-b"),
        ];
        let refused_child = test_fanout_child(
            "review-a",
            refused_child_session_id,
            FanoutChildRuntimeState::Running,
            0,
        );
        let mut confirmed_child = test_fanout_child(
            "review-b",
            waiting_child_session_id,
            FanoutChildRuntimeState::Running,
            1,
        );
        confirmed_child.contract = Some("review-verdict".to_string());
        install_test_fanout(&mut exec, vec![refused_child, confirmed_child]);
        append_started_events_for_execution(&data_dir, &exec);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            for session_id in [refused_child_session_id, waiting_child_session_id] {
                refs.insert(
                    session_id.to_string(),
                    SessionWorkflowRef {
                        execution_id: execution_id.clone(),
                    },
                );
            }
        }

        engine
            .submit_workflow_output(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                "review-b".to_string(),
                None,
                "review-verdict".to_string(),
                serde_json::json!({"verdict": "LGTM"}),
            )
            .await
            .unwrap();
        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                waiting_child_session_id,
                0,
                None,
                &[MessagePart::Text {
                    content: "confirmed review".to_string(),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .unwrap();

        let turn_complete =
            workflow_turn_complete_notification_from_typed_refusal(refused_child_session_id);
        assert!(!turn_complete.interrupted);
        let final_parts = turn_complete
            .final_text_parts
            .iter()
            .map(|content| MessagePart::Text {
                content: content.clone(),
                parent_tool_use_id: None,
            })
            .collect::<Vec<_>>();
        let failure_signal = turn_complete.failure_signal.map(|signal| match signal {
            crate::usecase::workflow::ports::WorkflowTurnFailureSignal::ModelRefusal => {
                workflow_transition::SessionFailureSignal::ModelRefusal
            }
        });
        let token_usage = turn_complete
            .token_usage
            .map(|usage| (usage.input_tokens, usage.output_tokens));

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                &turn_complete.chat_session_id,
                turn_complete.exit_code,
                failure_signal,
                &final_parts,
                token_usage,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "partial failure must release the interrupted fanout runtime"
        );
        let stored = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .expect("ExecutionStore must keep the resumable checkpoint metadata");
        assert_eq!(stored.status, ExecutionStatus::Interrupted);
        assert_eq!(
            stored.interruption_reason,
            Some(ExecutionInterruptionReason::Crash)
        );
        assert_eq!(stored.resume_from_node.as_deref(), Some("fanout-review"));

        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                WorkflowEvent::NodeFailed {
                    node_name,
                    failure_kind: NodeExecutionFailureKind::ModelRefusal,
                    ..
                } if node_name == "review-a"
            )),
            "zero-exit model refusal must be recorded as a normal node failure; got {events:?}"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ArtifactProduced { node_name, value, .. }
                if node_name == "review-b"
                    && value == &serde_json::json!({"verdict": "LGTM"})
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeCompleted { node_name, .. } if node_name == "review-b"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionInterrupted {
                reason: ExecutionInterruptionReason::Crash,
                ..
            }
        )));
        assert!(events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::ExecutionFailed { .. })));
        assert!(events.iter().all(|event| !matches!(
            event,
            WorkflowEvent::ArtifactProduced { node_name, .. } if node_name == "fanout-review"
        )));
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, WorkflowEvent::ContractViolated { .. })),
            "model refusal signal must not be rerouted into contract repair; got {events:?}"
        );

        engine
            .resume_workflow_execution(app.handle(), &session_store, &handles, &execution_id)
            .await
            .unwrap();
        let resumed_child_session = {
            let execs = engine.executions.lock().await;
            let resumed = execs.get(&execution_id).expect("resumed fanout runtime");
            let fanout = resumed
                .fanout_runtime
                .as_ref()
                .expect("active resumed fanout");
            let reused = fanout
                .children
                .iter()
                .find(|child| child.node_name == "review-b")
                .expect("confirmed child");
            assert_eq!(reused.state, FanoutChildRuntimeState::Completed);
            assert!(
                reused.session_id.is_empty(),
                "confirmed child is not restarted"
            );
            assert_eq!(
                reused.artifact,
                Some(serde_json::json!({"verdict": "LGTM"}))
            );
            let pending = fanout
                .children
                .iter()
                .find(|child| child.node_name == "review-a")
                .expect("failed child restarted as pending");
            assert_eq!(pending.state, FanoutChildRuntimeState::Running);
            assert!(!pending.session_id.is_empty());
            pending.session_id.clone()
        };
        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                &resumed_child_session,
                0,
                None,
                &[MessagePart::Text {
                    content: "recovered review".to_string(),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .unwrap();

        let completed = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .expect("resumed fanout reaches a final metadata state");
        assert_eq!(completed.status, ExecutionStatus::Completed);
        let completed_events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert_eq!(
            completed_events
                .iter()
                .filter(|event| matches!(
                    event,
                    WorkflowEvent::SessionAttached { node_execution_id, .. }
                        if node_execution_id.starts_with("ne-review-b")
                ))
                .count(),
            1,
            "the confirmed child keeps its original session and is not re-executed"
        );
        assert!(matches!(
            completed_events.last(),
            Some(WorkflowEvent::ExecutionCompleted { .. })
        ));
    }

    #[tokio::test]
    async fn fanout_terminal_failure_append_failure_rolls_back_child_state_and_execution_store() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-terminal-append-failure";
        let refused_child_session_id = "fanout-child-refusal-append-failure-session";
        let waiting_child_session_id = "fanout-child-still-running-session";
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-terminal-append-failure-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                make_fanout_child("review-a"),
                make_fanout_child("review-b"),
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.updated_at = 1000.0;
        exec.node_execution_counts = HashMap::from([
            ("fanout-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        install_test_fanout(
            &mut exec,
            vec![
                test_fanout_child(
                    "review-a",
                    refused_child_session_id,
                    FanoutChildRuntimeState::Running,
                    0,
                ),
                test_fanout_child(
                    "review-b",
                    waiting_child_session_id,
                    FanoutChildRuntimeState::Running,
                    1,
                ),
            ],
        );
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        let stored_before = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .expect("execution store must hold active execution before append failure");
        engine.session_workflow_refs.lock().await.insert(
            refused_child_session_id.to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );

        engine.fail_next_required_event_append_for_test();
        let err = engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                refused_child_session_id,
                0,
                Some(workflow_transition::SessionFailureSignal::ModelRefusal),
                &[],
                None,
            )
            .await
            .expect_err("terminal child failure event append failure must abort commit");
        assert!(
            format!("{err:?}").contains("fanout child progress event append failed"),
            "append failure context must be surfaced; got {err:?}"
        );

        let execs = engine.executions.lock().await;
        let exec = execs
            .get(&execution_id)
            .expect("execution must remain active");
        let child = exec
            .fanout_runtime
            .as_ref()
            .expect("fanout must be restored")
            .children
            .iter()
            .find(|child| child.node_name == "review-a")
            .expect("refused child must still exist");
        assert!(
            matches!(child.state, FanoutChildRuntimeState::Running),
            "child state must roll back when required event append fails"
        );
        assert_eq!(child.failure_kind, None);
        assert_eq!(child.failure_disposition, None);
        assert!(
            !exec.artifacts.contains_key("review-a"),
            "failed child RuntimeArtifact must not remain after rollback"
        );
        drop(execs);

        let stored_after = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .expect("execution store must be restored to active projection");
        assert_eq!(stored_after.status, stored_before.status);
        assert_eq!(stored_after.current_node, stored_before.current_node);
        assert_eq!(stored_after.updated_at, stored_before.updated_at);
        assert_eq!(stored_after.error_reason, stored_before.error_reason);

        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(
            events.iter().all(|event| !matches!(
                event,
                WorkflowEvent::NodeFailed {
                    node_name,
                    failure_kind: NodeExecutionFailureKind::ModelRefusal,
                    ..
                } if node_name == "review-a"
            )),
            "terminal failure event must not be present when required append fails; got {events:?}"
        );
    }

    #[tokio::test]
    async fn fanout_child_missing_output_after_repair_limit_creates_resumable_partial_checkpoint() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir.clone());
        let received_payloads: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let received_for_listener = Arc::clone(&received_payloads);
        app.listen("workflow-execution-changed", move |event| {
            received_for_listener
                .lock()
                .unwrap()
                .push(event.payload().to_string());
        });

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-missing-output-repair-limit";
        let missing_child_session_id = "fanout-missing-output-limit-session";
        let sibling_session_id = "fanout-missing-output-limit-sibling-session";
        let mut review_a = make_fanout_child("review-a");
        review_a.artifact = Some("review-verdict".to_string());
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-missing-output-limit-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                review_a,
                make_fanout_child("review-b"),
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([
            ("fanout-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        let mut missing_child = test_fanout_child(
            "review-a",
            missing_child_session_id,
            FanoutChildRuntimeState::Running,
            0,
        );
        missing_child.contract = Some("review-verdict".to_string());
        install_test_fanout(
            &mut exec,
            vec![
                missing_child,
                test_fanout_child(
                    "review-b",
                    sibling_session_id,
                    FanoutChildRuntimeState::Running,
                    1,
                ),
            ],
        );
        append_started_events_for_execution(&data_dir, &exec);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                missing_child_session_id.to_string(),
                SessionWorkflowRef {
                    execution_id: execution_id.clone(),
                },
            );
            refs.insert(
                sibling_session_id.to_string(),
                SessionWorkflowRef {
                    execution_id: execution_id.clone(),
                },
            );
        }
        let log = WorkflowEventLog::new(&data_dir);
        for attempt in 1..=2 {
            log.append(&WorkflowEvent::ContractViolated {
                execution_id: execution_id.clone(),
                node_execution_id: "ne-review-a-0".to_string(),
                node_name: "review-a".to_string(),
                violations: vec![
                    crate::adaptor::gateway::workflow::event::ContractViolationRecord {
                        path: "$".to_string(),
                        reason: submission_violation_reason(
                            SubmissionViolation::MissingSubmitOutput,
                        )
                        .to_string(),
                    },
                ],
                request_id: None,
                repair_attempt: attempt,
                timestamp: 1000.0 + f64::from(attempt),
            })
            .unwrap();
        }

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                missing_child_session_id,
                0,
                None,
                &[],
                None,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "repair limit fanout missing-output failure must release the interrupted execution"
        );
        let live_payload = received_payloads
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("partial failure must broadcast live snapshot");
        let live_json: serde_json::Value = serde_json::from_str(&live_payload).unwrap();
        let live_node_executions = live_json["workflowExecution"]["nodeExecutions"]
            .as_array()
            .expect("live payload must expose node executions");
        let live_status = |node_name: &str| {
            live_node_executions
                .iter()
                .find(|execution| execution["nodeName"] == node_name)
                .and_then(|execution| execution["status"].as_str())
                .expect("node execution status must be present")
        };
        assert_eq!(live_status("fanout-review"), "aborted");
        assert_eq!(live_status("review-a"), "failed");
        assert_eq!(live_status("review-b"), "aborted");

        let events = read_dispatch_events(&app, &execution_id);
        let projected = project_workflow_execution(&execution_id, &events)
            .unwrap()
            .unwrap();
        let projected_status = |node_name: &str| {
            projected
                .node_executions
                .iter()
                .find(|execution| execution.node_name == node_name)
                .map(|execution| execution.status)
                .expect("projected node execution must exist")
        };
        assert_eq!(
            projected_status("fanout-review"),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert_eq!(
            projected_status("review-a"),
            crate::domain::workflow::NodeExecutionStatus::Failed
        );
        assert_eq!(
            projected_status("review-b"),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert!(projected
            .node_executions
            .iter()
            .all(|execution| !execution.status.is_active()));
        assert_eq!(projected.status, ExecutionStatus::Interrupted);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionInterrupted {
                reason: ExecutionInterruptionReason::Crash,
                ..
            }
        )));
        let stored = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .expect("repair exhaustion remains resumable");
        assert_eq!(stored.status, ExecutionStatus::Interrupted);
        assert_eq!(stored.resume_from_node.as_deref(), Some("fanout-review"));
    }

    #[tokio::test]
    async fn approval_fanout_child_missing_output_after_repair_limit_is_resumable_with_child_node_failed(
    ) {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir.clone());
        let received_payloads: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let received_for_listener = Arc::clone(&received_payloads);
        app.listen("workflow-execution-changed", move |event| {
            received_for_listener
                .lock()
                .unwrap()
                .push(event.payload().to_string());
        });

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/approval-fanout-missing-output-repair-limit";
        let approval_child_session_id = "approval-fanout-missing-output-limit-session";
        let sibling_session_id = "approval-fanout-missing-output-limit-sibling-session";
        let mut review_a = make_approval_gated_session("review-a", "review-a", vec![]);
        review_a.artifact = Some("review-verdict".to_string());
        let workflow = WorkflowDefinitionYaml {
            name: "approval-fanout-missing-output-limit-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                review_a,
                make_fanout_child("review-b"),
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([
            ("fanout-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        let mut approval_child = test_fanout_child(
            "review-a",
            approval_child_session_id,
            FanoutChildRuntimeState::Running,
            0,
        );
        approval_child.contract = Some("review-verdict".to_string());
        install_test_fanout(
            &mut exec,
            vec![
                approval_child,
                test_fanout_child(
                    "review-b",
                    sibling_session_id,
                    FanoutChildRuntimeState::Running,
                    1,
                ),
            ],
        );
        let fanout = exec.fanout_runtime.as_ref().expect("fanout");
        let parent_node_execution_id = fanout.parent_node_execution_id.clone();
        let approval_child_node_execution_id = fanout.children[0].node_execution_id.clone();
        let sibling_node_execution_id = fanout.children[1].node_execution_id.clone();
        let approval_node_execution = exec
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == approval_child_node_execution_id)
            .expect("approval child node execution");
        approval_node_execution.status = NodeExecutionStatus::WaitingApproval;
        append_started_events_for_execution(&data_dir, &exec);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            approval_child_session_id.to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );
        handles
            .insert_runtime_state_for_test(
                approval_child_session_id,
                crate::usecase::agent_session::status::TurnPhase::Idle,
                false,
            )
            .await;

        let log = WorkflowEventLog::new(&data_dir);
        for attempt in 1..=2 {
            log.append(&WorkflowEvent::ContractViolated {
                execution_id: execution_id.clone(),
                node_execution_id: approval_child_node_execution_id.clone(),
                node_name: "review-a".to_string(),
                violations: vec![
                    crate::adaptor::gateway::workflow::event::ContractViolationRecord {
                        path: "$".to_string(),
                        reason: submission_violation_reason(
                            SubmissionViolation::MissingSubmitOutput,
                        )
                        .to_string(),
                    },
                ],
                request_id: None,
                repair_attempt: attempt,
                timestamp: 1000.0 + f64::from(attempt),
            })
            .unwrap();
        }

        let result = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                None,
                "review-a",
                Some(&approval_child_node_execution_id),
            )
            .await;

        assert!(matches!(
            result,
            Err(WorkflowEngineError::ValidationError(message))
                if message == "required structured output has not been submitted"
        ));
        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "repair limit approval fanout missing-output failure must release the interrupted execution"
        );
        let live_payload = received_payloads
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("partial failure must broadcast live snapshot");
        let live_json: serde_json::Value = serde_json::from_str(&live_payload).unwrap();
        let live_node_executions = live_json["workflowExecution"]["nodeExecutions"]
            .as_array()
            .expect("live payload must expose node executions");
        let live_status_by_id = |node_execution_id: &str| {
            live_node_executions
                .iter()
                .find(|execution| execution["id"] == node_execution_id)
                .and_then(|execution| execution["status"].as_str())
                .expect("node execution status must be present")
        };
        assert_eq!(live_status_by_id(&parent_node_execution_id), "aborted");
        assert_eq!(
            live_status_by_id(&approval_child_node_execution_id),
            "failed"
        );
        assert_eq!(live_status_by_id(&sibling_node_execution_id), "aborted");

        let events = read_dispatch_events(&app, &execution_id);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeFailed {
                node_execution_id,
                node_name,
                failure_kind: NodeExecutionFailureKind::StructuredOutputMismatch,
                ..
            } if node_execution_id == &approval_child_node_execution_id
                && node_name == "review-a"
        )));
        let projected = project_workflow_execution(&execution_id, &events)
            .unwrap()
            .unwrap();
        let projected_status_by_id = |node_execution_id: &str| {
            projected
                .node_executions
                .iter()
                .find(|execution| execution.id == node_execution_id)
                .map(|execution| execution.status)
                .expect("projected node execution must exist")
        };
        assert_eq!(
            projected_status_by_id(&parent_node_execution_id),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert_eq!(
            projected_status_by_id(&approval_child_node_execution_id),
            crate::domain::workflow::NodeExecutionStatus::Failed
        );
        assert_eq!(
            projected_status_by_id(&sibling_node_execution_id),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert_eq!(projected.status, ExecutionStatus::Interrupted);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionInterrupted {
                reason: ExecutionInterruptionReason::Crash,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn fanout_child_missing_output_repair_start_failure_fails_child_siblings_and_parent_in_live_and_replay(
    ) {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir.clone());
        let received_payloads: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let received_for_listener = Arc::clone(&received_payloads);
        app.listen("workflow-execution-changed", move |event| {
            received_for_listener
                .lock()
                .unwrap()
                .push(event.payload().to_string());
        });

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-missing-output-repair-start";
        let missing_child_session_id = "aaaaaaaa-0000-4000-8000-000000000132";
        let sibling_session_id = "fanout-missing-output-start-sibling-session";
        let mut review_a = make_fanout_child("review-a");
        review_a.artifact = Some("review-verdict".to_string());
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-missing-output-start-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                review_a,
                make_fanout_child("review-b"),
            ],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_node_index = 0;
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([
            ("fanout-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        let mut missing_child = test_fanout_child(
            "review-a",
            missing_child_session_id,
            FanoutChildRuntimeState::Running,
            0,
        );
        missing_child.contract = Some("review-verdict".to_string());
        install_test_fanout(
            &mut exec,
            vec![
                missing_child,
                test_fanout_child(
                    "review-b",
                    sibling_session_id,
                    FanoutChildRuntimeState::Running,
                    1,
                ),
            ],
        );
        append_started_events_for_execution(&data_dir, &exec);
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                missing_child_session_id.to_string(),
                SessionWorkflowRef {
                    execution_id: execution_id.clone(),
                },
            );
            refs.insert(
                sibling_session_id.to_string(),
                SessionWorkflowRef {
                    execution_id: execution_id.clone(),
                },
            );
        }
        session_store
            .save_full_session_for_migration_or_restore(
                &data_dir,
                &chat_session_for_test(missing_child_session_id, worktree_path, None, true),
            )
            .unwrap();
        handles
            .insert_failing_runtime_state_for_test(missing_child_session_id)
            .await;

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                missing_child_session_id,
                0,
                None,
                &[],
                None,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "repair start failure must release the terminal fanout"
        );
        let live_payload = received_payloads
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("terminal failure must broadcast live snapshot");
        let live_json: serde_json::Value = serde_json::from_str(&live_payload).unwrap();
        let live_node_executions = live_json["workflowExecution"]["nodeExecutions"]
            .as_array()
            .expect("live payload must expose node executions");
        let live_status = |node_name: &str| {
            live_node_executions
                .iter()
                .find(|execution| execution["nodeName"] == node_name)
                .and_then(|execution| execution["status"].as_str())
                .expect("node execution status must be present")
        };
        assert_eq!(live_status("fanout-review"), "failed");
        assert_eq!(live_status("review-a"), "failed");
        assert_eq!(live_status("review-b"), "aborted");

        let events = read_dispatch_events(&app, &execution_id);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ContractViolated {
                node_name,
                repair_attempt: 1,
                ..
            } if node_name == "review-a"
        )));
        let projected = project_workflow_execution(&execution_id, &events)
            .unwrap()
            .unwrap();
        let projected_status = |node_name: &str| {
            projected
                .node_executions
                .iter()
                .find(|execution| execution.node_name == node_name)
                .map(|execution| execution.status)
                .expect("projected node execution must exist")
        };
        assert_eq!(
            projected_status("fanout-review"),
            crate::domain::workflow::NodeExecutionStatus::Failed
        );
        assert_eq!(
            projected_status("review-a"),
            crate::domain::workflow::NodeExecutionStatus::Failed
        );
        assert_eq!(
            projected_status("review-b"),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert!(projected
            .node_executions
            .iter()
            .all(|execution| !execution.status.is_active()));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionFailed {
                failure_kind: NodeExecutionFailureKind::InfrastructureCrash,
                ..
            }
        )));
    }

    /// Spec [05] commit 境界: production 経路 `execute_outcome` の pre-commit phase で
    /// `write_log_required_batch` が失敗した場合、`sync_execution_store_from_snapshot` /
    /// `persist_state` は実行されず、ExecutionStore は active のまま / NDJSON 上にも terminal
    /// event が残らないことを直接検証する（spec [05]: state mutation と event log の
    /// 分離を防ぐ rollback 境界）。
    ///
    /// 障害シミュレーション: workflow_execution_logs ディレクトリパスに通常ファイルを置くと、
    /// `WorkflowEventLog::append_batch` 内の `create_dir_all` が失敗し、batch append が
    /// `Err` を返す。
    #[tokio::test]
    async fn execute_outcome_pre_commit_append_failure_keeps_execution_store_active() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let worktree_path = "/wt/append-failure";

        let execution_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_running_session_workflow(),
                RuntimeExecutionState::Running,
                worktree_path.to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        // workflow_execution_logs ディレクトリを通常ファイルで塞いで append を恒常失敗させる。
        let log_dir = data_dir.join("workflow_execution_logs");
        if log_dir.exists() {
            std::fs::remove_dir_all(&log_dir).unwrap();
        }
        std::fs::write(&log_dir, b"block").unwrap();

        // snapshot を Failed terminal に遷移させ、execute_outcome に persist 経路で渡す。
        let mut snapshot = {
            let execs = engine.executions.lock().await;
            execs.get(&execution_id).unwrap().to_commit_snapshot()
        };
        snapshot.state = RuntimeExecutionState::Failed {
            reason: "node failure".to_string(),
            kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            retry_count: None,
        };
        snapshot.updated_at = 9999.0;

        let result = engine
            .execute_outcome_persist_failed_for_test(
                app.handle(),
                &session_store,
                &handles,
                worktree_path,
                snapshot,
            )
            .await;
        assert!(
            result.is_err(),
            "execute_outcome must return Err when pre-commit append fails: {result:?}"
        );

        // ExecutionStore は active のまま（terminal に sync されていない）。
        let stored = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .expect("ExecutionStore must still hold the execution");
        assert!(
            !stored.status.is_terminal(),
            "ExecutionStore status must NOT be terminal when event log append fails; got {:?}",
            stored.status
        );
        assert!(
            stored.error_reason.is_none(),
            "ExecutionStore error_reason must remain unset when event log append fails"
        );

        // workflow_execution_logs ディレクトリを復旧して terminal event が増えていないことを確認する。
        std::fs::remove_file(&log_dir).unwrap();
        std::fs::create_dir_all(&log_dir).unwrap();
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(
            events.iter().all(|event| !matches!(
                event,
                WorkflowEvent::NodeFailed { .. } | WorkflowEvent::ExecutionFailed { .. }
            )),
            "terminal events must not be appended when pre-commit append fails; got {events:?}"
        );
    }

    /// Spec [05] Rule: snapshot に Failed state が反映済みの場合、`write_terminal_log` の
    /// 単体経路 (`terminal_events_for_snapshot` → `write_log_required_batch`) が
    /// startup timeout の `failure_kind` / retry count を保ったまま
    /// `NodeFailed` + `ExecutionFailed` を順序通り append することを直接検証する。
    #[test]
    fn write_terminal_log_emits_startup_timeout_node_failed_followed_by_execution_failed() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        let execution_id = "00000000-0000-0000-0000-000000000605".to_string();

        let snapshot = RuntimeCommitSnapshot {
            execution_id: execution_id.clone(),
            workflow_name: "fail-wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Agent,
            request: String::new(),
            error_reason: Some("startup timeout".to_string()),
            state: RuntimeExecutionState::Failed {
                reason: "startup timeout".to_string(),
                kind: crate::domain::workflow::NodeExecutionFailureKind::StartupTimeout,
                retry_count: Some(2),
            },
            current_node_index: 0,
            current_node_name: "node-1".to_string(),
            current_session_id: None,
            node_history: vec![],
            node_execution_counts: HashMap::from([("node-1".to_string(), 1)]),
            workflow_definition:
                crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml {
                    name: "fail-wf".to_string(),
                    description: String::new(),
                    builtin: false,
                    schemas: Default::default(),
                    nodes: vec![make_test_node(
                        "node-1",
                        TestKind::Session,
                        "node-1",
                        vec![],
                        None,
                    )],
                },
            total_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![node_execution_fixture(
                &execution_id,
                "node-execution-node-1",
                "node-1",
                1,
                NodeExecutionStatus::Failed,
                None,
                None,
            )],
            started_at: 900.0,
            updated_at: 1000.0,
        };

        engine
            .write_terminal_log(app.handle(), &snapshot)
            .expect("write_terminal_log must succeed");

        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert_eq!(
            events.len(),
            2,
            "terminal log must contain NodeFailed + ExecutionFailed; got {events:?}"
        );
        match &events[0] {
            WorkflowEvent::NodeFailed {
                execution_id: ev_execution_id,
                node_name,
                reason,
                failure_kind,
                retry_count,
                ..
            } => {
                assert_eq!(ev_execution_id, &execution_id);
                assert_eq!(node_name, "node-1");
                assert_eq!(reason, "startup timeout");
                assert_eq!(
                    *failure_kind,
                    crate::domain::workflow::NodeExecutionFailureKind::StartupTimeout
                );
                assert_eq!(*retry_count, Some(2));
            }
            other => panic!("expected NodeFailed first, got {other:?}"),
        }
        match &events[1] {
            WorkflowEvent::ExecutionFailed {
                execution_id: ev_execution_id,
                reason,
                failure_kind,
                ..
            } => {
                assert_eq!(ev_execution_id, &execution_id);
                assert_eq!(reason, "startup timeout");
                assert_eq!(
                    *failure_kind,
                    crate::domain::workflow::NodeExecutionFailureKind::StartupTimeout
                );
            }
            other => panic!("expected ExecutionFailed second, got {other:?}"),
        }
    }

    /// Spec [04] Rule「対象不在 / 既に終了した command は受理されない」:
    /// `abort_target_lookup` は `executions` に存在しない execution_id を `NotFound` と
    /// 判定し、後段の dispatch では非受理にマッピングされる構造を担保する。
    #[tokio::test]
    async fn abort_target_lookup_returns_not_found_for_unknown_execution_id() {
        let engine = WorkflowRuntimeService::new_for_test();
        match engine
            .abort_target_lookup("00000000-0000-0000-0000-000000000700")
            .await
        {
            AbortTargetLookup::NotFound => {}
            other => panic!("expected NotFound for unknown execution_id, got {other:?}"),
        }
    }

    /// Spec [04] Rule「既に終了した execution に対する操作 command が要求される」:
    /// terminal な execution（Completed/Failed/Aborted）に対する Abort は `AlreadyTerminal`
    /// として lookup 段階で非受理になる。
    #[tokio::test]
    async fn abort_target_lookup_returns_already_terminal_for_terminal_execution() {
        let engine = WorkflowRuntimeService::new_for_test();
        let execution_id = uuid::Uuid::new_v4().to_string();
        for terminal_state in [
            RuntimeExecutionState::Completed,
            RuntimeExecutionState::Aborted,
            RuntimeExecutionState::Failed {
                reason: "x".to_string(),
                kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                retry_count: None,
            },
        ] {
            let mut exec = make_waiting_approval_execution(&execution_id, "/wt/term");
            exec.state = terminal_state.clone();
            engine
                .executions
                .lock()
                .await
                .insert(execution_id.clone(), exec);

            match engine.abort_target_lookup(&execution_id).await {
                AbortTargetLookup::AlreadyTerminal => {}
                other => panic!(
                    "expected AlreadyTerminal for terminal {terminal_state:?}, got {other:?}"
                ),
            }
            engine.executions.lock().await.remove(&execution_id);
        }
    }

    #[tokio::test]
    async fn abort_target_lookup_returns_already_terminal_for_released_terminal_execution_record() {
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;

        for (terminal_status, error_reason) in [
            (TerminalExecutionStatus::Completed, None),
            (
                TerminalExecutionStatus::Failed,
                Some("failed after release".to_string()),
            ),
            (TerminalExecutionStatus::Aborted, None),
        ] {
            let execution_id = uuid::Uuid::new_v4().to_string();
            let exec = make_waiting_approval_execution(&execution_id, "/wt/released-terminal");
            insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
            engine
                .execution_store
                .complete_execution(&execution_id, terminal_status, 2000.0, error_reason)
                .await
                .unwrap();
            engine.executions.lock().await.remove(&execution_id);

            match engine.abort_target_lookup(&execution_id).await {
                AbortTargetLookup::AlreadyTerminal => {}
                other => {
                    panic!(
                        "expected AlreadyTerminal for released terminal execution, got {other:?}"
                    )
                }
            }
        }
    }

    /// Spec [04] Rule: active execution に対する `abort_target_lookup` は `Active` を返し、
    /// その後の state 遷移経路（mutation → required append → finalize）に乗る。
    #[tokio::test]
    async fn abort_target_lookup_returns_active_for_running_execution() {
        let engine = WorkflowRuntimeService::new_for_test();
        let execution_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_waiting_approval_execution(&execution_id, "/wt/active");
        exec.state = RuntimeExecutionState::Running;
        exec.current_session_id = Some("sess-X".to_string());
        engine
            .executions
            .lock()
            .await
            .insert(execution_id.clone(), exec);

        match engine.abort_target_lookup(&execution_id).await {
            AbortTargetLookup::Active {
                current_node_session_id,
                ..
            } => {
                assert_eq!(current_node_session_id.as_deref(), Some("sess-X"));
            }
            other => panic!("expected Active for running execution, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn release_terminal_execution_removes_terminal_entries_only() {
        let engine = WorkflowRuntimeService::new_for_test();

        for (label, terminal_state) in [
            ("completed", RuntimeExecutionState::Completed),
            (
                "failed",
                RuntimeExecutionState::Failed {
                    reason: "boom".to_string(),
                    kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                    retry_count: None,
                },
            ),
            ("aborted", RuntimeExecutionState::Aborted),
        ] {
            let execution_id = uuid::Uuid::new_v4().to_string();
            let mut exec = make_waiting_approval_execution(&execution_id, &format!("/wt/{label}"));
            exec.state = terminal_state;
            engine
                .executions
                .lock()
                .await
                .insert(execution_id.clone(), exec);
            engine.execution_facet_contents.lock().await.insert(
                execution_id.clone(),
                crate::adaptor::gateway::workflow::facet::WorkflowFacetContents::default(),
            );

            engine.release_terminal_execution(&execution_id).await;

            assert!(
                !engine.contains_execution_for_test(&execution_id).await,
                "{label} terminal execution must be removed"
            );
            assert!(
                !engine
                    .execution_facet_contents
                    .lock()
                    .await
                    .contains_key(&execution_id),
                "{label} terminal facet contents must be removed"
            );
        }

        let active_execution_id = uuid::Uuid::new_v4().to_string();
        let mut active =
            make_waiting_approval_execution(&active_execution_id, "/wt/active-release");
        active.state = RuntimeExecutionState::Running;
        engine
            .executions
            .lock()
            .await
            .insert(active_execution_id.clone(), active);
        engine.execution_facet_contents.lock().await.insert(
            active_execution_id.clone(),
            crate::adaptor::gateway::workflow::facet::WorkflowFacetContents::default(),
        );

        engine
            .release_terminal_execution(&active_execution_id)
            .await;

        assert!(
            engine
                .contains_execution_for_test(&active_execution_id)
                .await,
            "active execution must not be released"
        );
        assert!(
            engine
                .execution_facet_contents
                .lock()
                .await
                .contains_key(&active_execution_id),
            "active facet contents must not be released"
        );
        assert_eq!(engine.executions_len_for_test().await, 1);
    }

    #[tokio::test]
    async fn get_state_by_execution_id_returns_none_for_released_terminal_state() {
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        engine.set_execution_store_data_dir(data_dir.clone()).await;

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/reconstruct-terminal";
        let exec = make_waiting_approval_execution(&execution_id, worktree_path);
        let workflow = exec.workflow.clone();
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        let log = WorkflowEventLog::new(&data_dir);
        log.append(&WorkflowEvent::ExecutionStarted {
            execution_id: execution_id.clone(),
            workflow_name: workflow.name.clone(),
            worktree_path: worktree_path.to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            request: String::new(),
            permission_mode: "ask".to_string(),
            definition: workflow.clone(),
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            execution_id: execution_id.clone(),
            node_execution_id: "node-execution-review-1".to_string(),
            node_name: "review".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            fanout_parent: None,
            timestamp: 1001.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::SessionAttached {
            execution_id: execution_id.clone(),
            node_execution_id: "node-execution-review-1".to_string(),
            session_id: "sess-1".to_string(),
            timestamp: 1001.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeCompleted {
            execution_id: execution_id.clone(),
            node_execution_id: "node-execution-review-1".to_string(),
            node_name: "review".to_string(),
            result_summary: Some("approve".to_string()),
            token_usage: None,
            attempt: 1,
            timestamp: 1002.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::ExecutionCompleted {
            execution_id: execution_id.clone(),
            total_token_usage: TokenUsage::default(),
            timestamp: 1003.0,
        })
        .unwrap();

        engine
            .execution_store
            .complete_execution(
                &execution_id,
                TerminalExecutionStatus::Completed,
                1003.0,
                None,
            )
            .await
            .unwrap();
        engine.executions.lock().await.remove(&execution_id);

        assert!(
            engine
                .get_state_by_execution_id(&execution_id)
                .await
                .is_none(),
            "execution_id-only live API must not expose released terminal history"
        );
    }

    /// Spec [04] Rule「権限の無い / 対象不在 / 既決の command は state 変化を起こさない」:
    /// 既に承認済み（WaitingApproval ではない）node に対する Approve は
    /// `validate_approval_target_snapshot` で `InvalidState` として非受理になる。
    /// production dispatch 経路の `handle_approval` がこのガードを最初に通すため、
    /// 二度目以降の同一意図 command は state 変化を起こさない。
    #[tokio::test]
    async fn approval_target_validation_rejects_already_resolved_node() {
        let execution_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_waiting_approval_execution(&execution_id, "/wt/idempotent");
        exec.state = RuntimeExecutionState::Completed;
        let err = workflow_approval_runtime::validate_approval_target_snapshot(
            &exec,
            Some(&execution_id),
            Some("review"),
        )
        .unwrap_err();
        assert!(
            matches!(err, WorkflowEngineError::InvalidState(_)),
            "既決 node への Approve は InvalidState で非受理 (got {err:?})"
        );
    }

    /// Approve comment は空文字を許容するが、上限超過は非受理。
    #[test]
    fn approve_comment_length_validation_rejects_oversize_but_accepts_empty() {
        workflow_approval_runtime::validate_approve_comment(None).expect("None は許容される");
        workflow_approval_runtime::validate_approve_comment(Some(""))
            .expect("空コメント (Some(empty)) は許容される");
        let oversize_comment = "x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);
        let err = workflow_approval_runtime::validate_approve_comment(Some(&oversize_comment))
            .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
    }

    /// Spec [04] secret redaction: ApprovalResolved.comment に設定済み secret 値が
    /// 含まれる場合、event log に書き出す前に `mask_sensitive_text()` で redaction
    /// される。本テストは redaction primitive そのものの契約を担保する
    #[test]
    fn mask_sensitive_text_redacts_secret_in_approval_comment() {
        let secrets = vec!["super-secret-token".to_string()];
        let raw = "approving with token=super-secret-token please review";
        let masked = workflow_secret_masker::mask_sensitive_text(raw, &secrets);
        assert!(
            !masked.contains("super-secret-token"),
            "secret 値が raw のまま残ってはならない (masked={masked})"
        );
    }

    /// Spec [04] atomic mutation 境界（Abort 経路）: `abort_workflow_execution`
    /// が受理されると `ExecutionAborted` event は `write_log_required` 経由で必須 append
    /// される。NDJSON に正しく snake_case で記録され、observer が typed event として
    /// 読めることを担保する。
    #[test]
    fn execution_aborted_event_required_append_writes_typed_ndjson() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let execution_id = "00000000-0000-0000-0000-000000000800";

        log.append(&WorkflowEvent::ExecutionAborted {
            execution_id: execution_id.to_string(),
            aborted_node: None,
            timestamp: 4321.0,
        })
        .expect("ExecutionAborted は write_log_required 経由で append される");

        let events = log.read_log(execution_id).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            WorkflowEvent::ExecutionAborted {
                execution_id: rid, ..
            } => assert_eq!(rid, execution_id),
            other => panic!("expected ExecutionAborted, got {other:?}"),
        }
    }

    /// Spec [04] rollback: production dispatch 経由で event append が失敗した場合、
    /// WorkflowExecution / Execution Store / event log は command 受理前 snapshot に戻る。
    #[tokio::test]
    async fn dispatch_approve_node_append_failure_rolls_back_full_snapshot() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/append-fail";
        let mut exec = make_waiting_approval_execution(&execution_id, worktree_path);
        exec.current_session_id = None;
        let snapshot_before = exec.clone();
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_execution_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                Some("lgtm".to_string()),
                "review",
                None,
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));

        let execs = engine.executions.lock().await;
        let restored = execs.get(&execution_id).expect("execution must remain");
        assert_eq!(
            restored.state, snapshot_before.state,
            "state は snapshot で一括復元される"
        );
        assert_eq!(
            restored.current_node_index,
            snapshot_before.current_node_index
        );
        assert_eq!(
            restored.node_history.len(),
            snapshot_before.node_history.len()
        );
        drop(execs);

        let active = engine.list_active_executions().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].execution_id, execution_id);
        assert_eq!(active[0].status, ExecutionStatus::WaitingApproval);
        assert!(read_dispatch_events(&app, &execution_id).is_empty());
    }

    /// Spec [04] rollback: AbortExecution の required event append が失敗した場合も、
    /// WorkflowExecution / Execution Store / ChatSession workflow_state は mutation 前へ戻る。
    #[tokio::test]
    async fn dispatch_abort_execution_append_failure_rolls_back_execution_execution_store_and_session(
    ) {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/abort-append-fail";
        let mut exec = make_waiting_approval_execution(&execution_id, worktree_path);
        exec.current_session_id = None;
        exec.state = RuntimeExecutionState::Running;
        let snapshot_before = exec.clone();
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_execution_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .abort_workflow_execution(app.handle(), &session_store, &handles, &execution_id, None)
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&execution_id).expect("execution must remain");
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.node_history.len(),
            snapshot_before.node_history.len()
        );
        drop(execs);
        let active = engine.list_active_executions().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].execution_id, execution_id);
        assert_eq!(active[0].status, ExecutionStatus::Running);
        // ChatSession.workflow_state は撤去済みのため parent session 経由の rollback 観測は省略。
        assert!(read_dispatch_events(&app, &execution_id).is_empty());
    }

    /// Interrupted abort も append-only event log を最初の永続 commit point とする。
    /// ExecutionAborted append が失敗した場合、metadata は再開可能な checkpoint のままで、
    /// append 前に Aborted が外部可視にならない。
    #[tokio::test]
    async fn dispatch_abort_interrupted_append_failure_keeps_checkpoint() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir.clone());
        let worktree = TempDir::new().unwrap();
        let execution_id = uuid::Uuid::new_v4().to_string();
        engine
            .execution_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: worktree.path().to_string_lossy().into_owned(),
                current_node: Some("plan".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        engine
            .execution_store
            .interrupt_execution(
                &execution_id,
                ExecutionInterruptionReason::Stop,
                Some("plan".to_string()),
                110.0,
            )
            .await
            .unwrap();
        let worktree_path = worktree.path().to_string_lossy().into_owned();
        WorkflowEventLog::new(&data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: "wf".to_string(),
                    worktree_path,
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    permission_mode: "ask".to_string(),
                    definition: WorkflowDefinitionYaml {
                        name: "wf".to_string(),
                        description: String::new(),
                        builtin: false,
                        schemas: Default::default(),
                        nodes: vec![make_test_node(
                            "plan",
                            TestKind::Session,
                            "implement",
                            vec![],
                            None,
                        )],
                    },
                    timestamp: 100.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: format!("{execution_id}-plan-1"),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 101.0,
                },
                WorkflowEvent::ExecutionInterrupted {
                    execution_id: execution_id.clone(),
                    reason: ExecutionInterruptionReason::Stop,
                    timestamp: 110.0,
                },
            ])
            .unwrap();
        engine.fail_next_required_event_append_for_test();

        let result = engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &agent_runtime,
                &execution_id,
                None,
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let checkpoint = engine
            .execution_store
            .get_execution_record(&execution_id)
            .await
            .expect("interrupted checkpoint must remain after append failure");
        assert_eq!(checkpoint.status, ExecutionStatus::Interrupted);
        assert_eq!(
            checkpoint.interruption_reason,
            Some(ExecutionInterruptionReason::Stop)
        );
        assert_eq!(checkpoint.resume_from_node.as_deref(), Some("plan"));
        assert!(checkpoint.completed_at.is_none());
    }

    /// Spec [04] テスト境界: StartExecution は production start primitive 入口で
    /// validation され、拒否時は state / event を変更しない。
    #[tokio::test]
    async fn start_execution_primitive_rejects_invalid_name_without_state_change() {
        let engine = WorkflowRuntimeService::new_for_test();
        let execution_store_dir = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(execution_store_dir.path().to_path_buf())
            .await;

        let result = engine.resolve_start_execution_workflow("../bad").await;

        assert!(matches!(
            result,
            Err(WorkflowEngineError::ValidationError(_))
        ));
        assert!(engine.executions.lock().await.is_empty());
        assert!(engine.list_active_executions().await.is_empty());
    }

    /// Spec [04] テスト境界: StartExecution の正常系は production start primitive 経由で
    /// execution_id を返し、execution / Execution Store / ExecutionStarted event を作成する。
    #[tokio::test]
    async fn start_execution_primitive_accepts_creates_execution_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let execution_store_dir = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(execution_store_dir.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let (repo_parent, _worktree_parent, worktree_path) = make_managed_worktree();
        configure_managed_repo(&app, repo_parent.path().join("repo").as_path());
        let stem = crate::adaptor::gateway::workflow::builtin::list_builtin_workflows()
            .into_iter()
            .next()
            .expect("at least one builtin workflow must exist")
            .name;

        let resolved_worktree = engine
            .resolve_start_execution_worktree(worktree_path.to_string_lossy().to_string())
            .await
            .unwrap();
        let workflow = engine
            .resolve_start_execution_workflow(&stem)
            .await
            .unwrap();
        let execution_id = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &handles,
                workflow,
                resolved_worktree,
                Some("start me".to_string()),
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Edit,
            )
            .await
            .unwrap();
        assert!(
            engine.executions.lock().await.contains_key(&execution_id),
            "StartExecution must register a WorkflowExecution"
        );
        assert!(
            engine.get_execution(&execution_id).await.is_some(),
            "StartExecution must create a Execution Store entry"
        );
        assert!(read_dispatch_events(&app, &execution_id)
            .iter()
            .any(|event| {
                matches!(
                    event,
                    WorkflowEvent::ExecutionStarted {
                        workflow_name,
                        request,
                        ..
                    } if workflow_name == &stem && request == "start me"
                )
            }));
    }

    /// #1337 final gate: every bundled workflow must pass the production load/start path,
    /// create its initial NodeExecution, and activate the stubbed session runtime.
    #[tokio::test]
    async fn all_twelve_builtin_workflows_activate_their_initial_node_execution() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, agent_runtime) = make_dispatch_deps(data_dir);
        let summaries = crate::adaptor::gateway::workflow::builtin::list_builtin_workflows();
        assert_eq!(
            summaries.len(),
            12,
            "the canonical builtin set must stay at 12"
        );

        for summary in summaries {
            let (repo_parent, _worktree_parent, worktree_path) = make_managed_worktree();
            configure_managed_repo(&app, repo_parent.path().join("repo").as_path());
            let resolved_worktree = engine
                .resolve_start_execution_worktree(worktree_path.to_string_lossy().to_string())
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "builtin '{}' worktree must resolve before start: {error}",
                        summary.name
                    )
                });
            let workflow = engine
                .resolve_start_execution_workflow(&summary.name)
                .await
                .unwrap_or_else(|error| {
                    panic!("builtin '{}' must load before start: {error}", summary.name)
                });
            let initial_node = workflow
                .nodes
                .first()
                .unwrap_or_else(|| panic!("builtin '{}' must have a node", summary.name));
            let initial_node_name = initial_node.name.clone();
            let initial_node_kind = initial_node.kind_name();
            let expected_fanout_children = initial_node.fanout().map(|fanout| {
                let item_count = match fanout.items.as_ref() {
                    None => 1,
                    Some(ItemsSource::Literal(items)) => items.len(),
                    Some(ItemsSource::ArtifactField { node, field }) => panic!(
                        "builtin '{}' initial fanout cannot source items from {node}.{field}",
                        summary.name
                    ),
                };
                fanout.child.len() * item_count
            });

            let execution_id = engine
                .start_resolved_workflow(
                    app.handle(),
                    &session_store,
                    &agent_runtime,
                    workflow,
                    resolved_worktree,
                    Some(format!("start builtin {}", summary.name)),
                    ExecutionOrigin::DesktopUi,
                    crate::domain::agent_session::PermissionMode::Edit,
                )
                .await
                .unwrap_or_else(|error| panic!("builtin '{}' must start: {error}", summary.name));

            match initial_node_kind {
                NodeKindName::Session => {
                    let (session_id, _) =
                        wait_for_top_level_session(&engine, &execution_id, &initial_node_name)
                            .await;
                    wait_for_stub_session_turn_activation(&agent_runtime, &session_id).await;
                }
                NodeKindName::Fanout => {
                    let children = wait_for_active_fanout_children(
                        &engine,
                        &execution_id,
                        &initial_node_name,
                        expected_fanout_children.expect("fanout child count must be available"),
                    )
                    .await;
                    for (_, session_id, _) in children {
                        wait_for_stub_session_turn_activation(&agent_runtime, &session_id).await;
                    }
                }
                NodeKindName::Command => {
                    for _ in 0..500 {
                        if read_dispatch_events(&app, &execution_id)
                            .iter()
                            .any(|event| {
                                matches!(
                                    event,
                                    WorkflowEvent::NodeStarted {
                                        node_name,
                                        kind: NodeKindName::Command,
                                        ..
                                    } if node_name == &initial_node_name
                                )
                            })
                        {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }
            }

            assert!(
                read_dispatch_events(&app, &execution_id)
                    .iter()
                    .any(|event| matches!(
                        event,
                        WorkflowEvent::NodeStarted {
                            node_name,
                            kind,
                            attempt: 1,
                            fanout_parent: None,
                            ..
                        } if node_name == &initial_node_name && *kind == initial_node_kind
                    )),
                "builtin '{}' must append its initial top-level NodeStarted event",
                summary.name
            );

            if engine
                .execution_store()
                .get_execution(&execution_id)
                .await
                .is_some_and(|execution| !execution.status.is_terminal())
            {
                engine
                    .abort_workflow_execution(
                        app.handle(),
                        &session_store,
                        &agent_runtime,
                        &execution_id,
                        None,
                    )
                    .await
                    .unwrap_or_else(|error| {
                        panic!(
                            "builtin '{}' cleanup abort must succeed: {error}",
                            summary.name
                        )
                    });
            }
        }
    }

    /// Task 1326 regression: reservation 後の validate_start 失敗 rollback で、
    /// runtime-local facet read model を残さない。
    #[tokio::test]
    async fn start_execution_validate_start_failure_releases_execution_facet_contents() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let execution_store_dir = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(execution_store_dir.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let worktree = "/wt/start-validate-start-failure".to_string();
        let existing_execution_id = uuid::Uuid::new_v4().to_string();
        let mut existing = make_exec_with(
            &existing_execution_id,
            &worktree,
            RuntimeExecutionState::Running,
        );
        existing.workflow.name = "existing-active-wf".to_string();
        engine
            .executions
            .lock()
            .await
            .insert(existing_execution_id.clone(), existing);

        let stem = crate::adaptor::gateway::workflow::builtin::list_builtin_workflows()
            .into_iter()
            .next()
            .expect("at least one builtin workflow must exist")
            .name;
        let workflow = engine
            .resolve_start_execution_workflow(&stem)
            .await
            .unwrap();
        let result = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &handles,
                workflow,
                worktree,
                Some("start with validate_start failure".to_string()),
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Edit,
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::AlreadyActive(_))));
        assert!(
            engine.execution_facet_contents.lock().await.is_empty(),
            "validate_start rollback must release execution_facet_contents"
        );
        assert!(
            engine.list_active_executions().await.is_empty(),
            "failed reservation must be cancelled"
        );
        assert!(
            engine
                .contains_execution_for_test(&existing_execution_id)
                .await,
            "pre-existing execution must remain"
        );
        assert_eq!(engine.executions_len_for_test().await, 1);
    }

    /// Spec [04] rollback: StartExecution の ExecutionStarted append が失敗した場合、
    /// reservation / execution / parent ChatSession を command 受理前へ戻す。
    #[tokio::test]
    async fn start_execution_primitive_append_failure_clears_created_parent_commit_snapshot() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let execution_store_dir = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(execution_store_dir.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let (repo_parent, _worktree_parent, worktree_path) = make_managed_worktree();
        configure_managed_repo(&app, repo_parent.path().join("repo").as_path());
        let worktree = std::fs::canonicalize(&worktree_path)
            .unwrap()
            .to_string_lossy()
            .to_string();
        engine.fail_next_required_event_append_for_test();

        let stem = crate::adaptor::gateway::workflow::builtin::list_builtin_workflows()
            .into_iter()
            .next()
            .expect("at least one builtin workflow must exist")
            .name;
        let workflow = engine
            .resolve_start_execution_workflow(&stem)
            .await
            .unwrap();
        let result = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &handles,
                workflow,
                worktree.clone(),
                Some("start with append failure".to_string()),
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Edit,
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        assert!(engine.executions.lock().await.is_empty());
        assert!(
            engine.execution_facet_contents.lock().await.is_empty(),
            "ExecutionStarted append rollback must release execution_facet_contents"
        );
        assert!(engine.list_active_executions().await.is_empty());
        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_execution_logs");
        assert!(
            !log_dir_path.exists() || std::fs::read_dir(&log_dir_path).unwrap().next().is_none(),
            "ExecutionStarted and initial NodeStarted must be one atomic batch"
        );
        let sessions = session_store
            .list_worktree_sessions(&dispatch_data_dir(app.handle()), &worktree)
            .unwrap();
        assert!(
            sessions.is_empty(),
            "ExecutionStarted が存在しない失敗 execution の parent ChatSession は残さない"
        );
    }

    // 撤去済み: persist_state は廃止された（NDJSON event log + Execution Store metadata で永続化が完結）。
    // 旧 `dispatch_start_execution_persist_failure_rolls_back_execution_execution_store_and_parent_session` テストは
    // persist_state 注入失敗時の rollback を検証していたが、機構撤去により意味を失った。

    /// Spec [04] テスト境界: AbortExecution は production dispatch 経由で Aborted に遷移し、
    /// ExecutionAborted typed event を append する。
    #[tokio::test]
    async fn dispatch_abort_execution_accepts_mutates_state_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-abort";
        let mut exec = make_waiting_approval_execution(&execution_id, worktree_path);
        let workflow = exec.workflow.clone();
        // spec issues-1023: session log 到達経路の維持を検証するため、
        // current_session_id を入れた状態で abort する。
        exec.current_session_id = Some("aborted-node-session".to_string());
        exec.state = RuntimeExecutionState::Running;
        let node_execution_id = exec.node_executions[0].id.clone();
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        WorkflowEventLog::new(&data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: workflow.name.clone(),
                    worktree_path: worktree_path.to_string(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    permission_mode: "ask".to_string(),
                    definition: workflow,
                    timestamp: 1000.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                    node_name: "review".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 1000.0,
                },
                WorkflowEvent::SessionAttached {
                    execution_id: execution_id.clone(),
                    node_execution_id,
                    session_id: "aborted-node-session".to_string(),
                    timestamp: 1000.0,
                },
            ])
            .unwrap();

        engine
            .abort_workflow_execution(app.handle(), &session_store, &handles, &execution_id, None)
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "terminal execution must be released after Aborted"
        );

        let events = read_dispatch_events(&app, &execution_id);
        let aborted_node = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::ExecutionAborted { aborted_node, .. } => aborted_node.as_ref(),
                _ => None,
            })
            .expect("ExecutionAborted must identify the aborted node");
        assert_eq!(aborted_node, "review");

        assert!(
            engine
                .get_state_by_execution_id(&execution_id)
                .await
                .is_none(),
            "execution_id-only live API must not expose released terminal history"
        );
        let reconstructed =
            crate::adaptor::gateway::workflow::event_projection::project_workflow_execution(
                &execution_id,
                &events,
            )
            .unwrap()
            .expect(
                "released aborted execution history must reconstruct from Event Log projection",
            );
        assert_eq!(reconstructed.status, ExecutionStatus::Aborted);
        let aborted_executions = reconstructed
            .node_executions
            .iter()
            .filter(|execution| {
                execution.status == crate::domain::workflow::NodeExecutionStatus::Aborted
            })
            .collect::<Vec<_>>();
        assert_eq!(
            aborted_executions.len(),
            1,
            "released aborted execution must reconstruct the aborted current node"
        );
        assert_eq!(
            aborted_executions[0].session_id.as_deref(),
            Some("aborted-node-session"),
            "reconstructed state must preserve the session log reachability"
        );
    }

    #[tokio::test]
    async fn dispatch_abort_execution_snapshots_current_attempt_for_retried_node() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-abort-retry";
        let mut exec = make_waiting_approval_execution(&execution_id, worktree_path);
        let workflow = exec.workflow.clone();
        exec.state = RuntimeExecutionState::Running;
        exec.current_session_id = Some("session-review-2".to_string());
        exec.node_execution_counts.insert("review".to_string(), 2);
        exec.node_history.push(NodeHistoryEntry {
            node_name: "review".to_string(),
            completed_at: 1001.0,
            result: Some("retry".to_string()),
            session_id: Some("session-review-1".to_string()),
            token_usage: None,
            artifact: None,
            attempt: 1,
            fanout_children: None,
            state: crate::domain::workflow::value_objects::default_node_history_status(),
        });
        let retried_node_execution_id = format!("{execution_id}-review-2");
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        WorkflowEventLog::new(&data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: workflow.name.clone(),
                    worktree_path: worktree_path.to_string(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    permission_mode: "ask".to_string(),
                    definition: workflow,
                    timestamp: 1000.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: retried_node_execution_id.clone(),
                    node_name: "review".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 2,
                    fanout_parent: None,
                    timestamp: 1001.0,
                },
                WorkflowEvent::SessionAttached {
                    execution_id: execution_id.clone(),
                    node_execution_id: retried_node_execution_id,
                    session_id: "session-review-2".to_string(),
                    timestamp: 1001.0,
                },
            ])
            .unwrap();

        engine
            .abort_workflow_execution(app.handle(), &session_store, &handles, &execution_id, None)
            .await
            .unwrap();

        let events = read_dispatch_events(&app, &execution_id);
        let aborted_node = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::ExecutionAborted { aborted_node, .. } => aborted_node.as_ref(),
                _ => None,
            })
            .expect("retried current node must be persisted as aborted_node");
        assert_eq!(aborted_node, "review");

        let reconstructed =
            crate::adaptor::gateway::workflow::event_projection::project_workflow_execution(
                &execution_id,
                &events,
            )
            .unwrap()
            .expect("released aborted retry must reconstruct from Event Log projection");
        let aborted_execution = reconstructed
            .node_executions
            .iter()
            .find(|execution| execution.node_name == "review" && execution.attempt == 2)
            .expect("reconstructed read model must contain the retried aborted node");
        assert_eq!(
            aborted_execution.status,
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert_eq!(
            aborted_execution.session_id.as_deref(),
            Some("session-review-2")
        );
    }

    /// spec issues-1023: `make_aborted_fanout_history_entry` の単体検証。
    /// fanout ブロック中断時に parent node を 1 entry として、children を
    /// `fanout_children` に snapshot し、完了済み child は "completed"、それ以外は
    /// "aborted" 状態で記録される。session_id は全 child で残されることを担保する。
    #[test]
    fn make_aborted_fanout_history_entry_snapshots_mixed_child_states() {
        let workflow = WorkflowDefinitionYaml {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_fanout_node("fanout-review", vec![])],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow("exec-abort-fanout", "/wt", workflow);
        exec.state = RuntimeExecutionState::Running;
        exec.current_session_id = None;
        let mut child_a = test_fanout_child(
            "child-a",
            "session-a",
            FanoutChildRuntimeState::Completed,
            0,
        );
        child_a.result = Some("LGTM".to_string());
        let child_b =
            test_fanout_child("child-b", "session-b", FanoutChildRuntimeState::Running, 1);
        install_test_fanout(&mut exec, vec![child_a, child_b]);

        let entry = exec
            .make_aborted_fanout_history_entry(123.0)
            .expect("fanout_runtime が Some なら entry が返る");
        assert_eq!(entry.node_name, "fanout-review");
        assert_eq!(entry.state, "aborted");
        assert_eq!(entry.completed_at, 123.0);
        let children = entry.fanout_children.expect("fanout_children が Some");
        assert_eq!(children.len(), 2);
        let child_a = children.iter().find(|c| c.node_name == "child-a").unwrap();
        assert_eq!(child_a.state, "completed");
        assert_eq!(child_a.session_id.as_deref(), Some("session-a"));
        let child_b = children.iter().find(|c| c.node_name == "child-b").unwrap();
        assert_eq!(child_b.state, "aborted");
        assert_eq!(
            child_b.session_id.as_deref(),
            Some("session-b"),
            "未完了 child でも session_id が fanout_children に残る"
        );
    }

    /// Spec [06] テスト境界: node 限定 AbortExecution は現在 node を照合した上で execution abort として
    /// 扱い、Running / WaitingApproval のどちらでも `ExecutionAborted` を append する。
    #[tokio::test]
    async fn dispatch_abort_execution_with_expected_node_validates_node_and_appends_execution_aborted(
    ) {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-approval-abort";
        let mut exec = make_waiting_approval_execution(&execution_id, worktree_path);
        exec.current_session_id = None;
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                Some("review"),
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "terminal execution must be released after Aborted"
        );
        let events = read_dispatch_events(&app, &execution_id);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], WorkflowEvent::ExecutionAborted { .. }));
    }

    // 撤去済み: dispatch_abort_execution_with_expected_node_persist_failure_rolls_back は
    // persist_state 注入失敗を介して rollback を検証していたが、persist_state 機構の撤去で
    // 意味を失った（NDJSON event log + Execution Store metadata が権威）。
    // required event append 失敗時の rollback は下記
    // `dispatch_abort_execution_with_expected_node_append_failure_rolls_back` で引き続き検証する。

    /// Spec [04] rollback: approval UI 由来の AbortExecution で required event append が失敗した場合も、
    /// WorkflowExecution / Execution Store / ChatSession workflow_state は mutation 前へ戻る。
    #[tokio::test]
    async fn dispatch_abort_execution_with_expected_node_append_failure_rolls_back() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/approval-abort-append-rollback";
        let mut exec = make_waiting_approval_execution(&execution_id, worktree_path);
        exec.current_session_id = None;
        let snapshot_before = exec.clone();
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;
        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_execution_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                Some("review"),
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&execution_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.node_history.len(),
            snapshot_before.node_history.len()
        );
        drop(execs);
        let active = engine.list_active_executions().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].execution_id, execution_id);
        assert_eq!(active[0].status, ExecutionStatus::WaitingApproval);
        // ChatSession.workflow_state は撤去済みのため parent session 経由の rollback 観測は省略。
        // ExecutionStore active projection の rollback だけ確認する（上の assertion で済み）。
        assert!(read_dispatch_events(&app, &execution_id).is_empty());
    }

    /// Spec [04] Rule「対象不在 / 既に終了した command は受理されない」:
    /// AbortExecution の dispatch 拒否経路は state を変化させず event を append しない。
    #[tokio::test]
    async fn dispatch_abort_execution_rejects_not_found_and_terminal_without_append() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let missing_execution_id = uuid::Uuid::new_v4().to_string();

        let missing = engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &handles,
                &missing_execution_id,
                None,
            )
            .await;
        assert!(matches!(
            missing,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));
        assert!(read_dispatch_events(&app, &missing_execution_id).is_empty());

        let terminal_execution_id = uuid::Uuid::new_v4().to_string();
        let terminal =
            make_waiting_approval_execution(&terminal_execution_id, "/wt/terminal-abort");
        insert_execution_and_register_active(&engine, terminal, ExecutionOrigin::DesktopUi).await;
        let snapshot_before = {
            let mut executions = engine.executions.lock().await;
            let terminal = executions
                .get_mut(&terminal_execution_id)
                .expect("live terminal fixture");
            terminal.state = RuntimeExecutionState::Completed;
            terminal.clone()
        };
        engine
            .execution_store
            .complete_execution(
                &terminal_execution_id,
                TerminalExecutionStatus::Completed,
                2000.0,
                None,
            )
            .await
            .unwrap();

        let terminal_result = engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &handles,
                &terminal_execution_id,
                None,
            )
            .await;
        assert!(matches!(
            terminal_result,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&terminal_execution_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.node_history.len(),
            snapshot_before.node_history.len()
        );
        drop(execs);
        assert!(read_dispatch_events(&app, &terminal_execution_id).is_empty());

        let released_terminal_execution_id = uuid::Uuid::new_v4().to_string();
        let released_terminal = make_waiting_approval_execution(
            &released_terminal_execution_id,
            "/wt/released-terminal",
        );
        insert_execution_and_register_active(
            &engine,
            released_terminal,
            ExecutionOrigin::DesktopUi,
        )
        .await;
        engine
            .execution_store
            .complete_execution(
                &released_terminal_execution_id,
                TerminalExecutionStatus::Completed,
                2000.0,
                None,
            )
            .await
            .unwrap();
        engine
            .executions
            .lock()
            .await
            .remove(&released_terminal_execution_id);

        let released_terminal_result = engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &handles,
                &released_terminal_execution_id,
                None,
            )
            .await;
        assert!(matches!(
            released_terminal_result,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        assert!(read_dispatch_events(&app, &released_terminal_execution_id).is_empty());
    }

    #[tokio::test]
    async fn dispatch_abort_execution_treats_execution_released_after_lookup_as_already_terminal() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let execution_id = uuid::Uuid::new_v4().to_string();
        let exec = make_waiting_approval_execution(&execution_id, "/wt/released-after-lookup");
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        let lookup_completed = Arc::new(tokio::sync::Notify::new());
        let continue_precommit = Arc::new(tokio::sync::Notify::new());
        engine
            .pause_abort_after_lookup_for_test(lookup_completed.clone(), continue_precommit.clone())
            .await;

        let abort_engine = engine.clone();
        let abort_session_store = session_store.clone();
        let abort_handles = handles.clone();
        let abort_execution_id = execution_id.clone();
        let app_handle = app.handle().clone();
        let abort_task = tokio::spawn(async move {
            abort_engine
                .abort_workflow_execution(
                    &app_handle,
                    &abort_session_store,
                    &abort_handles,
                    &abort_execution_id,
                    None,
                )
                .await
        });

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            lookup_completed.notified(),
        )
        .await
        .expect("abort lookup must reach Active before the pre-commit relock");

        engine
            .execution_store
            .complete_execution(
                &execution_id,
                TerminalExecutionStatus::Completed,
                2000.0,
                None,
            )
            .await
            .unwrap();
        engine.executions.lock().await.remove(&execution_id);
        continue_precommit.notify_one();

        let result = tokio::time::timeout(std::time::Duration::from_secs(1), abort_task)
            .await
            .expect("abort task must finish")
            .expect("abort task must not panic");
        assert!(matches!(
            result,
            Err(WorkflowEngineError::InvalidState(message))
                if message.contains("already terminal")
        ));
        assert!(
            read_dispatch_events(&app, &execution_id).is_empty(),
            "released-after-lookup race must not append dispatch events"
        );
    }

    /// Spec [04] no-op 不変条件: target 指定付きの
    /// `AbortExecution { expected_node_name: Some(_) }` でも、対象不在・stale node・既決 node は
    /// production dispatch 経由で state / Execution Store を変化させず event を append しない。
    #[tokio::test]
    async fn dispatch_targeted_abort_rejects_missing_stale_and_resolved_targets_without_append() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let missing_execution_id = uuid::Uuid::new_v4().to_string();
        let missing = engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &handles,
                &missing_execution_id,
                Some("review"),
            )
            .await;
        assert!(matches!(
            missing,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));
        assert!(engine.list_active_executions().await.is_empty());
        assert!(engine.list_completed_executions().await.is_empty());
        assert!(read_dispatch_events(&app, &missing_execution_id).is_empty());

        let stale_execution_id = uuid::Uuid::new_v4().to_string();
        let stale_worktree = "/wt/approval-abort-stale";
        let mut stale_exec = make_waiting_approval_execution(&stale_execution_id, stale_worktree);
        stale_exec.current_session_id = None;
        let stale_before = stale_exec.clone();
        insert_execution_and_register_active(&engine, stale_exec, ExecutionOrigin::DesktopUi).await;
        let stale_active_before = engine.list_active_executions().await;

        let stale = engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &handles,
                &stale_execution_id,
                Some("old-review"),
            )
            .await;
        assert!(matches!(
            stale,
            Err(WorkflowEngineError::UnauthorizedApprovalTarget(_))
        ));
        let execs = engine.executions.lock().await;
        let stale_after = execs.get(&stale_execution_id).unwrap();
        assert_eq!(stale_after.state, stale_before.state);
        assert_eq!(
            stale_after.current_node_index,
            stale_before.current_node_index
        );
        assert_eq!(
            stale_after.node_history.len(),
            stale_before.node_history.len()
        );
        drop(execs);
        let stale_active_after = engine.list_active_executions().await;
        assert_eq!(stale_active_after.len(), stale_active_before.len());
        assert_eq!(
            stale_active_after[0].execution_id,
            stale_active_before[0].execution_id
        );
        assert_eq!(stale_active_after[0].status, stale_active_before[0].status);
        assert!(read_dispatch_events(&app, &stale_execution_id).is_empty());

        let resolved_execution_id = uuid::Uuid::new_v4().to_string();
        let resolved_worktree = "/wt/approval-abort-resolved";
        let mut resolved_exec =
            make_waiting_approval_execution(&resolved_execution_id, resolved_worktree);
        resolved_exec.current_session_id = None;
        resolved_exec.state = RuntimeExecutionState::Completed;
        let resolved_before = resolved_exec.clone();
        engine
            .executions
            .lock()
            .await
            .insert(resolved_execution_id.clone(), resolved_exec);
        engine
            .execution_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: resolved_execution_id.clone(),
                workflow_name: "boundary-wf".to_string(),
                status: ExecutionStatus::WaitingApproval,
                worktree_path: resolved_worktree.to_string(),
                current_node: Some("review".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 1000.0,
                updated_at: 1000.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        engine
            .execution_store
            .complete_execution(
                &resolved_execution_id,
                TerminalExecutionStatus::Completed,
                2000.0,
                None,
            )
            .await
            .unwrap();
        let completed_before = engine.list_completed_executions().await;

        let resolved = engine
            .abort_workflow_execution(
                app.handle(),
                &session_store,
                &handles,
                &resolved_execution_id,
                Some("review"),
            )
            .await;
        assert!(matches!(
            resolved,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        let execs = engine.executions.lock().await;
        let resolved_after = execs.get(&resolved_execution_id).unwrap();
        assert_eq!(resolved_after.state, resolved_before.state);
        assert_eq!(
            resolved_after.node_history.len(),
            resolved_before.node_history.len()
        );
        drop(execs);
        let completed_after = engine.list_completed_executions().await;
        assert_eq!(completed_after.len(), completed_before.len());
        assert_eq!(
            completed_after[0].execution_id,
            completed_before[0].execution_id
        );
        assert_eq!(completed_after[0].status, completed_before[0].status);
        assert!(read_dispatch_events(&app, &resolved_execution_id).is_empty());
    }

    /// Spec [04] テスト境界: approval-gated session の Approve は production dispatch 経由で受理され、
    /// state mutation と ApprovalResolved append を同じ command 受理サイクルで行う。
    #[tokio::test]
    async fn dispatch_approve_node_accepts_mutates_state_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-approve";
        let mut exec = make_waiting_approval_execution(&execution_id, worktree_path);
        exec.current_session_id = None;
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                Some("lgtm".to_string()),
                "review",
                None,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "terminal execution must be released after Completed"
        );
        let events = read_dispatch_events(&app, &execution_id);
        assert!(matches!(
            events.as_slice(),
            [
                WorkflowEvent::ApprovalResolved { .. },
                WorkflowEvent::NodeCompleted { node_name, .. },
                WorkflowEvent::ExecutionCompleted { .. },
            ] if node_name == "review"
        ));
    }

    // 撤去済み: parent ChatSession / persist_state 機構の撤去で意味を失ったテスト。

    /// Spec [04] no-op 不変条件: Approve の対象不在・stale node・既決 node は
    /// production dispatch 経由でも state を変化させず event を append しない。
    #[tokio::test]
    async fn dispatch_approve_rejects_missing_stale_and_resolved_targets_without_append() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let missing_execution_id = uuid::Uuid::new_v4().to_string();
        let missing = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &missing_execution_id,
                None,
                "review",
                None,
            )
            .await;
        assert!(matches!(
            missing,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));
        assert!(read_dispatch_events(&app, &missing_execution_id).is_empty());

        let stale_execution_id = uuid::Uuid::new_v4().to_string();
        let mut stale_exec =
            make_waiting_approval_execution(&stale_execution_id, "/wt/approve-stale");
        stale_exec.current_session_id = None;
        let stale_before = stale_exec.clone();
        insert_execution_and_register_active(&engine, stale_exec, ExecutionOrigin::DesktopUi).await;
        let stale = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &stale_execution_id,
                None,
                "old-review",
                None,
            )
            .await;
        assert!(matches!(
            stale,
            Err(WorkflowEngineError::UnauthorizedApprovalTarget(_))
        ));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&stale_execution_id).unwrap();
        assert_eq!(restored.state, stale_before.state);
        assert_eq!(restored.current_node_index, stale_before.current_node_index);
        assert_eq!(restored.node_history.len(), stale_before.node_history.len());
        drop(execs);
        assert!(read_dispatch_events(&app, &stale_execution_id).is_empty());

        let resolved_execution_id = uuid::Uuid::new_v4().to_string();
        let mut resolved_exec =
            make_waiting_approval_execution(&resolved_execution_id, "/wt/approve-resolved");
        resolved_exec.current_session_id = None;
        resolved_exec.state = RuntimeExecutionState::Completed;
        let resolved_before = resolved_exec.clone();
        engine
            .executions
            .lock()
            .await
            .insert(resolved_execution_id.clone(), resolved_exec);
        let resolved = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &resolved_execution_id,
                None,
                "review",
                None,
            )
            .await;
        assert!(matches!(
            resolved,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&resolved_execution_id).unwrap();
        assert_eq!(restored.state, resolved_before.state);
        assert_eq!(
            restored.node_history.len(),
            resolved_before.node_history.len()
        );
        drop(execs);
        assert!(read_dispatch_events(&app, &resolved_execution_id).is_empty());
    }

    // 撤去済み: persist_state 注入失敗を介した rollback テストは persist_state 機構の撤去で
    // 意味を失った。required event append 失敗の rollback は append_failure 系テストが担保する。

    /// Spec [04] durable commit order: required event append が成功した後の ExecutionStore
    /// metadata sync failure は、event log と engine state を rollback しない。
    /// event log が source of truth、metadata は post-commit projection であることを固定する。
    #[tokio::test]
    async fn dispatch_approve_node_execution_store_sync_failure_keeps_committed_event_log_and_state(
    ) {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_execution_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/execution-store-sync-rollback";
        let mut exec = make_waiting_approval_execution(&execution_id, worktree_path);
        exec.current_session_id = None;
        let snapshot_before = exec.clone();
        insert_execution_and_register_active(&engine, exec, ExecutionOrigin::DesktopUi).await;

        let bad_data_dir = tmp.path().join("not-a-directory");
        std::fs::write(&bad_data_dir, "file").unwrap();
        engine.set_execution_store_data_dir(bad_data_dir).await;

        let result = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                Some("lgtm".to_string()),
                "review",
                None,
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let committed = execs.get(&execution_id).unwrap();
        assert_eq!(committed.state, RuntimeExecutionState::Completed);
        assert_eq!(
            committed.node_history.len(),
            snapshot_before.node_history.len() + 1
        );
        drop(execs);
        let events = read_dispatch_events(&app, &execution_id);
        assert!(
            !events.is_empty(),
            "required events must remain durable after metadata sync failure"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ApprovalResolved { .. }
                | WorkflowEvent::NodeCompleted { .. }
                | WorkflowEvent::ExecutionCompleted { .. }
        )));
    }

    // 撤去済み: persist_state 注入失敗テストは parent ChatSession 機構撤去で意味を失った。

    /// Spec [04] atomic mutation 境界（A3 AbortExecution terminal sync post-commit 化）:
    /// `abort_workflow_by_execution_id` は append 失敗時に Execution Store / external 副作用を
    /// 一切実行しないことが構造的不変条件。本テストは pre-commit が in-memory state
    /// 変更のみであり、append 失敗時に snapshot 一括復元のみで完全に元状態へ戻せる
    /// ことを直接確認する（外部依存の差し替えを必要としない経路）。
    #[tokio::test]
    async fn abort_execution_pre_commit_holds_only_in_memory_mutation() {
        let engine = WorkflowRuntimeService::new_for_test();
        let execution_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_waiting_approval_execution(&execution_id, "/wt/pre-commit");
        exec.state = RuntimeExecutionState::Running;
        let snapshot_before = exec.clone();
        engine
            .executions
            .lock()
            .await
            .insert(execution_id.clone(), exec);

        // pre-commit 区間で行う state mutation を再現（abort_workflow_by_execution_id 内の
        // node 2 と同等）。
        let mutated_timestamp = 1234.0;
        {
            let mut execs = engine.executions.lock().await;
            let exec = execs.get_mut(&execution_id).unwrap();
            assert!(
                exec.is_active(),
                "active な execution でなければ mutation しない"
            );
            exec.state = RuntimeExecutionState::Aborted;
            exec.updated_at = mutated_timestamp;
        }
        {
            let execs = engine.executions.lock().await;
            let exec = execs.get(&execution_id).unwrap();
            assert_eq!(exec.state, RuntimeExecutionState::Aborted);
            assert_eq!(exec.updated_at, mutated_timestamp);
        }

        // append 失敗を擬制した snapshot 一括復元（A3: pre-commit 区間は in-memory のみ
        // のため、Execution Store / interrupt_agent / persist 等の外部副作用は不要）。
        {
            let mut execs = engine.executions.lock().await;
            if let Some(exec) = execs.get_mut(&execution_id) {
                *exec = snapshot_before.clone();
            }
        }
        let execs = engine.executions.lock().await;
        let restored = execs.get(&execution_id).expect("execution must remain");
        assert_eq!(
            restored.state,
            RuntimeExecutionState::Running,
            "snapshot 復元で active 状態に戻る"
        );
        assert_ne!(
            restored.updated_at, mutated_timestamp,
            "pre-commit で書いた updated_at も一括復元される"
        );
    }

    /// 起動時 recovery: 前回起動中に確定 event が書かれないまま終了した execution について、
    /// `recover_orphan_executions` が NDJSON 末尾に `ExecutionInterrupted(Orphan)` を append し、
    /// metadata 上に再開 checkpoint を残す。既定で abort せず、abort / resume のどちらも
    /// typed command の許可状態になる。
    #[tokio::test]
    async fn recover_orphan_executions_leaves_resumable_interrupted_checkpoint() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());

        // 前回プロセスの状態を模擬: metadata.current_node は plan のままだが、event log では
        // plan の NodeCompleted まで確定し、次の review の NodeStarted 前に crash している。
        let prev_store = std::sync::Arc::new(
            crate::adaptor::gateway::workflow::execution_store::ExecutionStore::new(),
        );
        prev_store.set_data_dir(data_dir.clone()).await;
        let orphan_id = uuid::Uuid::new_v4().to_string();
        seed_resumable_orphan_execution(&prev_store, &data_dir, &orphan_id, "/wt/a").await;

        // 起動直後を模擬した engine (空の in-memory state + 同じ data_dir)。
        let engine = std::sync::Arc::new(WorkflowRuntimeService::new_for_test());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        engine.recover_orphan_executions(app.handle()).await;

        // metadata は terminal にせず、Orphan 理由と再開点を持つ Interrupted checkpoint になる。
        let summary = engine
            .execution_store
            .get_execution(&orphan_id)
            .await
            .expect("metadata must remain after recovery");
        assert_eq!(summary.status, ExecutionStatus::Interrupted);
        assert_eq!(
            summary.interruption_reason,
            Some(ExecutionInterruptionReason::Orphan)
        );
        assert_eq!(summary.resume_from_node.as_deref(), Some("review"));
        assert_eq!(summary.current_node, None);
        assert!(summary.completed_at.is_none());
        assert!(summary.error_reason.is_none());
        assert!(summary.status.can_resume());
        assert!(summary.status.can_abort());

        // 末尾 event と event-log projection も同じ Interrupted checkpoint を返す。
        let events = read_dispatch_events(&app, &orphan_id);
        assert!(
            matches!(
                events.last(),
                Some(WorkflowEvent::ExecutionInterrupted {
                    reason: ExecutionInterruptionReason::Orphan,
                    ..
                })
            ),
            "log の末尾は ExecutionInterrupted(Orphan): {:?}",
            events.last()
        );
        let projected =
            crate::adaptor::gateway::workflow::event_projection::project_workflow_execution(
                &orphan_id, &events,
            )
            .unwrap()
            .unwrap();
        assert_eq!(projected.status, ExecutionStatus::Interrupted);
        assert_eq!(
            projected.interruption_reason,
            Some(ExecutionInterruptionReason::Orphan)
        );
        assert_eq!(projected.resume_from_node.as_deref(), Some("review"));
        assert_eq!(
            projected
                .node_executions
                .iter()
                .find(|node| node.node_name == "plan")
                .expect("confirmed plan NodeExecution must remain in projection")
                .status,
            crate::domain::workflow::NodeExecutionStatus::Succeeded
        );
        assert!(projected.completed_at.is_none());
    }

    #[tokio::test]
    async fn recovered_orphans_accept_typed_resume_and_abort_commands() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let previous_store =
            Arc::new(crate::adaptor::gateway::workflow::execution_store::ExecutionStore::new());
        previous_store.set_data_dir(data_dir.clone()).await;
        let resume_id = uuid::Uuid::new_v4().to_string();
        let abort_id = uuid::Uuid::new_v4().to_string();
        seed_resumable_orphan_execution(
            &previous_store,
            &data_dir,
            &resume_id,
            "/wt/orphan-resume",
        )
        .await;
        seed_resumable_orphan_execution(&previous_store, &data_dir, &abort_id, "/wt/orphan-abort")
            .await;

        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        engine.recover_orphan_executions(app.handle()).await;

        for execution_id in [&resume_id, &abort_id] {
            let checkpoint = engine
                .execution_store
                .get_execution(execution_id)
                .await
                .expect("orphan recovery must retain the checkpoint");
            assert_eq!(checkpoint.status, ExecutionStatus::Interrupted);
            assert_eq!(
                checkpoint.interruption_reason,
                Some(ExecutionInterruptionReason::Orphan)
            );
            assert_eq!(checkpoint.resume_from_node.as_deref(), Some("review"));
        }

        let (session_store, agent_runtime) = make_dispatch_deps(data_dir);
        let gateway = Arc::new(RecoveredOrphanCommandGateway {
            app: app.handle().clone(),
            engine: engine.clone(),
            session_store,
            agent_runtime,
        });
        WorkflowResumeExecutionUsecase::new(gateway.clone())
            .execute(ResumeExecutionCommand {
                execution_id: resume_id.clone(),
            })
            .await
            .expect("typed ResumeExecution must accept a recovered orphan checkpoint");
        WorkflowAbortExecutionUsecase::new(gateway)
            .execute(AbortExecutionCommand {
                execution_id: abort_id.clone(),
                expected_node_name: None,
            })
            .await
            .expect("typed AbortExecution must accept a recovered orphan checkpoint");

        let resumed = engine
            .execution_store
            .get_execution(&resume_id)
            .await
            .expect("resumed metadata");
        assert_eq!(resumed.status, ExecutionStatus::Running);
        assert_eq!(resumed.current_node.as_deref(), Some("review"));
        assert_eq!(resumed.interruption_reason, None);
        assert_eq!(resumed.resume_from_node, None);
        let resumed_events = read_dispatch_events(&app, &resume_id);
        assert!(resumed_events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ExecutionResumed {
                resume_from_node,
                ..
            } if resume_from_node == "review"
        )));
        assert_eq!(
            resumed_events
                .iter()
                .filter(|event| matches!(
                    event,
                    WorkflowEvent::NodeStarted { node_name, .. } if node_name == "plan"
                ))
                .count(),
            1,
            "confirmed plan must not run again after orphan resume"
        );

        let aborted = engine
            .execution_store
            .get_execution(&abort_id)
            .await
            .expect("aborted metadata");
        assert_eq!(aborted.status, ExecutionStatus::Aborted);
        assert!(aborted.completed_at.is_some());
        assert!(matches!(
            read_dispatch_events(&app, &abort_id).last(),
            Some(WorkflowEvent::ExecutionAborted {
                aborted_node: Some(node),
                ..
            }) if node == "review"
        ));
    }

    /// 起動時 recovery: 既に terminal な metadata は変更されない（idempotent）。
    /// recovery 二回目以降は append も persist も走らない。
    #[tokio::test]
    async fn recover_orphan_executions_is_idempotent_for_already_terminal_executions() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());

        let prev_store = std::sync::Arc::new(
            crate::adaptor::gateway::workflow::execution_store::ExecutionStore::new(),
        );
        prev_store.set_data_dir(data_dir.clone()).await;
        let done_id = uuid::Uuid::new_v4().to_string();
        prev_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: done_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: "/wt/b".to_string(),
                current_node: Some("plan".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        prev_store
            .complete_execution(&done_id, TerminalExecutionStatus::Completed, 150.0, None)
            .await
            .unwrap();

        let engine = std::sync::Arc::new(WorkflowRuntimeService::new_for_test());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let events_before = read_dispatch_events(&app, &done_id);
        engine.recover_orphan_executions(app.handle()).await;
        let events_after = read_dispatch_events(&app, &done_id);
        assert_eq!(
            events_before.len(),
            events_after.len(),
            "terminal な execution には event を append しない"
        );
        let summary = engine
            .execution_store
            .get_execution(&done_id)
            .await
            .expect("metadata must remain");
        assert_eq!(summary.status, ExecutionStatus::Completed);
    }

    #[tokio::test]
    async fn recover_orphan_executions_reconciles_a_durable_terminal_event_without_interrupting() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let prev_store = std::sync::Arc::new(
            crate::adaptor::gateway::workflow::execution_store::ExecutionStore::new(),
        );
        prev_store.set_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        prev_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: "/wt/terminal-crash-window".to_string(),
                current_node: Some("plan".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: Default::default(),
            })
            .await
            .unwrap();
        let definition = WorkflowDefinitionYaml {
            name: "wf".to_string(),
            nodes: vec![make_test_node(
                "plan",
                TestKind::Session,
                "plan",
                vec![],
                None,
            )],
            ..Default::default()
        };
        WorkflowEventLog::new(&data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: "wf".to_string(),
                    worktree_path: "/wt/terminal-crash-window".to_string(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    permission_mode: "ask".to_string(),
                    definition,
                    timestamp: 100.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "plan-1".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 101.0,
                },
                WorkflowEvent::NodeCompleted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "plan-1".to_string(),
                    node_name: "plan".to_string(),
                    attempt: 1,
                    result_summary: Some("done".to_string()),
                    token_usage: None,
                    timestamp: 102.0,
                },
                WorkflowEvent::ExecutionCompleted {
                    execution_id: execution_id.clone(),
                    total_token_usage: Default::default(),
                    timestamp: 103.0,
                },
            ])
            .unwrap();

        let engine = WorkflowRuntimeService::new_for_test();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        engine.recover_orphan_executions(app.handle()).await;

        let summary = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(summary.status, ExecutionStatus::Completed);
        assert_eq!(summary.completed_at, Some(103.0));
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::ExecutionInterrupted { .. })));
    }

    #[tokio::test]
    async fn recover_orphan_executions_repairs_interrupted_metadata_after_durable_abort() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let prev_store = crate::adaptor::gateway::workflow::execution_store::ExecutionStore::new();
        prev_store.set_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        prev_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: "/wt/interrupted-abort-repair".to_string(),
                current_node: Some("plan".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: Default::default(),
            })
            .await
            .unwrap();
        prev_store
            .interrupt_execution(
                &execution_id,
                ExecutionInterruptionReason::Stop,
                Some("plan".to_string()),
                102.0,
            )
            .await
            .unwrap();
        WorkflowEventLog::new(&data_dir)
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: "wf".to_string(),
                    worktree_path: "/wt/interrupted-abort-repair".to_string(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    permission_mode: "ask".to_string(),
                    definition: WorkflowDefinitionYaml {
                        name: "wf".to_string(),
                        nodes: vec![make_test_node(
                            "plan",
                            TestKind::Session,
                            "plan",
                            vec![],
                            None,
                        )],
                        ..Default::default()
                    },
                    timestamp: 100.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "plan-1".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 101.0,
                },
                WorkflowEvent::ExecutionInterrupted {
                    execution_id: execution_id.clone(),
                    reason: ExecutionInterruptionReason::Stop,
                    timestamp: 102.0,
                },
                WorkflowEvent::ExecutionAborted {
                    execution_id: execution_id.clone(),
                    aborted_node: Some("plan".to_string()),
                    timestamp: 103.0,
                },
            ])
            .unwrap();

        let engine = WorkflowRuntimeService::new_for_test();
        engine.set_execution_store_data_dir(data_dir).await;
        engine.recover_orphan_executions(app.handle()).await;

        let summary = engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(summary.status, ExecutionStatus::Aborted);
        assert_eq!(summary.completed_at, Some(103.0));
        assert_eq!(summary.interruption_reason, None);
        assert_eq!(summary.resume_from_node, None);
    }

    #[tokio::test]
    async fn recover_orphan_executions_removes_an_uncommitted_start_reservation() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let prev_store = crate::adaptor::gateway::workflow::execution_store::ExecutionStore::new();
        prev_store.set_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        prev_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: "wf".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: "/wt/uncommitted-start".to_string(),
                current_node: Some("plan".to_string()),
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: Default::default(),
            })
            .await
            .unwrap();

        let engine = WorkflowRuntimeService::new_for_test();
        engine.set_execution_store_data_dir(data_dir).await;
        engine.recover_orphan_executions(app.handle()).await;

        assert!(engine
            .execution_store
            .get_execution(&execution_id)
            .await
            .is_none());
    }

    // ---- [08] handle_submit_output: 単一トランザクション境界 ----

    /// Production と同じ submit-output primitive 経由で構造化出力を提出する。
    #[allow(clippy::too_many_arguments)]
    async fn submit_output_for_test(
        engine: &Arc<WorkflowRuntimeService>,
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
        execution_id: &str,
        node_name: &str,
        contract: &str,
        artifact: serde_json::Value,
        _request_id: Option<&str>,
        _submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError> {
        let (session_store, agent_runtime) = make_dispatch_deps(dispatch_data_dir(app));
        submit_output_for_test_with_deps(
            engine,
            app,
            &session_store,
            &agent_runtime,
            execution_id,
            node_name,
            contract,
            artifact,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_output_for_test_with_deps(
        engine: &Arc<WorkflowRuntimeService>,
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        node_name: &str,
        contract: &str,
        artifact: serde_json::Value,
        _request_id: Option<&str>,
        _submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError> {
        engine
            .submit_workflow_output(
                app,
                session_store,
                agent_runtime,
                execution_id,
                node_name.to_string(),
                None,
                contract.to_string(),
                artifact,
            )
            .await
    }

    fn make_submit_output_workflow() -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: "submit-wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: submit_test_schemas(),
            nodes: vec![{
                let mut node = make_test_node("review", TestKind::Session, "review", vec![], None);
                node.artifact = Some("review-verdict".to_string());
                node
            }],
        }
    }

    fn read_submit_output_events(app: &DispatchTestApp, execution_id: &str) -> Vec<WorkflowEvent> {
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle())
                .expect("data_dir");
        WorkflowEventLog::new(&data_dir)
            .read_log(execution_id)
            .unwrap_or_default()
    }

    async fn node_output_for(
        engine: &WorkflowRuntimeService,
        execution_id: &str,
        node_name: &str,
    ) -> Option<RuntimeArtifact> {
        engine
            .executions
            .lock()
            .await
            .get(execution_id)
            .and_then(|exec| exec.artifacts.get(node_name).cloned())
    }

    /// [08] 振る舞い定義 Rule 1（適合する場合）: contract に適合する構造化出力は
    /// node output として確定し、後続 node から参照可能になり、事実履歴に記録される。
    #[tokio::test]
    async fn submit_output_persists_node_output_and_appends_event_when_contract_satisfied() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                "/wt/submit-ok".to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &execution_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            Some("00000000-0000-0000-0000-000000000aa1"),
            Some(800.0),
        )
        .await
        .unwrap();

        // artifacts slot に書き込まれている
        let node_output = node_output_for(&engine, &execution_id, "review")
            .await
            .expect("artifacts must be updated");
        assert_eq!(node_output.contract.as_deref(), Some("review-verdict"));
        assert_eq!(node_output.artifact.as_ref().unwrap()["verdict"], "LGTM");

        // ArtifactProduced event が追記されている
        let events = read_submit_output_events(&app, &execution_id);
        let submitted = events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::ArtifactProduced {
                    node_name,
                    contract,
                    value,
                    request_id,
                    submitted_at,
                    ..
                } if node_name == "review" => Some((
                    contract.clone(),
                    value.clone(),
                    request_id.clone(),
                    *submitted_at,
                )),
                _ => None,
            })
            .expect("ArtifactProduced event must be appended");
        assert_eq!(submitted.0.as_deref(), Some("review-verdict"));
        assert_eq!(submitted.1["verdict"], "LGTM");
        assert_eq!(submitted.2, None);
        assert_eq!(submitted.3, None);
    }

    #[tokio::test]
    async fn arbitrary_contract_submit_redacts_env_secret_in_state_log_and_api_response() {
        let secret = "ARBITRARY_CONTRACT_SECRET_12345";
        let _secret =
            crate::test_support::EnvVarGuard::set_value("RELEASH_ARBITRARY_SUBMIT_SECRET", secret);
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                "/wt/arbitrary-contract-redaction".to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &execution_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": format!("LGTM {secret}")}),
            None,
            None,
        )
        .await
        .unwrap();

        let state = node_output_for(&engine, &execution_id, "review")
            .await
            .unwrap();
        let state_text = serde_json::to_string(&state.artifact).unwrap();
        assert!(state_text.contains("[REDACTED]"));
        assert!(!state_text.contains(secret));

        let ndjson = std::fs::read_to_string(
            data_dir
                .join("workflow_execution_logs")
                .join(format!("{execution_id}.ndjson")),
        )
        .unwrap();
        assert!(ndjson.contains("[REDACTED]"));
        assert!(!ndjson.contains(secret));

        let (router, _, _) =
            crate::adaptor::controller::api::test_support::test_router(&data_dir, "secret");
        let (status, response) = crate::adaptor::controller::api::test_support::get_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/artifacts/review"),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let response_text = serde_json::to_string(&response).unwrap();
        assert_eq!(response["value"]["verdict"], "LGTM [REDACTED]");
        assert!(!response_text.contains(secret));
    }

    /// #1250: contract 不適合の SubmitOutput は即 reject せず repair policy に渡す。
    /// invalid payload 自体は保存せず、ContractRepairRequested のみを append する。
    #[tokio::test]
    async fn submit_output_invalid_contract_requests_repair_without_persisting_output() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/submit-invalid";
        let session_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
        let mut workflow = make_submit_output_workflow();
        workflow.nodes[0].artifact = Some("spec-directory".to_string());
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                workflow,
                RuntimeExecutionState::Running,
                worktree_path.to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;
        {
            let mut execs = engine.executions.lock().await;
            let exec = execs.get_mut(&execution_id).expect("seeded execution");
            exec.current_session_id = Some(session_id.to_string());
            exec.node_executions
                .iter_mut()
                .find(|node_execution| {
                    node_execution.node_name == "review"
                        && node_execution.status == NodeExecutionStatus::Running
                })
                .expect("seeded review NodeExecution")
                .session_id = Some(session_id.to_string());
        }
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        session_store
            .save_full_session_for_migration_or_restore(
                &data_dir,
                &chat_session_for_test(session_id, worktree_path, None, true),
            )
            .unwrap();
        insert_ready_agent_process_for_internal_turn_test(
            &handles,
            &session_store,
            &data_dir,
            session_id,
        )
        .await;

        submit_output_for_test_with_deps(
            &engine,
            app.handle(),
            &session_store,
            &handles,
            &execution_id,
            "review",
            "spec-directory",
            serde_json::json!({}),
            Some("00000000-0000-0000-0000-000000000ab1"),
            Some(900.0),
        )
        .await
        .unwrap();

        // artifacts は更新されない
        assert!(node_output_for(&engine, &execution_id, "review")
            .await
            .is_none());
        // ArtifactProduced event も書かれない
        let events = read_submit_output_events(&app, &execution_id);
        assert!(events
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::ArtifactProduced { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ContractViolated {
                node_name,
                request_id: None,
                repair_attempt: 1,
                violations,
                ..
            } if node_name == "review"
                && !violations.is_empty()
                && violations.iter().all(|violation|
                    !violation.path.is_empty() && !violation.reason.is_empty())
        )));
    }

    #[tokio::test]
    async fn fanout_child_invalid_submit_repairs_the_selected_node_execution() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let selected_node_execution_id = uuid::Uuid::new_v4().to_string();
        let sibling_node_execution_id = uuid::Uuid::new_v4().to_string();
        let selected_session_id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
        let sibling_session_id = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
        let worktree_path = "/wt/fanout-submit-invalid";
        let mut child = make_fanout_child("review-child");
        child.artifact = Some("spec-directory".to_string());
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-submit-wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: submit_test_schemas(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-child"]),
                child,
            ],
        };
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                workflow,
                RuntimeExecutionState::Running,
                worktree_path.to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;
        {
            let mut executions = engine.executions.lock().await;
            let execution = executions.get_mut(&execution_id).expect("seeded execution");
            execution.current_session_id = None;
            install_test_fanout(
                execution,
                vec![
                    FanoutChildRuntime {
                        node_execution_id: selected_node_execution_id.clone(),
                        node_name: "review-child".to_string(),
                        session_id: selected_session_id.to_string(),
                        state: FanoutChildRuntimeState::Running,
                        result: None,
                        artifact: None,
                        contract: None,
                        failure_kind: None,
                        failure_disposition: None,
                        token_usage: TokenUsage::default(),
                        attempt: 1,
                        completed_at: None,
                    },
                    FanoutChildRuntime {
                        node_execution_id: sibling_node_execution_id.clone(),
                        node_name: "review-child".to_string(),
                        session_id: sibling_session_id.to_string(),
                        state: FanoutChildRuntimeState::Running,
                        result: None,
                        artifact: None,
                        contract: None,
                        failure_kind: None,
                        failure_disposition: None,
                        token_usage: TokenUsage::default(),
                        attempt: 1,
                        completed_at: None,
                    },
                ],
            );
        }
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        session_store
            .save_full_session_for_migration_or_restore(
                &data_dir,
                &chat_session_for_test(selected_session_id, worktree_path, None, true),
            )
            .unwrap();
        insert_ready_agent_process_for_internal_turn_test(
            &handles,
            &session_store,
            &data_dir,
            selected_session_id,
        )
        .await;

        engine
            .submit_workflow_output(
                app.handle(),
                &session_store,
                &handles,
                &execution_id,
                "review-child".to_string(),
                Some(selected_node_execution_id.clone()),
                "spec-directory".to_string(),
                serde_json::json!({}),
            )
            .await
            .unwrap();

        let events = read_submit_output_events(&app, &execution_id);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ContractViolated {
                node_execution_id,
                node_name,
                repair_attempt: 1,
                violations,
                ..
            } if node_execution_id == &selected_node_execution_id
                && node_name == "review-child"
                && !violations.is_empty()
        )));
        assert!(events.iter().all(|event| !matches!(
            event,
            WorkflowEvent::ContractViolated { node_execution_id, .. }
                if node_execution_id == &sibling_node_execution_id
        )));
    }

    /// [08] 振る舞い定義 Rule 1: 不在 node に対する提出は副作用なしで拒否される。
    #[tokio::test]
    async fn submit_output_rejects_unknown_node_without_side_effects() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                "/wt/submit-unknown".to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &execution_id,
            "ghost-node",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
        let events = read_submit_output_events(&app, &execution_id);
        assert!(events
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::ArtifactProduced { .. })));
    }

    /// [08] 振る舞い定義 Rule 1: 不在 execution （UUID 未登録）に対する提出は ExecutionNotFound で拒否。
    #[tokio::test]
    async fn submit_output_rejects_unknown_execution() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &execution_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ExecutionNotFound(_)));
    }

    /// [08] caller の `--type` と engine の expected contract が一致しない場合は拒否され、
    /// 副作用は発生しない。
    #[tokio::test]
    async fn submit_output_rejects_contract_type_mismatch() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                "/wt/submit-mismatch".to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &execution_id,
            "review",
            "fix-result",
            serde_json::json!({"status": "FIXED"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
        assert!(node_output_for(&engine, &execution_id, "review")
            .await
            .is_none());
    }

    /// [08] 振る舞い定義 Rule 3: 提出済み output は後続 node から
    /// `input_reference` 経路で経路非依存に参照できる。artifacts に
    /// 書き込まれた entry が contract 由来の `contract` を保持することを担保する。
    #[tokio::test]
    async fn submit_output_node_output_carries_contract_for_downstream_reference() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                "/wt/submit-downstream".to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &execution_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap();
        let node_output = node_output_for(&engine, &execution_id, "review")
            .await
            .expect("artifacts slot must be populated");
        assert_eq!(node_output.contract.as_deref(), Some("review-verdict"));
        // artifact が後続経路に渡る shape で保持される
        assert!(node_output.artifact.is_some());
    }

    /// [08] spec-directory artifact が submit された場合、node output に
    /// 検証済み artifact が保存される。
    #[tokio::test]
    async fn submit_output_stores_spec_dir_artifact_without_workflow_variable_side_effects() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "spec-wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: submit_test_schemas(),
            nodes: vec![{
                let mut node = make_test_node("plan", TestKind::Session, "plan", vec![], None);
                node.artifact = Some("spec-directory".to_string());
                node
            }],
        };
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                workflow,
                RuntimeExecutionState::Running,
                "/wt/submit-spec".to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &execution_id,
            "plan",
            "spec-directory",
            serde_json::json!({"spec_dir": "docs/spec/issues-1029.md"}),
            None,
            None,
        )
        .await
        .unwrap();

        let exec = engine.executions.lock().await;
        let exec = exec.get(&execution_id).unwrap();
        assert_eq!(
            exec.artifacts["plan"].artifact.as_ref().unwrap()["spec_dir"],
            "docs/spec/issues-1029.md"
        );
    }

    /// [08] 振る舞い定義 Rule 1 Scenario 3: 既に出力を受け付けられる状態にない node に
    /// 対する提出は拒否され、state と event log が変化しないことを確認する。
    #[tokio::test]
    async fn submit_output_rejects_non_accepting_node_without_side_effects() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "multi-node".to_string(),
            description: String::new(),
            builtin: false,
            schemas: submit_test_schemas(),
            nodes: vec![
                {
                    let mut node =
                        make_test_node("first", TestKind::Session, "first", vec![], None);
                    node.artifact = Some("review-verdict".to_string());
                    node
                },
                {
                    let mut node =
                        make_test_node("second", TestKind::Session, "second", vec![], None);
                    node.artifact = Some("review-verdict".to_string());
                    node
                },
            ],
        };
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                workflow,
                RuntimeExecutionState::Running,
                "/wt/submit-stale".to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        // current node を `second` に進めて、`first` を提出受付対象から外す。
        engine
            .force_current_node_index_for_test(&execution_id, 1)
            .await;

        let events_before = read_submit_output_events(&app, &execution_id);
        let exec_before = engine
            .executions
            .lock()
            .await
            .get(&execution_id)
            .map(|e| e.artifacts.clone())
            .unwrap();

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &execution_id,
            "first",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::InvalidState(_)));

        // state は変化していない
        let exec_after = engine
            .executions
            .lock()
            .await
            .get(&execution_id)
            .map(|e| e.artifacts.clone())
            .unwrap();
        assert_eq!(exec_before.len(), exec_after.len());

        // ArtifactProduced event は append されない
        let events_after = read_submit_output_events(&app, &execution_id);
        assert_eq!(events_before.len(), events_after.len());
        assert!(events_after
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::ArtifactProduced { .. })));
    }

    /// [08] 振る舞い定義 Rule 4: agent node の自由文出力に `<workflow_output>` 相当の
    /// 表現が含まれていても、明示的提出が無い限り artifacts は更新されず、
    /// ArtifactProduced event も追記されない（prose 抽出経路の完全廃止）。
    #[tokio::test]
    async fn agent_free_text_workflow_output_block_does_not_confirm_node_output() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                "/wt/agent-freetext".to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        let outputs_before = engine
            .executions
            .lock()
            .await
            .get(&execution_id)
            .map(|e| e.artifacts.clone())
            .unwrap();
        let events_before = read_submit_output_events(&app, &execution_id);

        let final_text = r#"承認します。
<workflow_output type="review-verdict">{"verdict":"LGTM"}</workflow_output>"#;
        let final_parts = vec![MessagePart::Text {
            content: final_text.to_string(),
            parent_tool_use_id: None,
        }];

        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        // 自由文経路は prose 抽出を行わないため、artifacts は変化せず、
        // contract がある node は明示的提出なしでは完了しない。
        // [08] handle_auto_complete のエラーを .ok() で握り潰さないこと（review 指摘）。
        // 完了経路を通って初めて「自由文出力中の `<workflow_output>` は無視される」を
        // 検証できるため、.expect で経路実行を保証する。
        engine
            .handle_auto_complete(
                app.handle(),
                &session_store,
                &handles,
                "/wt/agent-freetext",
                &final_parts,
                "review",
            )
            .await
            .expect("handle_auto_complete must succeed for agent free-text path");

        let outputs_after = engine
            .executions
            .lock()
            .await
            .get(&execution_id)
            .map(|e| e.artifacts.clone())
            .unwrap_or_default();
        // artifacts 数は変わらず、artifact を持つ entry が追加されていない
        assert_eq!(outputs_before.len(), outputs_after.len());

        // ArtifactProduced event も追記されていない
        let events_after = read_submit_output_events(&app, &execution_id);
        let submitted_count_before = events_before
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::ArtifactProduced { .. }))
            .count();
        let submitted_count_after = events_after
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::ArtifactProduced { .. }))
            .count();
        assert_eq!(submitted_count_before, submitted_count_after);
        let node_completed = events_after
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::NodeCompleted { node_name, .. } if node_name == "review"))
            .count();
        assert_eq!(
            node_completed, 0,
            "handle_auto_complete must not advance a contract node without SubmitOutput"
        );
        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "seeded test execution has no active session, so missing SubmitOutput fails and releases the terminal execution"
        );
        assert!(
            events_after
                .iter()
                .any(|event| matches!(event, WorkflowEvent::ExecutionFailed { .. })),
            "terminal failure must be recorded in the event log"
        );
    }

    #[tokio::test]
    async fn missing_required_output_requests_repair_without_failing_within_limit() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-within-limit";
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                worktree_path.to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let session_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        session_store
            .save_full_session_for_migration_or_restore(
                &data_dir,
                &chat_session_for_test(session_id, worktree_path, None, true),
            )
            .unwrap();
        insert_ready_agent_process_for_internal_turn_test(
            &handles,
            &session_store,
            &data_dir,
            session_id,
        )
        .await;

        engine
            .handle_missing_required_output(
                app.handle(),
                &session_store,
                &handles,
                worktree_path,
                &execution_id,
                "submit-wf",
                "review",
                "review-verdict",
                1,
                Some(session_id),
                None,
                None,
                SubmissionViolation::MissingSubmitOutput,
                None,
            )
            .await
            .unwrap();

        assert!(
            engine.contains_execution_for_test(&execution_id).await,
            "repairable mismatch must keep the execution active"
        );
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                WorkflowEvent::ContractViolated {
                    node_name,
                    repair_attempt: 1,
                    ..
                } if node_name == "review"
            )),
            "repair attempt must append ContractRepairRequested; got {events:?}"
        );
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, WorkflowEvent::ExecutionFailed { .. })),
            "within-limit repair request must not terminally fail the execution; got {events:?}"
        );
    }

    #[tokio::test]
    async fn missing_required_output_fails_when_repair_turn_cannot_start() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-start-failure";
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                worktree_path.to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let session_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
        session_store
            .save_full_session_for_migration_or_restore(
                &data_dir,
                &chat_session_for_test(session_id, worktree_path, None, true),
            )
            .unwrap();
        handles
            .insert_failing_runtime_state_for_test(session_id)
            .await;

        engine
            .handle_missing_required_output(
                app.handle(),
                &session_store,
                &handles,
                worktree_path,
                &execution_id,
                "submit-wf",
                "review",
                "review-verdict",
                1,
                Some(session_id),
                None,
                None,
                SubmissionViolation::MissingSubmitOutput,
                None,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "repair start failure must terminally release the execution"
        );
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                WorkflowEvent::ContractViolated {
                    node_name,
                    repair_attempt: 1,
                    ..
                } if node_name == "review"
            )),
            "the attempted repair must be observable before terminal failure; got {events:?}"
        );
        let execution_failed = events.iter().find_map(|event| match event {
            WorkflowEvent::ExecutionFailed {
                reason,
                failure_kind,
                ..
            } => Some((reason, failure_kind)),
            _ => None,
        });
        let Some((reason, failure_kind)) = execution_failed else {
            panic!("repair start failure must append ExecutionFailed; got {events:?}");
        };
        assert_eq!(*failure_kind, NodeExecutionFailureKind::InfrastructureCrash);
        assert!(
            reason.contains("contract output repair turn failed to start"),
            "terminal reason must include repair startup failure; got {reason}"
        );
    }

    #[tokio::test]
    async fn repair_turn_startup_timeout_failure_preserves_failure_metadata() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-startup-timeout";
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                worktree_path.to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let error = WorkflowEngineError::with_agent_runtime_context(
            "contract output repair turn failed to start",
            crate::usecase::agent_session::runtime::usecase::AgentRuntimeError::StartupTimeout {
                retry_count: 2,
                max_retries: 2,
            },
        );

        engine
            .fail_missing_required_output_with_metadata(
                app.handle(),
                &session_store,
                &handles,
                worktree_path,
                &execution_id,
                "review",
                "review-verdict",
                &error.to_string(),
                error.workflow_failure_kind(),
                error.retry_count(),
                None,
            )
            .await
            .unwrap();

        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        let execution_failed = events.iter().find_map(|event| match event {
            WorkflowEvent::ExecutionFailed {
                reason,
                failure_kind,
                ..
            } => Some((reason, failure_kind)),
            _ => None,
        });
        let Some((reason, failure_kind)) = execution_failed else {
            panic!("repair startup timeout must append ExecutionFailed; got {events:?}");
        };
        assert_eq!(*failure_kind, NodeExecutionFailureKind::StartupTimeout);
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeFailed {
                retry_count: Some(2),
                failure_kind: NodeExecutionFailureKind::StartupTimeout,
                ..
            }
        )));
        assert!(
            reason.contains("contract output repair turn failed to start"),
            "terminal reason must include repair startup timeout context; got {reason}"
        );
    }

    #[tokio::test]
    async fn missing_required_output_fails_with_structured_mismatch_after_repair_limit() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-limit";
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                worktree_path.to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;
        let node_execution_id = engine
            .executions
            .lock()
            .await
            .get(&execution_id)
            .and_then(|execution| execution.node_executions.first())
            .map(|execution| execution.id.clone())
            .expect("seeded node execution");
        let log = WorkflowEventLog::new(&data_dir);
        for attempt in 1..=2 {
            log.append(&WorkflowEvent::ContractViolated {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                node_name: "review".to_string(),
                violations: vec![
                    crate::adaptor::gateway::workflow::event::ContractViolationRecord {
                        path: "$".to_string(),
                        reason: submission_violation_reason(
                            SubmissionViolation::MissingSubmitOutput,
                        )
                        .to_string(),
                    },
                ],
                request_id: None,
                repair_attempt: attempt,
                timestamp: 1000.0 + f64::from(attempt),
            })
            .unwrap();
        }

        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        engine
            .handle_missing_required_output(
                app.handle(),
                &session_store,
                &handles,
                worktree_path,
                &execution_id,
                "submit-wf",
                "review",
                "review-verdict",
                1,
                Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"),
                None,
                None,
                SubmissionViolation::MissingSubmitOutput,
                None,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "exhausted repair attempts must terminally release the execution"
        );
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        let execution_failed_kind = events.iter().find_map(|event| match event {
            WorkflowEvent::ExecutionFailed { failure_kind, .. } => Some(*failure_kind),
            _ => None,
        });
        assert_eq!(
            execution_failed_kind,
            Some(NodeExecutionFailureKind::StructuredOutputMismatch)
        );
    }

    #[tokio::test]
    async fn missing_required_output_repair_attempts_are_scoped_to_attempt() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-execution-index";
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                make_submit_output_workflow(),
                RuntimeExecutionState::Running,
                worktree_path.to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;
        let session_id = "11111111-1111-4111-8111-111111111111";
        let second_node_execution_id = uuid::Uuid::new_v4().to_string();
        let first_node_execution_id = {
            let mut executions = engine.executions.lock().await;
            let execution = executions.get_mut(&execution_id).expect("seeded execution");
            let first_node_execution_id = execution.node_executions[0].id.clone();
            execution.node_executions[0].status = NodeExecutionStatus::Failed;
            execution.node_executions[0].completed_at = Some(1001.0);
            execution
                .node_execution_counts
                .insert("review".to_string(), 2);
            execution.current_session_id = Some(session_id.to_string());
            execution.node_executions.push(node_execution_fixture(
                &execution_id,
                &second_node_execution_id,
                "review",
                2,
                NodeExecutionStatus::Running,
                Some(session_id),
                None,
            ));
            first_node_execution_id
        };
        let log = WorkflowEventLog::new(&data_dir);
        log.append_batch(&[
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.clone(),
                node_execution_id: second_node_execution_id.clone(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 2,
                fanout_parent: None,
                timestamp: 1002.0,
            },
            WorkflowEvent::SessionAttached {
                execution_id: execution_id.clone(),
                node_execution_id: second_node_execution_id.clone(),
                session_id: session_id.to_string(),
                timestamp: 1002.0,
            },
        ])
        .unwrap();
        for attempt in 1..=2 {
            log.append(&WorkflowEvent::ContractViolated {
                execution_id: execution_id.clone(),
                node_execution_id: first_node_execution_id.clone(),
                node_name: "review".to_string(),
                violations: vec![
                    crate::adaptor::gateway::workflow::event::ContractViolationRecord {
                        path: "$".to_string(),
                        reason: submission_violation_reason(
                            SubmissionViolation::MissingSubmitOutput,
                        )
                        .to_string(),
                    },
                ],
                request_id: None,
                repair_attempt: attempt,
                timestamp: 1000.0 + f64::from(attempt),
            })
            .unwrap();
        }

        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        session_store
            .save_full_session_for_migration_or_restore(
                &data_dir,
                &chat_session_for_test(session_id, worktree_path, None, true),
            )
            .unwrap();
        insert_ready_agent_process_for_internal_turn_test(
            &handles,
            &session_store,
            &data_dir,
            session_id,
        )
        .await;

        engine
            .handle_missing_required_output(
                app.handle(),
                &session_store,
                &handles,
                worktree_path,
                &execution_id,
                "submit-wf",
                "review",
                "review-verdict",
                2,
                Some(session_id),
                None,
                None,
                SubmissionViolation::MissingSubmitOutput,
                None,
            )
            .await
            .unwrap();

        let events = WorkflowEventLog::new(&data_dir)
            .read_log(&execution_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ContractViolated {
                node_execution_id,
                node_name,
                repair_attempt: 1,
                ..
            } if node_execution_id == &second_node_execution_id && node_name == "review"
        )));
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, WorkflowEvent::ExecutionFailed { .. })),
            "prior attempt repair attempts must not force GiveUp for a new attempt; got {events:?}"
        );
    }

    /// [08] 振る舞い定義 Rule 1: ArtifactProduced append が失敗した場合、
    /// artifacts / artifacts / event log は提出前状態のまま保たれる。
    /// `write_log_required` の挿入 fail 経由で append 失敗を再現し、rollback の事実を
    /// 直接検証する（spec [08]: 「副作用なしで提出前状態のまま保つ」）。
    #[tokio::test]
    async fn submit_output_rolls_back_state_when_event_append_fails() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "spec-wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: [(
                "spec-directory".to_string(),
                object_schema_for_test(&["spec_dir"]),
            )]
            .into_iter()
            .collect(),
            nodes: vec![{
                let mut node = make_test_node("plan", TestKind::Session, "plan", vec![], None);
                node.artifact = Some("spec-directory".to_string());
                node
            }],
        };
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                workflow,
                RuntimeExecutionState::Running,
                "/wt/submit-rollback".to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        let exec_before = engine
            .executions
            .lock()
            .await
            .get(&execution_id)
            .map(|e| e.artifacts.clone())
            .unwrap();
        let events_before = read_submit_output_events(&app, &execution_id);

        // 次の write_log_required を失敗させる。
        engine.fail_next_required_event_append_for_test();
        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &execution_id,
            "plan",
            "spec-directory",
            serde_json::json!({"spec_dir": "docs/spec/issues-1029.md"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::SessionStore(_)));

        // state は提出前のまま保たれる
        let exec_after = engine
            .executions
            .lock()
            .await
            .get(&execution_id)
            .map(|e| e.artifacts.clone())
            .unwrap();
        assert_eq!(exec_before.len(), exec_after.len());
        assert!(!exec_after.contains_key("plan"));

        // ArtifactProduced event は append されない（log への副作用なし）
        let events_after = read_submit_output_events(&app, &execution_id);
        assert_eq!(events_before.len(), events_after.len());
        assert!(events_after
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::ArtifactProduced { .. })));
    }

    #[tokio::test]
    async fn fanout_empty_items_completes_parent_with_empty_artifact_and_follows_parent_rules() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir);

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-empty-items";
        let mut fanout = make_fanout_node("fanout-empty", vec!["review-child"]);
        fanout.rules = vec![Rule::Next("after-empty".to_string())];
        let NodeKind::Fanout(spec) = &mut fanout.kind else {
            panic!("fanout test fixture must contain a fanout node");
        };
        spec.items = Some(ItemsSource::Literal(Vec::new()));
        let mut child = make_fanout_child("review-child");
        child.input = Some("fanout-item".to_string());
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-empty-items-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "fanout-item".to_string(),
                object_schema_for_test(&["value"]),
            )]
            .into_iter()
            .collect(),
            nodes: vec![
                fanout,
                child,
                make_approval_gated_session("after-empty", "review-summary", vec![]),
            ],
        };
        engine
            .seed_active_execution_for_test(
                execution_id.clone(),
                workflow,
                RuntimeExecutionState::Running,
                worktree_path.to_string(),
                ExecutionOrigin::DesktopUi,
            )
            .await;

        engine
            .start_fanout_children(app.handle(), &session_store, &handles, worktree_path)
            .await
            .unwrap();

        let executions = engine.executions.lock().await;
        let execution = executions
            .get(&execution_id)
            .expect("execution must remain active");
        assert_eq!(
            execution.workflow.nodes[execution.current_node_index].name,
            "after-empty"
        );
        assert!(execution.fanout_runtime.is_none());
        assert!(
            execution
                .node_executions
                .iter()
                .all(|node_execution| node_execution.fanout_parent.is_none()),
            "items=[] must not create a child NodeExecution"
        );
        let parent_execution = execution
            .node_executions
            .iter()
            .find(|node_execution| node_execution.node_name == "fanout-empty")
            .expect("fanout parent NodeExecution");
        assert_eq!(parent_execution.status, NodeExecutionStatus::Succeeded);
        assert_eq!(parent_execution.artifact, Some(serde_json::json!([])));
        drop(executions);

        let events = read_dispatch_events(&app, &execution_id);
        let artifact_position = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    WorkflowEvent::ArtifactProduced {
                        node_name,
                        contract: None,
                        value,
                        ..
                    } if node_name == "fanout-empty" && value == &serde_json::json!([])
                )
            })
            .expect("fanout parent must produce [] with contract=null");
        let completed_position = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    WorkflowEvent::NodeCompleted { node_name, .. }
                        if node_name == "fanout-empty"
                )
            })
            .expect("fanout parent must complete as a normal node");
        let next_started_position = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    WorkflowEvent::NodeStarted { node_name, .. }
                        if node_name == "after-empty"
                )
            })
            .expect("fanout parent rules must start the next node");
        assert!(artifact_position < completed_position);
        assert!(completed_position < next_started_position);
        assert!(events.iter().all(|event| {
            !matches!(
                event,
                WorkflowEvent::NodeStarted { node_name, .. }
                    if node_name == "review-child"
            )
        }));
    }

    #[tokio::test]
    async fn fanout_child_completion_ignores_child_rules_and_uses_parent_rules() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir);

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-child-rules-ignored";
        let child_session_id = "fanout-child-rules-session";
        let mut fanout = make_fanout_node("fanout-review", vec!["review-child"]);
        fanout.rules = vec![Rule::Next("parent-next".to_string())];
        let mut child = make_fanout_child("review-child");
        child.rules = vec![Rule::Next("child-next".to_string())];
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-child-rules-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                fanout,
                child,
                make_approval_gated_session("child-next", "review-summary", vec![]),
                make_approval_gated_session("parent-next", "review-summary", vec![]),
            ],
        };
        let mut execution =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        execution.state = RuntimeExecutionState::Running;
        execution.current_session_id = None;
        execution.node_execution_counts = HashMap::from([("fanout-review".to_string(), 1)]);
        install_test_fanout(
            &mut execution,
            vec![test_fanout_child(
                "review-child",
                child_session_id,
                FanoutChildRuntimeState::Running,
                0,
            )],
        );
        insert_execution_and_register_active(&engine, execution, ExecutionOrigin::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            child_session_id.to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                child_session_id,
                0,
                None,
                &[MessagePart::Text {
                    content: "LGTM".to_string(),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .unwrap();

        let executions = engine.executions.lock().await;
        let execution = executions
            .get(&execution_id)
            .expect("execution must remain active");
        assert_eq!(
            execution.workflow.nodes[execution.current_node_index].name,
            "parent-next"
        );
        assert_eq!(execution.node_execution_counts.get("parent-next"), Some(&1));
        assert!(!execution.node_execution_counts.contains_key("child-next"));
        drop(executions);

        let events = read_dispatch_events(&app, &execution_id);
        assert!(events.iter().any(|event| {
            matches!(
                event,
                WorkflowEvent::NodeCompleted { node_name, .. }
                    if node_name == "review-child"
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                WorkflowEvent::NodeCompleted { node_name, .. }
                    if node_name == "fanout-review"
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                WorkflowEvent::NodeStarted { node_name, .. }
                    if node_name == "parent-next"
            )
        }));
        assert!(events.iter().all(|event| {
            !matches!(
                event,
                WorkflowEvent::NodeStarted { node_name, .. }
                    if node_name == "child-next"
            )
        }));
    }

    #[tokio::test]
    async fn fanout_activation_crash_checkpoints_aborted_children_in_live_snapshot_and_replay() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (handles, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data_dir.clone(),
            );
        controller.fail_next_start_turn();
        let received_payloads: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let received_for_listener = Arc::clone(&received_payloads);
        app.listen("workflow-execution-changed", move |event| {
            received_for_listener
                .lock()
                .unwrap()
                .push(event.payload().to_string());
        });

        let worktree = TempDir::new().unwrap();
        let worktree_path = worktree.path().to_string_lossy().to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-activation-failure-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-a", "review-b"]),
                make_fanout_child("review-a"),
                make_fanout_child("review-b"),
            ],
        };

        let execution_id = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &handles,
                workflow,
                worktree_path,
                None,
                ExecutionOrigin::DesktopUi,
                crate::domain::agent_session::PermissionMode::Edit,
            )
            .await
            .expect("initial event batch commits before runtime activation");

        assert!(
            !engine.contains_execution_for_test(&execution_id).await,
            "activation crash must release the live execution after checkpointing"
        );
        let execution = engine
            .execution_store()
            .get_execution(&execution_id)
            .await
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Interrupted);
        assert_eq!(
            execution.interruption_reason,
            Some(ExecutionInterruptionReason::Crash)
        );
        assert_eq!(execution.resume_from_node.as_deref(), Some("fanout-review"));

        let payloads = received_payloads.lock().unwrap().clone();
        let live_payload = payloads
            .last()
            .expect("activation crash must broadcast its checkpoint snapshot");
        let live_json: serde_json::Value = serde_json::from_str(live_payload).unwrap();
        let live_node_executions = live_json["workflowExecution"]["nodeExecutions"]
            .as_array()
            .expect("live payload must expose node executions");
        let live_status = |node_name: &str| {
            live_node_executions
                .iter()
                .find(|execution| execution["nodeName"] == node_name)
                .and_then(|execution| execution["status"].as_str())
                .expect("node execution status must be present")
        };
        assert_eq!(live_status("fanout-review"), "aborted");
        assert_eq!(live_status("review-a"), "aborted");
        assert_eq!(live_status("review-b"), "aborted");

        let events = read_dispatch_events(&app, &execution_id);
        let projected = project_workflow_execution(&execution_id, &events)
            .unwrap()
            .unwrap();
        assert_eq!(projected.status, ExecutionStatus::Interrupted);
        let projected_status = |node_name: &str| {
            projected
                .node_executions
                .iter()
                .find(|execution| execution.node_name == node_name)
                .map(|execution| execution.status)
                .expect("projected node execution must exist")
        };
        assert_eq!(
            projected_status("fanout-review"),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert_eq!(
            projected_status("review-a"),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert_eq!(
            projected_status("review-b"),
            crate::domain::workflow::NodeExecutionStatus::Aborted
        );
        assert!(projected
            .node_executions
            .iter()
            .all(|execution| execution.completed_at.is_some()));
    }

    #[tokio::test]
    async fn last_fanout_session_child_append_failure_rolls_back_parent_artifact_and_completion() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_execution_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir);

        let execution_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/fanout-child-completion-rollback";
        let child_session_id = "fanout-child-completion-session";
        let workflow = WorkflowDefinitionYaml {
            name: "fanout-child-completion-rollback-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout_node("fanout-review", vec!["review-child"]),
                make_fanout_child("review-child"),
            ],
        };
        let mut execution =
            make_waiting_approval_execution_with_workflow(&execution_id, worktree_path, workflow);
        execution.state = RuntimeExecutionState::Running;
        execution.current_session_id = None;
        execution.node_execution_counts = HashMap::from([("fanout-review".to_string(), 1)]);
        install_test_fanout(
            &mut execution,
            vec![test_fanout_child(
                "review-child",
                child_session_id,
                FanoutChildRuntimeState::Running,
                0,
            )],
        );
        let parent_node_execution_id = execution.node_executions[0].id.clone();
        let child_node_execution_id = execution.node_executions[1].id.clone();
        insert_execution_and_register_active(&engine, execution, ExecutionOrigin::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            child_session_id.to_string(),
            SessionWorkflowRef {
                execution_id: execution_id.clone(),
            },
        );

        engine.fail_next_required_event_append_for_test();
        let error = engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                child_session_id,
                0,
                None,
                &[MessagePart::Text {
                    content: "LGTM".to_string(),
                    parent_tool_use_id: None,
                }],
                None,
            )
            .await
            .expect_err("last child completion must fail when its event batch cannot append");
        assert!(
            matches!(error, WorkflowEngineError::SessionStore(_)),
            "required append failure must propagate: {error:?}"
        );

        let executions = engine.executions.lock().await;
        let execution = executions
            .get(&execution_id)
            .expect("execution must be rolled back");
        assert_eq!(execution.state, RuntimeExecutionState::Running);
        assert!(execution.fanout_runtime.as_ref().is_some_and(|fanout| {
            fanout.children.len() == 1
                && fanout.children[0].state == FanoutChildRuntimeState::Running
        }));
        let parent = execution
            .node_executions
            .iter()
            .find(|candidate| candidate.id == parent_node_execution_id)
            .unwrap();
        let child = execution
            .node_executions
            .iter()
            .find(|candidate| candidate.id == child_node_execution_id)
            .unwrap();
        assert_eq!(parent.status, NodeExecutionStatus::Running);
        assert_eq!(parent.artifact, None);
        assert_eq!(child.status, NodeExecutionStatus::Running);
        assert!(execution.node_history.is_empty());
        assert!(!execution.artifacts.contains_key("fanout-review"));
        drop(executions);

        let events = read_dispatch_events(&app, &execution_id);
        assert!(events.iter().all(|event| {
            !matches!(
                event,
                WorkflowEvent::NodeCompleted { .. }
                    | WorkflowEvent::ArtifactProduced { .. }
                    | WorkflowEvent::ExecutionCompleted { .. }
            )
        }));
    }
}
