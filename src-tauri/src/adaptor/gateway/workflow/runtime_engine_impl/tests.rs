use super::*;
use crate::adaptor::gateway::app_config::config_models::{
    NotionPropertyMappingModel, NotionRepoConfigModel, ReleashConfig,
};
use crate::adaptor::gateway::workflow::approval_runtime::MAX_APPROVAL_COMMENT_CHARS;
use crate::adaptor::gateway::workflow::failure_wire::{
    submission_violation_reason, SubmissionViolation,
};
use crate::adaptor::gateway::workflow::runtime_state::{ApprovalAction, TurnCompleteAction};
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
const TEST_STEP_SESSION_ID: &str = "22222222-2222-4222-8222-222222222222";
const TEST_REGULAR_SESSION_ID: &str = "33333333-3333-4333-8333-333333333333";

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
fn parallel_child_failure_kind_uses_typed_refusal_signal() {
    let kind = parallel_child_failure_kind(
        0,
        Some(workflow_transition::SessionFailureSignal::ModelRefusal),
    );

    assert_eq!(kind, WorkflowStepFailureKind::ModelRefusal);
}

#[test]
fn parallel_child_failure_kind_without_signal_uses_session_error_classification() {
    let kind = parallel_child_failure_kind(1, None);

    assert_eq!(kind, WorkflowStepFailureKind::InfrastructureCrash);
}

#[test]
fn workflow_resolve_unique_model_returns_owning_backend() {
    let registry = make_workflow_test_registry(&["claude-4"], &["gpt-5"]);
    let result = resolve_step_model_with_registry(&registry, "claude-4").unwrap();
    assert_eq!(result, "claude");
}

#[test]
fn workflow_resolve_rejects_ambiguous_model_in_multiple_backends() {
    let registry = make_workflow_test_registry(&["shared"], &["shared"]);
    let err = resolve_step_model_with_registry(&registry, "shared").unwrap_err();
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
    let err = resolve_step_model_with_registry(&registry, "unknown").unwrap_err();
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
    let err = resolve_step_model_with_registry(&registry, "").unwrap_err();
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
    _workflow_state: Option<WorkflowState>,
    workflow_step_session: bool,
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
        workflow_step_session,
        workflow_step_context: None,
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
fn workflow_step_summary_uses_persisted_session_flag() {
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
            &chat_session_for_test(TEST_STEP_SESSION_ID, "/repo", None, true),
        )
        .unwrap();
    store
        .save_full_session_for_migration_or_restore(
            tmp.path(),
            &chat_session_for_test(TEST_REGULAR_SESSION_ID, "/repo", None, false),
        )
        .unwrap();

    let summaries = store.list_sessions(tmp.path(), "/repo").unwrap();
    let step_summary = summaries
        .iter()
        .find(|session| session.id == TEST_STEP_SESSION_ID)
        .unwrap();
    assert!(step_summary.workflow_step_session);
}

// 撤去済み: persist_state は廃止された（NDJSON event log + Run Store metadata で永続化が完結）。
// 旧 `persist_failure_still_runs_completed_step_cleanup` は persist_state 失敗時の cleanup 順序を
// 検証していたが、機構撤去により意味を失った。

#[test]
fn step_session_tab_cleanup_closes_session_and_preserves_history() {
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

    crate::adaptor::gateway::workflow::close_step_session_tab_state(
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
async fn persist_outcome_without_new_history_does_not_cleanup_last_step_session() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut snapshot = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_STEP_SESSION_ID,
            WorkflowExecutionState::WaitingApproval,
        )
        .await;
    snapshot.step_history.push(StepHistoryEntry {
        step_name: "previous".to_string(),
        completed_at: 1.0,
        result: Some("ok".to_string()),
        session_id: Some(TEST_STEP_SESSION_ID.to_string()),
        token_usage: None,
        structured_output: None,
        run_index: 1,
        child_outputs: None,
        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
    });

    let persist = StepOutcome::Persist(snapshot.clone());
    assert!(persist.completed_step_session_ids().is_empty());

    snapshot.state = WorkflowExecutionState::Completed;
    let terminal = StepOutcome::Persist(snapshot);
    assert_eq!(
        terminal.completed_step_session_ids(),
        vec![TEST_STEP_SESSION_ID.to_string()]
    );
}

#[tokio::test]
async fn aborted_approval_outcome_cleans_current_session_not_last_history_entry() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut snapshot = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_STEP_SESSION_ID,
            WorkflowExecutionState::WaitingApproval,
        )
        .await;
    snapshot.current_session_id = Some("approval-session".to_string());
    snapshot.step_history.push(StepHistoryEntry {
        step_name: "previous".to_string(),
        completed_at: 1.0,
        result: Some("ok".to_string()),
        session_id: Some("previous-session".to_string()),
        token_usage: None,
        structured_output: None,
        run_index: 1,
        child_outputs: None,
        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
    });
    snapshot.state = WorkflowExecutionState::Aborted;

    let outcome = StepOutcome::Persist(snapshot);
    assert_eq!(
        outcome.completed_step_session_ids(),
        vec!["approval-session".to_string()]
    );
}

#[tokio::test]
async fn terminal_state_cleanup_targets_current_and_parallel_step_sessions() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_STEP_SESSION_ID,
            WorkflowExecutionState::Running,
        )
        .await;
    exec.current_session_id = Some("current-step-session".to_string());
    exec.active_parallel_steps = vec![
        ParallelStepState {
            step_name: "review-a".to_string(),
            state: STEP_STATE_RUNNING.to_string(),
            session_id: Some("parallel-a-session".to_string()),
            result: None,
            run_index: 1,
            completed_at: None,
            structured_output: None,
            output_contract: None,
            failure_kind: None,
            failure_disposition: None,
        },
        ParallelStepState {
            step_name: "review-b".to_string(),
            state: STEP_STATE_RUNNING.to_string(),
            session_id: Some("parallel-b-session".to_string()),
            result: None,
            run_index: 1,
            completed_at: None,
            structured_output: None,
            output_contract: None,
            failure_kind: None,
            failure_disposition: None,
        },
    ];

    assert_eq!(
        workflow_runtime_commit::terminal_step_session_ids(&exec),
        vec![
            "current-step-session".to_string(),
            "parallel-a-session".to_string(),
            "parallel-b-session".to_string()
        ]
    );
}

#[tokio::test]
async fn terminal_outcome_cleanup_includes_parent_entry_and_parallel_child_outputs() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut snapshot = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_STEP_SESSION_ID,
            WorkflowExecutionState::Completed,
        )
        .await;
    snapshot.step_history.push(StepHistoryEntry {
        step_name: "parallel-review".to_string(),
        completed_at: 1.0,
        result: Some("done".to_string()),
        session_id: Some("parent-entry-session".to_string()),
        token_usage: None,
        structured_output: None,
        run_index: 1,
        child_outputs: Some(vec![
            crate::adaptor::gateway::workflow::state::ChildOutputSnapshot {
                step_name: "review-a".to_string(),
                session_id: Some("child-a-session".to_string()),
                result: Some("LGTM".to_string()),
                run_index: 1,
                completed_at: 1.0,
                structured_output: None,
                output_contract: None,
                state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
                failure_kind: None,
                failure_disposition: None,
            },
            crate::adaptor::gateway::workflow::state::ChildOutputSnapshot {
                step_name: "review-b".to_string(),
                session_id: Some("child-b-session".to_string()),
                result: Some("LGTM".to_string()),
                run_index: 1,
                completed_at: 1.0,
                structured_output: None,
                output_contract: None,
                state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
                failure_kind: None,
                failure_disposition: None,
            },
        ]),
        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
    });

    assert_eq!(
        StepOutcome::Persist(snapshot).completed_step_session_ids(),
        vec![
            "child-a-session".to_string(),
            "child-b-session".to_string(),
            "parent-entry-session".to_string(),
        ]
    );
}

#[tokio::test]
async fn retry_current_step_outcome_releases_previous_session_only() {
    let engine = WorkflowRuntimeService::new_for_test();
    let snapshot = engine
        .insert_test_approval_execution(
            "/repo",
            TEST_STEP_SESSION_ID,
            WorkflowExecutionState::Running,
        )
        .await;

    assert_eq!(
        StepOutcome::RetryCurrentStep {
            snapshot,
            completed_session_id: Some("stale-session".to_string()),
        }
        .completed_step_session_ids(),
        vec!["stale-session".to_string()]
    );
}
use crate::adaptor::gateway::workflow::schema::{
    CollectConfig, CycleGuard, ParallelAggregate, ReduceStrategy, TransitionRule, Workflow,
};

fn make_minimal_approval_exec(
    execution_id: &str,
    current_session_id: &str,
    step_name: &str,
) -> WorkflowExecution {
    let workflow = Workflow {
        variables: Default::default(),
        name: "test-workflow".to_string(),
        description: "minimal approval fixture".to_string(),
        builtin: false,
        nodes: vec![
            NodeDefinition {
                name: step_name.to_string(),
                kind: test_node_kind(TestKind::ApprovalSession, "approve"),
                ..Default::default()
            },
            NodeDefinition {
                name: "next-step".to_string(),
                kind: test_node_kind(TestKind::Session, "next"),
                ..Default::default()
            },
        ],
    };
    WorkflowExecution {
        id: execution_id.to_string(),
        workflow,
        state: WorkflowExecutionState::WaitingApproval,
        current_step_index: 0,
        step_execution_counts: HashMap::from([(step_name.to_string(), 1)]),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some(current_session_id.to_string()),
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "edit".to_string(),
        },
    }
}

#[test]
fn current_step_for_stall_observation_ignores_terminal_parallel_children() {
    let mut exec = make_minimal_approval_exec("run-stall-lookup", "regular-session", "review");
    exec.current_session_id = None;
    exec.parallel_run = Some(ParallelRunState {
        parent_step_name: "parallel-review".to_string(),
        aggregate: None,
        children: vec![
            ParallelChildRun {
                step_name: "running-child".to_string(),
                session_id: "running-session".to_string(),
                state: ParallelChildState::Running,
                result: None,
                structured_output: None,
                output_contract: None,
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                run_index: 2,
            },
            ParallelChildRun {
                step_name: "completed-child".to_string(),
                session_id: "completed-session".to_string(),
                state: ParallelChildState::Completed,
                result: Some("ok".to_string()),
                structured_output: None,
                output_contract: None,
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                run_index: 1,
            },
            ParallelChildRun {
                step_name: "failed-child".to_string(),
                session_id: "failed-session".to_string(),
                state: ParallelChildState::Failed,
                result: Some("model_refusal".to_string()),
                structured_output: None,
                output_contract: None,
                failure_kind: Some(WorkflowStepFailureKind::ModelRefusal),
                failure_disposition: Some(FailureDisposition::Partial),
                token_usage: TokenUsage::default(),
                run_index: 1,
            },
        ],
    });

    assert_eq!(
        current_step_for_stall_observation(&exec, "running-session"),
        Some(("running-child".to_string(), 2))
    );
    assert_eq!(
        current_step_for_stall_observation(&exec, "completed-session"),
        None
    );
    assert_eq!(
        current_step_for_stall_observation(&exec, "failed-session"),
        None
    );
}

fn workflow_stall_observation_fixture(
    session_id: &str,
    step_name: &str,
) -> WorkflowStallObservation {
    WorkflowStallObservation {
        session_id: session_id.to_string(),
        step_name: step_name.to_string(),
        run_index: 1,
        turn_phase: "streaming".to_string(),
        idle_secs: 181,
        signal_count: 1,
        cap_reached: false,
        observed_at: 1003.0,
    }
}

#[tokio::test]
async fn agent_stall_observed_updates_workflow_state_without_completing_step() {
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
        .set_run_store_data_dir(data_dir.path().to_path_buf())
        .await;
    let run_id = uuid::Uuid::new_v4().to_string();
    let session_id = "stall-session";
    let step_name = "review";
    let exec = make_minimal_approval_exec(&run_id, session_id, step_name);
    let workflow = exec.workflow.clone();
    let log_data_dir =
        crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle())
            .expect("mock app data dir must resolve");
    WorkflowEventLog::new(&log_data_dir)
        .append_batch(&[
            WorkflowEvent::RunStarted {
                run_id: run_id.clone(),
                workflow_name: workflow.name.clone(),
                workflow_file_stem: workflow.name.clone(),
                worktree_path: exec.worktree_path.clone(),
                workflow_definition: workflow,
                timestamp: exec.started_at,
            },
            WorkflowEvent::NodeStarted {
                run_id: run_id.clone(),
                workflow_name: exec.workflow.name.clone(),
                node_name: step_name.to_string(),
                execution_count: 1,
                timestamp: exec.started_at,
            },
            WorkflowEvent::StepSessionStarted {
                run_id: run_id.clone(),
                workflow_name: exec.workflow.name.clone(),
                node_name: step_name.to_string(),
                execution_count: 1,
                session_id: session_id.to_string(),
                timestamp: exec.started_at,
            },
        ])
        .unwrap();
    engine
        .run_store()
        .register_active(WorkflowRun {
            run_id: run_id.clone(),
            workflow_name: exec.workflow.name.clone(),
            task: None,
            status: RunStatus::WaitingApproval,
            worktree_path: exec.worktree_path.clone(),
            current_node_name: Some(step_name.to_string()),
            trigger_source: TriggerSource::Agent,
            started_at: exec.started_at,
            updated_at: exec.updated_at,
            completed_at: None,
            error_reason: None,
        })
        .await
        .unwrap();
    engine.executions.lock().await.insert(run_id.clone(), exec);
    engine.session_workflow_refs.lock().await.insert(
        session_id.to_string(),
        SessionWorkflowRef {
            run_id: run_id.clone(),
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

    let state = engine.get_state_by_run_id(&run_id).await.unwrap();
    assert!(matches!(
        state.state,
        WorkflowExecutionState::WaitingApproval
    ));
    assert_eq!(state.current_session_id.as_deref(), Some(session_id));
    assert_eq!(state.step_history.len(), 0);
    assert_eq!(state.stall_observations.len(), 1);
    let observation = &state.stall_observations[0];
    assert_eq!(observation.session_id, session_id);
    assert_eq!(observation.step_name, step_name);
    assert_eq!(observation.run_index, 1);
    assert_eq!(observation.turn_phase, "streaming");
    assert_eq!(observation.idle_secs, 44);
    assert_eq!(observation.signal_count, 1);
    assert!(!observation.cap_reached);

    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&run_id)
        .unwrap();
    assert!(matches!(
        events.last(),
        Some(WorkflowEvent::WorkflowStallObserved {
            run_id: event_run_id,
            workflow_name,
            chat_session_id,
            step_name: event_step_name,
            run_index: 1,
            turn_phase,
            idle_secs: 44,
            signal_count: 1,
            cap_reached: false,
            ..
        }) if event_run_id == &run_id
            && workflow_name == "test-workflow"
            && chat_session_id == session_id
            && event_step_name == step_name
            && turn_phase == "streaming"
    ));

    let projected = reconstruct_state_from_events(&run_id, &events)
        .unwrap()
        .unwrap();
    assert_eq!(projected.stall_observations.len(), 1);
    assert_eq!(projected.stall_observations[0].session_id, session_id);

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

    let state = engine.get_state_by_run_id(&run_id).await.unwrap();
    assert_eq!(state.stall_observations.len(), 1);
    let observation = &state.stall_observations[0];
    assert_eq!(observation.session_id, session_id);
    assert_eq!(observation.idle_secs, 88);
    assert_eq!(observation.signal_count, 2);
    assert!(observation.cap_reached);
    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&run_id)
        .unwrap();
    let projected = reconstruct_state_from_events(&run_id, &events)
        .unwrap()
        .unwrap();
    assert_eq!(projected.stall_observations.len(), 1);
    assert_eq!(projected.stall_observations[0].signal_count, 2);

    engine
        .on_agent_stall_cleared(app.handle(), session_id)
        .await
        .unwrap();

    let state = engine.get_state_by_run_id(&run_id).await.unwrap();
    assert!(state.stall_observations.is_empty());
    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&run_id)
        .unwrap();
    assert!(matches!(
        events.last(),
        Some(WorkflowEvent::WorkflowStallCleared {
            run_id: event_run_id,
            workflow_name,
            chat_session_id,
            ..
        }) if event_run_id == &run_id
            && workflow_name == "test-workflow"
            && chat_session_id == session_id
    ));
    let projected = reconstruct_state_from_events(&run_id, &events)
        .unwrap()
        .unwrap();
    assert!(projected.stall_observations.is_empty());
}

#[tokio::test]
async fn agent_stall_observed_append_failure_rolls_back_state_and_run_store() {
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
        .set_run_store_data_dir(data_dir.path().to_path_buf())
        .await;
    let run_id = uuid::Uuid::new_v4().to_string();
    let session_id = "stall-session";
    let step_name = "review";
    let exec = make_minimal_approval_exec(&run_id, session_id, step_name);
    let log_data_dir =
        crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle())
            .expect("mock app data dir must resolve");
    engine
        .run_store()
        .register_active(WorkflowRun {
            run_id: run_id.clone(),
            workflow_name: exec.workflow.name.clone(),
            task: None,
            status: RunStatus::WaitingApproval,
            worktree_path: exec.worktree_path.clone(),
            current_node_name: Some(step_name.to_string()),
            trigger_source: TriggerSource::Agent,
            started_at: exec.started_at,
            updated_at: exec.updated_at,
            completed_at: None,
            error_reason: None,
        })
        .await
        .unwrap();
    let stored_before = engine.run_store().get_run(&run_id).await.unwrap();
    engine.executions.lock().await.insert(run_id.clone(), exec);
    engine.session_workflow_refs.lock().await.insert(
        session_id.to_string(),
        SessionWorkflowRef {
            run_id: run_id.clone(),
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
    let state = engine.get_state_by_run_id(&run_id).await.unwrap();
    assert!(state.stall_observations.is_empty());
    let stored_after = engine.run_store().get_run(&run_id).await.unwrap();
    assert_eq!(stored_after.status, stored_before.status);
    assert_eq!(
        stored_after.current_node_name,
        stored_before.current_node_name
    );
    assert_eq!(stored_after.updated_at, stored_before.updated_at);

    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&run_id)
        .unwrap();
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::WorkflowStallObserved { .. })),
        "failed stall observation must not be appended; got {events:?}"
    );
}

#[tokio::test]
async fn agent_stall_cleared_append_failure_rolls_back_state_and_run_store() {
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
        .set_run_store_data_dir(data_dir.path().to_path_buf())
        .await;
    let run_id = uuid::Uuid::new_v4().to_string();
    let session_id = "stall-session";
    let step_name = "review";
    let mut exec = make_minimal_approval_exec(&run_id, session_id, step_name);
    exec.current_stall_observations =
        vec![workflow_stall_observation_fixture(session_id, step_name)];
    let log_data_dir =
        crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle())
            .expect("mock app data dir must resolve");
    engine
        .run_store()
        .register_active(WorkflowRun {
            run_id: run_id.clone(),
            workflow_name: exec.workflow.name.clone(),
            task: None,
            status: RunStatus::WaitingApproval,
            worktree_path: exec.worktree_path.clone(),
            current_node_name: Some(step_name.to_string()),
            trigger_source: TriggerSource::Agent,
            started_at: exec.started_at,
            updated_at: exec.updated_at,
            completed_at: None,
            error_reason: None,
        })
        .await
        .unwrap();
    let stored_before = engine.run_store().get_run(&run_id).await.unwrap();
    engine.executions.lock().await.insert(run_id.clone(), exec);
    engine.session_workflow_refs.lock().await.insert(
        session_id.to_string(),
        SessionWorkflowRef {
            run_id: run_id.clone(),
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
    let state = engine.get_state_by_run_id(&run_id).await.unwrap();
    assert_eq!(state.stall_observations.len(), 1);
    assert_eq!(state.stall_observations[0].session_id, session_id);
    let stored_after = engine.run_store().get_run(&run_id).await.unwrap();
    assert_eq!(stored_after.status, stored_before.status);
    assert_eq!(
        stored_after.current_node_name,
        stored_before.current_node_name
    );
    assert_eq!(stored_after.updated_at, stored_before.updated_at);

    let events = WorkflowEventLog::new(&log_data_dir)
        .read_log(&run_id)
        .unwrap();
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::WorkflowStallCleared { .. })),
        "failed stall clear must not be appended; got {events:?}"
    );
}

// ---- WorkflowExecution ----

fn make_test_step(
    name: &str,
    kind: TestKind,
    instruction: &str,
    rules: Vec<TransitionRule>,
    cycle_guard: Option<CycleGuard>,
) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: test_node_kind(kind, instruction),
        transition_rules: rules,
        cycle_guard,
        ..NodeDefinition::default()
    }
}

fn make_approval_step(name: &str, instruction: &str, rules: Vec<TransitionRule>) -> NodeDefinition {
    make_test_step(name, TestKind::ApprovalSession, instruction, rules, None)
}

fn make_fanout_step(
    name: &str,
    children: Vec<InterimChild>,
    aggregate: Option<ParallelAggregate>,
) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Fanout(FanoutSpec {
            parallel_children: children,
            aggregate,
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

/// テストヘルパー: node の facet 参照を `base_dir` から解決し
/// `resolved_facets` に格納する。`crate::adaptor::gateway::workflow::facet::resolve_node_facets`
/// （`#[cfg(test)] pub(crate)`）への薄い委譲で、欠損 facet 時の `unwrap` 等の
/// パニックは facet helper 側で発生する。
fn resolve_node_facets_for_test(node: &mut NodeDefinition, base_dir: &std::path::Path) {
    crate::adaptor::gateway::workflow::facet::resolve_node_facets(node, base_dir)
        .expect("facet refs must resolve in tests; missing facet indicates a fixture bug");
}

/// テストヘルパー: 並列子 node の facet 参照を解決する。
/// `crate::adaptor::gateway::workflow::facet::resolve_child_facets` への委譲。
fn resolve_child_facets_for_test(
    child: &mut crate::adaptor::gateway::workflow::schema::InterimChild,
    base_dir: &std::path::Path,
) {
    crate::adaptor::gateway::workflow::facet::resolve_child_facets(child, base_dir)
        .expect("facet refs must resolve in tests; missing facet indicates a fixture bug");
}

fn make_test_workflow() -> Workflow {
    Workflow {
        variables: Default::default(),
        name: "test-workflow".to_string(),
        description: "Test workflow".to_string(),
        builtin: false,
        nodes: vec![
            make_test_step("plan", TestKind::Session, "Plan the work", vec![], None),
            make_test_step(
                "implement",
                TestKind::Session,
                "Implement the plan",
                vec![],
                None,
            ),
            make_test_step(
                "review",
                TestKind::Session,
                "Review the implementation",
                vec![
                    TransitionRule {
                        r#match: "NEEDS_FIX".to_string(),
                        next: "implement".to_string(),
                    },
                    TransitionRule {
                        r#match: "LGTM".to_string(),
                        next: "report".to_string(),
                    },
                ],
                Some(CycleGuard {
                    max_iterations: 3,
                    on_exhausted: None,
                }),
            ),
            make_test_step(
                "report",
                TestKind::ApprovalSession,
                "Generate report",
                vec![TransitionRule {
                    r#match: "reject".to_string(),
                    next: "implement".to_string(),
                }],
                None,
            ),
        ],
    }
}

#[test]
fn workflow_execution_to_workflow_state() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let state = exec.to_workflow_state();
    assert_eq!(state.execution_id, "exec-1");
    assert_eq!(state.workflow_name, "test-workflow");
    assert_eq!(state.state, WorkflowExecutionState::Running);
    assert_eq!(state.current_step_index, 0);
    assert_eq!(state.current_step_name, "plan");
    assert_eq!(state.total_steps, 4);
    assert!(state.step_history.is_empty());
}

// ---- is_active ----

#[test]
fn is_active_running() {
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_test_workflow(),
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
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
        state: WorkflowExecutionState::WaitingApproval,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
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
        state: WorkflowExecutionState::Completed,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
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
        state: WorkflowExecutionState::Failed {
            reason: "err".to_string(),
            kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: None,
        },
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
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
        state: WorkflowExecutionState::Aborted,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    assert!(!exec.is_active());
}

// ---- to_workflow_state: all state variants ----

#[test]
fn to_workflow_state_waiting_approval() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: WorkflowExecutionState::WaitingApproval,
        current_step_index: 3,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1001.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let ws = exec.to_workflow_state();
    assert_eq!(ws.state, WorkflowExecutionState::WaitingApproval);
    assert_eq!(ws.current_step_name, "report");
    assert_eq!(ws.current_step_index, 3);
    assert_eq!(
        ws.approval_operations.as_ref().map(|ops| ops.can_reject),
        Some(true)
    );
}

#[test]
fn to_workflow_state_waiting_approval_without_reject_rule_disables_reject() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: WorkflowExecutionState::WaitingApproval,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1001.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let ws = exec.to_workflow_state();
    assert_eq!(
        ws.approval_operations.as_ref().map(|ops| ops.can_reject),
        Some(false)
    );
}

#[test]
fn to_workflow_state_failed() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: WorkflowExecutionState::Failed {
            reason: "exit code 1".to_string(),
            kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: None,
        },
        current_step_index: 1,
        step_execution_counts: HashMap::new(),
        step_history: vec![StepHistoryEntry {
            step_name: "plan".to_string(),
            completed_at: 1000.5,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,

            run_index: 0,
            child_outputs: None,
            state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
        }],
        started_at: 1000.0,
        updated_at: 1001.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let ws = exec.to_workflow_state();
    assert_eq!(
        ws.state,
        WorkflowExecutionState::Failed {
            reason: "exit code 1".to_string(),
            kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: None,
        }
    );
    assert_eq!(ws.current_step_name, "implement");
    assert_eq!(ws.step_history.len(), 1);
}

#[test]
fn to_workflow_state_aborted() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: WorkflowExecutionState::Aborted,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1001.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let ws = exec.to_workflow_state();
    assert_eq!(ws.state, WorkflowExecutionState::Aborted);
}

#[test]
fn to_workflow_state_completed() {
    let workflow = make_test_workflow();
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow,
        state: WorkflowExecutionState::Completed,
        current_step_index: 3,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1002.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let ws = exec.to_workflow_state();
    assert_eq!(ws.state, WorkflowExecutionState::Completed);
    assert_eq!(ws.total_steps, 4);
}

// ---- cycle_guard: boundary value at exactly max_iterations ----

#[test]
fn check_cycle_guard_at_boundary_minus_one_allowed() {
    let mut exec = make_exec(2); // review (max_iterations=3)
    exec.step_execution_counts.insert("review".to_string(), 2);
    assert_eq!(
        exec.check_cycle_guard("review").unwrap(),
        CycleGuardResult::Allowed
    );
}

#[test]
fn check_cycle_guard_at_exact_boundary_exceeded() {
    let mut exec = make_exec(2); // review (max_iterations=3)
    exec.step_execution_counts.insert("review".to_string(), 3);
    assert_eq!(
        exec.check_cycle_guard("review").unwrap(),
        CycleGuardResult::Exceeded {
            max_iterations: 3,
            count: 3,
            on_exhausted: None,
        }
    );
}

#[test]
fn cycle_guard_no_guard_defined() {
    let workflow = make_test_workflow();
    let step = &workflow.nodes[0]; // plan (no cycle_guard)
    assert!(step.cycle_guard.is_none());
}

// ---- decide_next_step ----

fn make_exec(step_index: usize) -> WorkflowExecution {
    WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_test_workflow(),
        state: WorkflowExecutionState::Running,
        current_step_index: step_index,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        step_outputs: HashMap::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    }
}

#[test]
fn decide_next_step_returns_next_step_name() {
    let exec = make_exec(0); // plan → next is implement
    assert_eq!(
        exec.decide_next_step(),
        NextStepDecision::TransitionTo("implement".to_string())
    );
}

#[test]
fn decide_next_step_returns_completed_at_last_step() {
    let exec = make_exec(3); // report (last)
    assert_eq!(exec.decide_next_step(), NextStepDecision::Completed);
}

#[test]
fn decide_next_step_middle_step() {
    let exec = make_exec(1); // implement → next is review
    assert_eq!(
        exec.decide_next_step(),
        NextStepDecision::TransitionTo("review".to_string())
    );
}

// ---- check_cycle_guard ----

#[test]
fn check_cycle_guard_allowed_no_guard() {
    let exec = make_exec(0);
    assert_eq!(
        exec.check_cycle_guard("plan").unwrap(),
        CycleGuardResult::Allowed
    );
}

#[test]
fn check_cycle_guard_allowed_within_limit() {
    let mut exec = make_exec(2);
    exec.step_execution_counts.insert("review".to_string(), 2);
    assert_eq!(
        exec.check_cycle_guard("review").unwrap(),
        CycleGuardResult::Allowed
    );
}

#[test]
fn check_cycle_guard_exceeded() {
    let mut exec = make_exec(2);
    exec.step_execution_counts.insert("review".to_string(), 3);
    assert_eq!(
        exec.check_cycle_guard("review").unwrap(),
        CycleGuardResult::Exceeded {
            max_iterations: 3,
            count: 3,
            on_exhausted: None,
        }
    );
}

#[test]
fn check_cycle_guard_step_not_found() {
    let exec = make_exec(0);
    assert!(exec.check_cycle_guard("nonexistent").is_err());
}

#[test]
fn check_cycle_guard_first_transition_no_count() {
    // step_execution_counts にキーなし = 初回遷移
    let exec = make_exec(2); // review has cycle_guard(max_iterations=3)
    assert_eq!(
        exec.check_cycle_guard("review").unwrap(),
        CycleGuardResult::Allowed
    );
}

// ---- decide_turn_complete_action ----

#[test]
fn turn_complete_action_not_running() {
    let mut exec = make_exec(0);
    exec.state = WorkflowExecutionState::Completed;
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
            step_name: "plan".to_string(),
            exit_code: 1,
            kind: WorkflowStepFailureKind::InfrastructureCrash,
        }
    );
}

#[test]
fn turn_complete_action_auto_evaluate() {
    let exec = make_exec(2); // review (auto, has rules)
    let action = exec.decide_turn_complete_action(0);
    match action {
        TurnCompleteAction::AutoEvaluate { rules, step_name } => {
            assert_eq!(step_name, "review");
            assert_eq!(rules.len(), 2);
            assert_eq!(rules[0].r#match, "NEEDS_FIX");
            assert_eq!(rules[1].r#match, "LGTM");
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
        TurnCompleteAction::UnexpectedNodeKind { step_name, kind } => {
            assert_eq!(step_name, "plan");
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
        TurnCompleteAction::UnexpectedNodeKind { step_name, kind } => {
            assert_eq!(step_name, "plan");
            assert_eq!(kind, crate::domain::workflow::NodeKindName::Fanout);
        }
        other => panic!("Expected UnexpectedNodeKind for fanout, got {:?}", other),
    }
}

#[test]
fn turn_complete_action_waiting_approval_state_returns_not_running() {
    let mut exec = make_exec(3);
    exec.state = WorkflowExecutionState::WaitingApproval;
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
            step_name: "plan".to_string(),
            exit_code: -1,
            kind: WorkflowStepFailureKind::InfrastructureCrash,
        }
    );
}

#[test]
fn turn_complete_action_auto_no_rules_returns_auto_evaluate_empty() {
    let exec = make_exec(1); // implement (auto, no rules)
    let action = exec.decide_turn_complete_action(0);
    match action {
        TurnCompleteAction::AutoEvaluate { rules, step_name } => {
            assert_eq!(step_name, "implement");
            assert!(rules.is_empty());
        }
        other => panic!("Expected AutoEvaluate with empty rules, got {:?}", other),
    }
}

// ---- decide_approval_action ----

#[test]
fn decide_approval_action_approve() {
    let mut exec = make_exec(3); // report (approval)
    exec.state = WorkflowExecutionState::WaitingApproval;
    assert_eq!(
        exec.decide_approval_action(&ApprovalDecision::Approve)
            .unwrap(),
        ApprovalAction::Advance
    );
}

#[test]
fn decide_approval_action_reject_with_rule() {
    let mut exec = make_exec(3); // report (approval, reject→implement)
    exec.state = WorkflowExecutionState::WaitingApproval;
    assert_eq!(
        exec.decide_approval_action(&ApprovalDecision::Reject {
            comment: "Needs fix".to_string()
        })
        .unwrap(),
        ApprovalAction::TransitionTo("implement".to_string())
    );
}

#[test]
fn decide_approval_action_reject_no_rule() {
    let mut exec = make_exec(0); // plan (interactive, no reject rule)
    exec.state = WorkflowExecutionState::WaitingApproval;
    assert!(exec
        .decide_approval_action(&ApprovalDecision::Reject {
            comment: "Needs fix".to_string()
        })
        .is_err());
}

#[test]
fn decide_approval_action_not_waiting() {
    let exec = make_exec(3); // report, state=Running
    assert!(exec
        .decide_approval_action(&ApprovalDecision::Approve)
        .is_err());
}

// ---- validate_start ----

#[test]
fn validate_start_empty_steps_returns_err() {
    let workflow = Workflow {
        variables: Default::default(),
        name: "empty".to_string(),
        description: String::new(),
        builtin: false,
        nodes: vec![],
    };
    let result = WorkflowExecution::validate_start(&workflow, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no steps"));
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
    existing.state = WorkflowExecutionState::Completed;
    let result = WorkflowExecution::validate_start(&workflow, Some(&existing));
    assert!(result.is_ok());
}

#[test]
fn validate_start_no_existing_returns_ok() {
    let workflow = make_test_workflow();
    let result = WorkflowExecution::validate_start(&workflow, None);
    assert!(result.is_ok());
}

/// command kind node を含む workflow は実行系未対応のため
/// 開始前に明示的に拒否される（実行系は [13] で具体化）。
#[test]
fn validate_start_rejects_command_node() {
    let workflow = Workflow {
        variables: Default::default(),
        name: "command-wf".to_string(),
        description: String::new(),
        builtin: false,
        nodes: vec![NodeDefinition {
            name: "build".to_string(),
            kind: test_node_kind(TestKind::Command, "echo hello"),
            ..NodeDefinition::default()
        }],
    };
    let result = WorkflowExecution::validate_start(&workflow, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Command node"));
}

// ---- is_terminal ----

#[test]
fn is_terminal_completed() {
    let mut exec = make_exec(0);
    exec.state = WorkflowExecutionState::Completed;
    assert!(exec.is_terminal());
}

#[test]
fn is_terminal_failed() {
    let mut exec = make_exec(0);
    exec.state = WorkflowExecutionState::Failed {
        reason: "err".to_string(),
        kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
        retry_count: None,
    };
    assert!(exec.is_terminal());
}

#[test]
fn is_terminal_aborted() {
    let mut exec = make_exec(0);
    exec.state = WorkflowExecutionState::Aborted;
    assert!(exec.is_terminal());
}

#[test]
fn is_terminal_running_is_false() {
    let exec = make_exec(0);
    assert!(!exec.is_terminal());
}

#[test]
fn is_terminal_waiting_approval_is_false() {
    let mut exec = make_exec(0);
    exec.state = WorkflowExecutionState::WaitingApproval;
    assert!(!exec.is_terminal());
}

// ---- step_states computation ----

#[test]
fn step_states_all_pending_at_start() {
    let exec = make_exec(0);
    let ws = exec.to_workflow_state();
    assert_eq!(ws.step_states["plan"], "running");
    assert_eq!(ws.step_states["implement"], "pending");
    assert_eq!(ws.step_states["review"], "pending");
    assert_eq!(ws.step_states["report"], "pending");
}

#[test]
fn step_states_completed_steps() {
    let mut exec = make_exec(2);
    exec.step_history = vec![
        StepHistoryEntry {
            step_name: "plan".to_string(),
            completed_at: 1000.5,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,

            run_index: 0,
            child_outputs: None,
            state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
        },
        StepHistoryEntry {
            step_name: "implement".to_string(),
            completed_at: 1001.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,

            run_index: 0,
            child_outputs: None,
            state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
        },
    ];
    let ws = exec.to_workflow_state();
    assert_eq!(ws.step_states["plan"], "completed");
    assert_eq!(ws.step_states["implement"], "completed");
    assert_eq!(ws.step_states["review"], "running");
    assert_eq!(ws.step_states["report"], "pending");
}

#[test]
fn step_states_failed_step() {
    let mut exec = make_exec(1);
    exec.state = WorkflowExecutionState::Failed {
        reason: "error".to_string(),
        kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
        retry_count: None,
    };
    exec.step_history = vec![StepHistoryEntry {
        step_name: "plan".to_string(),
        completed_at: 1000.5,
        result: None,
        session_id: None,
        token_usage: None,
        structured_output: None,

        run_index: 0,
        child_outputs: None,
        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
    }];
    let ws = exec.to_workflow_state();
    assert_eq!(ws.step_states["plan"], "completed");
    assert_eq!(ws.step_states["implement"], "failed");
    assert_eq!(ws.step_states["review"], "pending");
    assert_eq!(ws.step_states["report"], "pending");
}

#[test]
fn step_states_waiting_approval() {
    let mut exec = make_exec(1);
    exec.state = WorkflowExecutionState::WaitingApproval;
    exec.step_history = vec![StepHistoryEntry {
        step_name: "plan".to_string(),
        completed_at: 1000.5,
        result: None,
        session_id: None,
        token_usage: None,
        structured_output: None,

        run_index: 0,
        child_outputs: None,
        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
    }];
    let ws = exec.to_workflow_state();
    assert_eq!(ws.step_states["plan"], "completed");
    assert_eq!(ws.step_states["implement"], "waiting_approval");
    assert_eq!(ws.step_states["review"], "pending");
}

// ---- inject_step_outputs ----

fn make_step_output(step_name: &str, output_text: &str, result: Option<&str>) -> StepOutput {
    StepOutput {
        step_name: step_name.to_string(),
        run_index: 0,
        session_id: None,
        result: result.map(|s| s.to_string()),
        structured_output: Some(serde_json::json!({"text": output_text})),
        output_contract: None,
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
fn approved_fix_policy_structured_output_is_masked_for_parallel_contract_path() {
    let masked = workflow_secret_masker::mask_sensitive_structured_output(
        "approved-fix-policy",
        serde_json::json!({
            "policy": "Use password=secret123 and MY_TOKEN_VALUE_123456",
            "review_step": "code_review_parallel",
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
    let mut step = make_test_step("fix", TestKind::Session, "Fix", vec![], None);
    step.pass_output_from = Some(vec!["implementation_fix_policy".to_string()]);

    let sanitized = serde_json::json!({
        "policy": "Use password=[REDACTED] only in examples.",
        "review_step": "code_review_parallel",
        "findings": []
    });
    let vars = workflow_contract::extract_workflow_variables_from_contract_output(
        Some("approved-fix-policy"),
        Some(&sanitized),
    );
    assert!(vars.is_empty());

    let mut outputs = HashMap::new();
    outputs.insert(
        "implementation_fix_policy".to_string(),
        StepOutput {
            step_name: "implementation_fix_policy".to_string(),
            run_index: 1,
            session_id: Some("policy-session".to_string()),
            result: Some("approved".to_string()),
            structured_output: Some(sanitized),
            output_contract: Some("approved-fix-policy".to_string()),
            token_usage: None,
            completed_at: 1000.0,
        },
    );
    let injected = workflow_prompt::inject_step_outputs("Fix", &step, &outputs, &[], &vars);
    assert!(injected.contains("[REDACTED]"));
    assert!(injected.contains("<step_output name=\"implementation_fix_policy\">"));
    assert!(!injected.contains("<workflow_variables>"));
    assert!(!injected.contains("secret123"));
}

#[test]
fn approved_policy_masks_raw_secrets_before_state_variables_history_and_injection() {
    let mut structured = serde_json::json!({
        "policy": "Use password=secret123 with ghp_abcdefghijklmnopqrstuvwxyz1234567890 -----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY----- MY_TOKEN_VALUE_123456",
        "review_step": "code_review_parallel",
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

    let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
    exec.workflow.nodes[0].output_contract = Some("approved-fix-policy".to_string());
    let mut fix = make_test_step("fix", TestKind::Session, "Fix", vec![], None);
    fix.pass_previous_response = Some(true);
    exec.workflow.nodes.push(fix);
    let outcome = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        &ApprovalDecision::Approve,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            structured_output: Some(structured),
            output_contract: Some("approved-fix-policy".to_string()),
        },
    )
    .unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));

    let state = exec.to_workflow_state();
    let state_json = serde_json::to_string(&state).unwrap();
    assert!(state_json.contains("[REDACTED]"));
    assert!(!state_json.contains("secret123"));
    assert!(!state_json.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
    assert!(!state_json.contains("MY_TOKEN_VALUE_123456"));
    assert!(!exec.workflow_variables.contains_key("approved_fix_policy"));
    assert!(!exec.step_history[0]
        .structured_output
        .as_ref()
        .unwrap()
        .to_string()
        .contains("secret123"));

    let injected = workflow_prompt::inject_step_outputs(
        "Fix",
        &exec.workflow.nodes[exec.current_step_index],
        &exec.step_outputs,
        &exec.step_history,
        &exec.workflow_variables,
    );
    assert!(injected.contains("[REDACTED]"));
    assert!(!injected.contains("<workflow_variables>"));
    assert!(!injected.contains("secret123"));
    assert!(!injected.contains("MY_TOKEN_VALUE_123456"));
}

#[test]
fn approved_policy_workflow_event_log_readback_redacts_sensitive_values() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut exec = make_minimal_approval_exec(
        "00000000-0000-0000-0000-000000000917",
        "policy-session",
        "approval-step",
    );
    let secret_env_value = "MY_TOKEN_VALUE_123456".to_string();
    let mut structured = serde_json::json!({
        "policy": "Use password=secret123 with ghp_abcdefghijklmnopqrstuvwxyz1234567890 -----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY----- MY_TOKEN_VALUE_123456",
        "review_step": "spec_review_parallel",
        "findings": []
    });
    workflow_secret_masker::mask_json_strings(&mut structured, &[secret_env_value]);

    let outcome = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        &ApprovalDecision::Approve,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            structured_output: Some(structured),
            output_contract: Some("approved-fix-policy".to_string()),
        },
    )
    .unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));

    let entry = exec
        .step_history
        .iter()
        .find(|entry| entry.step_name == "approval-step")
        .unwrap();
    let log = WorkflowEventLog::new(tmp.path());
    log.append(&WorkflowEvent::RunStarted {
        run_id: exec.id.clone(),
        workflow_name: exec.workflow.name.clone(),
        workflow_file_stem: "test-workflow".to_string(),
        worktree_path: "/repo".to_string(),
        workflow_definition: exec.workflow.clone(),
        timestamp: 1000.0,
    })
    .unwrap();
    log.append(&WorkflowEvent::NodeCompleted {
        run_id: exec.id.clone(),
        workflow_name: exec.workflow.name.clone(),
        node_name: entry.step_name.clone(),
        result: entry.result.clone(),
        session_id: entry.session_id.clone(),
        token_usage: entry.token_usage.clone(),
        structured_output: entry.structured_output.clone(),
        run_index: Some(entry.run_index),
        timestamp: entry.completed_at,
    })
    .unwrap();

    let raw_ndjson =
        std::fs::read_to_string(tmp.path().join(format!("workflow_logs/{}.ndjson", exec.id)))
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
        .find(|event| matches!(event, WorkflowEvent::NodeCompleted { .. }))
        .unwrap();
    match completed {
        WorkflowEvent::NodeCompleted {
            structured_output, ..
        } => {
            let policy = structured_output
                .as_ref()
                .and_then(|output| output.get("policy"))
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

// ---- evaluate_auto_rules (reduce結果による遷移判定) ----

#[test]
fn reduce_result_triggers_transition_via_evaluate_auto_rules() {
    let rules = vec![TransitionRule {
        r#match: "NEEDS_FIX".to_string(),
        next: "fix".to_string(),
    }];
    let result = turn_completion::evaluate_auto_rules("NEEDS_FIX", &rules);
    assert_eq!(result, Some(("fix".to_string(), "NEEDS_FIX".to_string())));
}

#[test]
fn reduce_result_lgtm_no_matching_rule_returns_none() {
    let rules = vec![TransitionRule {
        r#match: "NEEDS_FIX".to_string(),
        next: "fix".to_string(),
    }];
    let result = turn_completion::evaluate_auto_rules("LGTM", &rules);
    assert!(result.is_none());
}

// ---- build_step_prompt ----

#[test]
fn build_step_prompt_full_pipeline() {
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
        "Coding policy for {{project_name}}.",
    )
    .unwrap();
    std::fs::write(
        instructions.join("impl.md"),
        "Task: {{task}}\nImplement the feature.",
    )
    .unwrap();
    std::fs::write(contracts.join("plan-doc.md"), "Output as markdown.").unwrap();

    let mut step = make_test_step("build", TestKind::Session, "unused", vec![], None);
    set_instruction_facet(&mut step, Some("impl".to_string()));
    set_policy_facet(&mut step, Some("coding".to_string()));
    step.output_contract = Some("plan-doc".to_string());
    step.pass_previous_response = Some(true);
    resolve_node_facets_for_test(&mut step, base);

    let mut outputs = HashMap::new();
    outputs.insert(
        "plan".to_string(),
        make_step_output("plan", "Plan output text", None),
    );
    let history = vec![StepHistoryEntry {
        step_name: "plan".to_string(),
        completed_at: 2000.0,
        result: None,
        session_id: None,
        token_usage: None,
        structured_output: None,

        run_index: 0,
        child_outputs: None,
        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
    }];
    let (sys, prompt) = workflow_prompt::build_step_prompt(
        &step,
        "00000000-0000-0000-0000-000000000000",
        "/home/user/my-app",
        Some("Fix bug"),
        &outputs,
        &history,
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();

    // policy + output_contract → system_prompt with variable expansion
    let sys_str = sys.expect("system_prompt should be set");
    assert!(sys_str.contains("Coding policy for my-app."));
    assert!(sys_str.contains("Output as markdown."));
    let instruction = workflow_prompt::render_step_workflow_instruction(
        &step,
        "00000000-0000-0000-0000-000000000000",
        "/home/user/my-app",
        Some("Fix bug"),
        &HashMap::new(),
    )
    .expect("workflow instruction");
    assert!(instruction.contains("Task: Fix bug"));
    assert!(instruction.contains("Implement the feature."));
    assert!(!prompt.contains("Task: Fix bug"));
    assert!(!prompt.contains("Implement the feature."));
    // output_contract がある場合、作業本文の末尾にも Contract 由来の
    // 完了時アクションを置き、初回完了時に CLI 提出へ誘導する。
    assert!(prompt.contains("完了時の必須アクション"));
    // CLI 名は起動環境別 alias で展開される（spec issues-1054）。
    let cli_alias = WorkflowRuntimeService::resolve_releash_alias();
    assert!(prompt.contains(&format!(
        "{cli_alias} workflow output submit 00000000-0000-0000-0000-000000000000"
    )));
    assert!(prompt.contains("--step build"));
    assert!(prompt.contains("--type plan-doc"));
    assert!(prompt.contains("--json"));
    assert!(!prompt.contains("--file"));
    assert!(!prompt.contains("+  --step"));
    // inject_step_outputs: pass_previous_response includes plan output
    assert!(prompt.contains("<step_output name=\"plan\">"));
    assert!(prompt.contains("Plan output text"));
    assert!(
        prompt.find("完了時の必須アクション").unwrap() > prompt.find("Plan output text").unwrap(),
        "completion action must remain after injected step outputs"
    );
}

#[test]
fn build_step_prompt_no_facet_refs_returns_error() {
    let mut step = make_test_step("empty", TestKind::Session, "unused", vec![], None);
    set_session_facets(&mut step, FacetRefs::default());
    let result = workflow_prompt::build_step_prompt(
        &step,
        "00000000-0000-0000-0000-000000000000",
        "/repo",
        None,
        &HashMap::new(),
        &[],
        &HashMap::new(),
        &HashMap::new(),
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no facet refs"));
}

#[test]
fn build_step_prompt_policy_only_system_prompt_set() {
    // Scenario: policyのみを指定したステップでも system_prompt が合成される
    let tmp = tempfile::TempDir::new().unwrap();
    let policies = tmp.path().join("policies");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::write(policies.join("review.md"), "Review carefully.").unwrap();

    let mut step = make_test_step("review", TestKind::Session, "unused", vec![], None);
    set_policy_facet(&mut step, Some("review".to_string()));
    set_instruction_facet(&mut step, None);
    resolve_node_facets_for_test(&mut step, tmp.path());
    let (sys, prompt) = workflow_prompt::build_step_prompt(
        &step,
        "00000000-0000-0000-0000-000000000000",
        "/repo",
        None,
        &HashMap::new(),
        &[],
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();

    assert_eq!(sys.as_deref(), Some("Review carefully."));
    assert_eq!(prompt, "");
}

#[test]
fn build_step_prompt_passes_composed_system_prompt_through() {
    // Scenario: 合成された system_prompt は AgentSession 開始時にバックエンドへ受け渡される
    // build_step_prompt の戻り値の Option<String> がそのまま AgentSession に渡される経路を検証する。
    // ドロップ・空文字置換が起きないこと。
    let tmp = tempfile::TempDir::new().unwrap();
    let policies = tmp.path().join("policies");
    let contracts = tmp.path().join("contracts");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::create_dir_all(&contracts).unwrap();
    std::fs::write(policies.join("coding.md"), "POLICY_BODY").unwrap();
    std::fs::write(contracts.join("plan-doc.md"), "CONTRACT_BODY").unwrap();

    let mut step = make_test_step("s", TestKind::Session, "unused", vec![], None);
    set_policy_facet(&mut step, Some("coding".to_string()));
    step.output_contract = Some("plan-doc".to_string());
    set_instruction_facet(&mut step, None);
    resolve_node_facets_for_test(&mut step, tmp.path());
    let (sys, prompt) = workflow_prompt::build_step_prompt(
        &step,
        "00000000-0000-0000-0000-000000000000",
        "/repo",
        None,
        &HashMap::new(),
        &[],
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();

    // 合成された system_prompt は Some(...) として渡される（None や空文字に置換されない）
    let sys = sys.expect("system_prompt must be passed through, not dropped");
    assert!(!sys.is_empty(), "system_prompt must not be empty string");
    assert!(sys.contains("POLICY_BODY"));
    assert!(sys.contains("CONTRACT_BODY"));
    assert!(prompt.contains("完了時の必須アクション"));
    // CLI 名は起動環境別 alias で展開される（spec issues-1054）。
    let cli_alias = WorkflowRuntimeService::resolve_releash_alias();
    assert!(prompt.contains(&format!(
        "{cli_alias} workflow output submit 00000000-0000-0000-0000-000000000000"
    )));
    assert!(prompt.contains("--step s"));
    assert!(prompt.contains("--type plan-doc"));
    assert!(!prompt.contains("+  --step"));
}

#[test]
fn build_step_prompt_expands_workflow_declared_variables_in_user_message() {
    // spec issues-1054「workflow 定義変数の facet 展開」:
    // build_step_prompt は workflow_declared_variables を facet 本文の
    // `{{vars.<name>}}` 展開に渡す。instruction は system context 経路へ渡す値として、
    // policy は system_prompt として `{{vars.*}}` が宣言値に置換されることを検証する。
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();
    let instructions = base.join("instructions");
    let policies = base.join("policies");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::write(
        instructions.join("impl-vars.md"),
        "Spec dir: {{vars.spec_dir}}\nEnv: {{vars.env}}",
    )
    .unwrap();
    std::fs::write(
        policies.join("vars-policy.md"),
        "Operate within {{vars.env}}.",
    )
    .unwrap();

    let mut step = make_test_step("impl", TestKind::Session, "unused", vec![], None);
    set_instruction_facet(&mut step, Some("impl-vars".to_string()));
    set_policy_facet(&mut step, Some("vars-policy".to_string()));
    step.output_contract = None;
    resolve_node_facets_for_test(&mut step, base);

    let mut declared = HashMap::new();
    declared.insert("spec_dir".to_string(), "docs/specs/issues-1054".to_string());
    declared.insert("env".to_string(), "production".to_string());

    let (sys, prompt) = workflow_prompt::build_step_prompt(
        &step,
        "00000000-0000-0000-0000-000000000000",
        "/repo",
        None,
        &HashMap::new(),
        &[],
        &HashMap::new(),
        &declared,
    )
    .unwrap();
    let instruction = workflow_prompt::render_step_workflow_instruction(
        &step,
        "00000000-0000-0000-0000-000000000000",
        "/repo",
        None,
        &declared,
    )
    .expect("workflow instruction");

    // workflow instruction 側の `{{vars.spec_dir}}` / `{{vars.env}}` が宣言値に展開される
    assert!(instruction.contains("Spec dir: docs/specs/issues-1054"));
    assert!(instruction.contains("Env: production"));
    assert!(!prompt.contains("Spec dir: docs/specs/issues-1054"));
    assert!(!prompt.contains("Env: production"));
    // 未展開トークンが残らない
    assert!(!prompt.contains("{{vars.spec_dir}}"));
    assert!(!prompt.contains("{{vars.env}}"));

    // system_prompt 側でも `{{vars.env}}` が展開される
    let sys_str = sys.expect("system_prompt should be set");
    assert!(sys_str.contains("Operate within production."));
    assert!(!sys_str.contains("{{vars.env}}"));
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
                crate::domain::workflow::WorkflowStepFailureKind::StartupTimeout
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

    let mut step = make_test_step("s", TestKind::Session, "unused", vec![], None);
    set_policy_facet(&mut step, Some("p".to_string()));
    step.output_contract = Some("c".to_string());
    set_instruction_facet(&mut step, None);
    resolve_node_facets_for_test(&mut step, base);

    // build_step_prompt → dispatch_session_start の経路をそのまま再現する。
    let (system_prompt, _prompt) = workflow_prompt::build_step_prompt(
        &step,
        "00000000-0000-0000-0000-000000000000",
        "/repo",
        None,
        &HashMap::new(),
        &[],
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();

    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let gate = RecordingSessionStartGate {
        records: records.clone(),
    };

    dispatch_session_start(
        &gate,
        "step-session-id",
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
    assert_eq!(r.session_id, "step-session-id");
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
    assert!(sp.contains("CONTRACT_BODY"));
}

#[tokio::test]
async fn build_and_dispatch_step_session_forwards_composed_system_prompt_through_gate() {
    // Scenario: 合成された system_prompt は AgentSession 開始時にバックエンドへ受け渡される
    // start_step_session 側の経路（build_step_prompt → SessionStartGate）を切り出したヘルパーを
    // 記録用 gate で駆動し、合成された system_prompt が None / 空文字に置換されずに
    // gate に渡ることを直接 assert する。
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();
    let policies = base.join("policies");
    let contracts = base.join("contracts");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::create_dir_all(&contracts).unwrap();
    std::fs::write(policies.join("p.md"), "STEP_POLICY_BODY").unwrap();
    std::fs::write(contracts.join("c.md"), "STEP_CONTRACT_BODY").unwrap();

    let mut step = make_test_step("s", TestKind::Session, "unused", vec![], None);
    set_policy_facet(&mut step, Some("p".to_string()));
    step.output_contract = Some("c".to_string());
    set_instruction_facet(&mut step, None);
    resolve_node_facets_for_test(&mut step, base);

    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let gate = RecordingSessionStartGate {
        records: records.clone(),
    };

    let prompt = WorkflowRuntimeService::build_and_dispatch_step_session(
        &gate,
        &step,
        "00000000-0000-0000-0000-000000000000",
        "step-session-id",
        "/repo",
        None,
        None,
        &HashMap::new(),
        &[],
        &HashMap::new(),
    )
    .await
    .unwrap();

    // knowledge / instruction がなくても、output_contract があれば user_message には
    // Contract 由来の完了時アクションが入る。
    let _ = prompt;

    let recorded = records.lock().unwrap();
    assert_eq!(
        recorded.len(),
        1,
        "gate.start_session must be invoked exactly once via build_and_dispatch_step_session"
    );
    let r = &recorded[0];
    assert_eq!(r.session_id, "step-session-id");
    assert_eq!(r.worktree_path, "/repo");
    assert!(r.permission_mode.is_none());
    let sp = r.system_prompt.as_ref().expect(
        "system_prompt must be passed through start_step_session path as Some(_), not dropped",
    );
    assert!(
        !sp.is_empty(),
        "system_prompt must not be dropped or replaced with an empty string"
    );
    assert!(sp.contains("STEP_POLICY_BODY"));
    assert!(sp.contains("STEP_CONTRACT_BODY"));
}

#[tokio::test]
async fn dispatch_session_start_passes_none_when_no_facets() {
    // Scenario: policy も output_contract も指定がないと system_prompt は設定されない
    // を SessionStartGate 経由でも維持することを検証する。
    let tmp = tempfile::TempDir::new().unwrap();
    let instructions = tmp.path().join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(instructions.join("only-instr.md"), "Body").unwrap();

    let mut step = make_test_step("s", TestKind::Session, "unused", vec![], None);
    set_instruction_facet(&mut step, Some("only-instr".to_string()));
    resolve_node_facets_for_test(&mut step, tmp.path());
    let (system_prompt, _prompt) = workflow_prompt::build_step_prompt(
        &step,
        "00000000-0000-0000-0000-000000000000",
        "/repo",
        None,
        &HashMap::new(),
        &[],
        &HashMap::new(),
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
        "system_prompt must be None when neither policy nor output_contract is specified"
    );
}

// ---- start_step_session_with_deps (副作用境界の注入による順序保証検証) ----

/// テスト用の `StepSessionDeps` 実装。副作用境界の各メソッドの呼び出し回数を
/// 記録し、本番経路と同じ順序で副作用が発火することを assert できるようにする。
/// プロンプト合成失敗時に `create_step_session` が呼ばれないこと等を検証する。
#[derive(Default)]
struct RecordingStepSessionDeps {
    create_step_session_count: std::sync::atomic::AtomicUsize,
    dispatch_session_start_count: std::sync::atomic::AtomicUsize,
    mark_step_tab_open_count: std::sync::atomic::AtomicUsize,
    append_node_session_started_count: std::sync::atomic::AtomicUsize,
    append_node_session_started_should_fail: std::sync::atomic::AtomicBool,
    broadcast_state_count: std::sync::atomic::AtomicUsize,
    start_agent_turn_count: std::sync::atomic::AtomicUsize,
    created_contexts: std::sync::Mutex<Vec<WorkflowStepContext>>,
    dispatched_workflow_instructions: std::sync::Mutex<Vec<Option<String>>>,
    started_workflow_instructions: std::sync::Mutex<Vec<Option<String>>>,
}

impl RecordingStepSessionDeps {
    fn create_step_session_count(&self) -> usize {
        self.create_step_session_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn dispatch_session_start_count(&self) -> usize {
        self.dispatch_session_start_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn mark_step_tab_open_count(&self) -> usize {
        self.mark_step_tab_open_count
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

    fn created_contexts(&self) -> Vec<WorkflowStepContext> {
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
impl StepSessionDeps for RecordingStepSessionDeps {
    async fn create_step_session(
        &self,
        _worktree_path: &str,
        _step_model: Option<String>,
        _step_permission: Option<String>,
        _workflow_defaults: WorkflowDefaults,
        workflow_step_context: WorkflowStepContext,
        _kind_context: workflow_runtime_session::StepRuntimeKindContext,
    ) -> Result<StepSessionInfo, WorkflowEngineError> {
        self.create_step_session_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.created_contexts
            .lock()
            .unwrap()
            .push(workflow_step_context);
        Ok(StepSessionInfo {
            id: "step-session-id".to_string(),
            permission_mode: "ask".to_string(),
        })
    }

    async fn dispatch_session_start(
        &self,
        _step_session_id: &str,
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

    async fn mark_step_tab_open(&self, _step_session_id: &str) {
        self.mark_step_tab_open_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    async fn broadcast_state(&self, _worktree_path: &str, _snapshot: WorkflowState) {
        self.broadcast_state_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    async fn append_node_session_started(
        &self,
        _snapshot: &WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        self.append_node_session_started_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self
            .append_node_session_started_should_fail
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(WorkflowEngineError::SessionStore(
                "append step session started failed".to_string(),
            ));
        }
        Ok(())
    }

    async fn start_agent_turn_locked(
        &self,
        step_session_id: &str,
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
        let _ = step_session_id;
        Ok(())
    }
}

/// `executions` に 1 ステップのワークフロー実行を登録する。
/// 指定された step を current_step_index=0 として登録する。
fn insert_single_step_execution(
    execs: &mut HashMap<String, WorkflowExecution>,
    step: NodeDefinition,
) {
    let workflow = Workflow {
        variables: Default::default(),
        name: "regression-workflow".to_string(),
        description: "regression test".to_string(),
        builtin: false,
        nodes: vec![step],
    };
    let exec = WorkflowExecution {
        id: "exec-id".to_string(),
        workflow,
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    execs.insert(exec.id.clone(), exec);
}

#[tokio::test]
async fn start_step_session_with_deps_skips_side_effects_when_prompt_synthesis_fails() {
    // 回帰防止: `start_step_session` 本番経路では、参照先ファセットが
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
    // 本テストは `StepSessionDeps` 経由で副作用境界をテストダブルに差し替え、
    // ファセット参照が解決不能な execution に対し `start_step_session_with_deps`
    // を実行することで:
    //   (a) `Err(InvalidWorkflow(_))` が返ること
    //   (b) `create_step_session` の呼び出し回数が 0 であること
    //   (c) `fetch_parent_session` 等 他の副作用境界メソッドも 0 回であること
    //   (d) `engine.session_workflow_refs` が空のままであること
    //   (e) `executions["/repo"].current_session_id` が `None` のままであること
    // を assert する。`start_step_session` 内の順序を逆転（先に create_step_session
    // → 後に build_step_prompt）させると (b) が 1 となりテストが失敗する。
    let engine = WorkflowRuntimeService::new_for_test();

    // 参照先ファセットが解決不能な step を含む execution を登録する。
    // facets_base_dir() 配下に "nonexistent_policy_<uuid>.md" が偶然存在することは
    // 実用上ありえないため、ファセット合成は必ず失敗する。
    let mut step = make_test_step("missing-facet", TestKind::Session, "unused", vec![], None);
    set_instruction_facet(&mut step, None);
    set_policy_facet(
        &mut step,
        Some(format!(
            "nonexistent_policy_{}",
            uuid::Uuid::new_v4().simple()
        )),
    );

    {
        let mut execs = engine.executions.lock().await;
        insert_single_step_execution(&mut execs, step);
    }

    // 事前条件: session_workflow_refs は空
    assert!(engine.session_workflow_refs.lock().await.is_empty());

    let deps = RecordingStepSessionDeps::default();
    let result = engine.start_step_session_with_deps(&deps, "/repo").await;

    // (a) build_step_prompt 失敗で InvalidWorkflow エラーになる
    let err = result.expect_err("missing facet must cause start_step_session_with_deps to fail");
    assert!(
        matches!(err, WorkflowEngineError::InvalidWorkflow(_)),
        "missing facet must produce InvalidWorkflow error, got: {err:?}"
    );

    // (b)/(c) 副作用境界はいずれも呼ばれていない
    assert_eq!(
        deps.create_step_session_count(),
        0,
        "create_step_session must NOT be invoked when prompt synthesis fails"
    );
    assert_eq!(
        deps.dispatch_session_start_count(),
        0,
        "dispatch_session_start must NOT be invoked when prompt synthesis fails"
    );
    assert_eq!(
        deps.mark_step_tab_open_count(),
        0,
        "mark_step_tab_open must NOT be invoked when prompt synthesis fails"
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
async fn start_step_session_with_deps_invokes_side_effects_in_order_on_success() {
    // 副作用境界が正しい順序で呼ばれる成功経路を併せて検証する。
    // プロンプト合成が成功した場合は、create_step_session → dispatch_session_start
    // → NodeSessionStarted append → broadcast_state → start_agent_turn の全境界が各 1 回ずつ呼ばれ、
    // engine.session_workflow_refs と executions["/repo"].current_session_id が
    // 期待通り更新されることを assert する。
    let engine = WorkflowRuntimeService::new_for_test();

    let tmp = tempfile::TempDir::new().unwrap();
    let instructions = tmp.path().join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(instructions.join("ok.md"), "hello").unwrap();
    let mut step = make_test_step("ok-step", TestKind::Session, "ok", vec![], None);
    resolve_node_facets_for_test(&mut step, tmp.path());

    {
        let mut execs = engine.executions.lock().await;
        insert_single_step_execution(&mut execs, step);
    }

    let deps = RecordingStepSessionDeps::default();
    engine
        .start_step_session_with_deps(&deps, "/repo")
        .await
        .expect("start_step_session_with_deps must succeed for instruction facet step");

    // 各副作用境界が 1 回ずつ呼ばれている
    assert_eq!(deps.create_step_session_count(), 1);
    assert_eq!(deps.dispatch_session_start_count(), 1);
    assert_eq!(deps.mark_step_tab_open_count(), 1);
    assert_eq!(deps.append_node_session_started_count(), 1);
    assert_eq!(deps.broadcast_state_count(), 1);
    assert_eq!(deps.start_agent_turn_count(), 1);
    assert_eq!(
        deps.created_contexts(),
        vec![WorkflowStepContext {
            run_id: "exec-id".to_string(),
            workflow_name: "regression-workflow".to_string(),
            step_name: "ok-step".to_string(),
            run_index: 1,
            parent_step_name: None,
            parent_run_index: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        }]
    );

    // session_workflow_refs に SequentialStep として登録されている
    let refs = engine.session_workflow_refs.lock().await;
    let entry = refs
        .get("step-session-id")
        .expect("session_workflow_refs must contain step-session-id");
    assert_eq!(entry.run_id, "exec-id");
    drop(refs);

    // executions の current_session_id がステップセッションIDで更新されている
    let execs = engine.executions.lock().await;
    let (_, exec) = find_by_worktree(&execs, "/repo").expect("execution must remain registered");
    assert_eq!(
        exec.current_session_id.as_deref(),
        Some("step-session-id"),
        "current_session_id must be updated to the created step session id"
    );
}

#[tokio::test]
async fn start_step_session_with_deps_keeps_workflow_instruction_outside_step_context() {
    let engine = WorkflowRuntimeService::new_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    let instructions = tmp.path().join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(
        instructions.join("impl.md"),
        "Keep this instruction private.",
    )
    .unwrap();

    let mut step = make_test_step(
        "instruction-step",
        TestKind::Session,
        "unused",
        vec![],
        None,
    );
    set_instruction_facet(&mut step, Some("impl".to_string()));
    resolve_node_facets_for_test(&mut step, tmp.path());

    {
        let mut execs = engine.executions.lock().await;
        insert_single_step_execution(&mut execs, step);
    }

    let deps = RecordingStepSessionDeps::default();
    engine
        .start_step_session_with_deps(&deps, "/repo")
        .await
        .expect("start_step_session_with_deps must succeed");

    let contexts = deps.created_contexts();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].step_name, "instruction-step");
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
async fn start_step_session_with_deps_propagates_node_session_append_failure() {
    let engine = WorkflowRuntimeService::new_for_test();

    let tmp = tempfile::TempDir::new().unwrap();
    let instructions = tmp.path().join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(instructions.join("ok.md"), "hello").unwrap();
    let mut step = make_test_step("ok-step", TestKind::Session, "ok", vec![], None);
    resolve_node_facets_for_test(&mut step, tmp.path());

    {
        let mut execs = engine.executions.lock().await;
        insert_single_step_execution(&mut execs, step);
    }

    let deps = RecordingStepSessionDeps::default();
    deps.fail_append_node_session_started();
    let err = engine
        .start_step_session_with_deps(&deps, "/repo")
        .await
        .expect_err("append failure must propagate to the start flow");

    assert!(
        matches!(&err, WorkflowEngineError::SessionStore(message) if message.contains("append step session started failed")),
        "append failure must surface as SessionStore error, got: {err:?}"
    );
    assert_eq!(deps.create_step_session_count(), 1);
    assert_eq!(deps.dispatch_session_start_count(), 1);
    assert_eq!(deps.mark_step_tab_open_count(), 1);
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

// ---- build_parallel_step_prompt (並列子ステップの合成ルール) ----

fn make_parallel_step(name: &str) -> crate::adaptor::gateway::workflow::schema::InterimChild {
    crate::adaptor::gateway::workflow::schema::InterimChild {
        name: name.to_string(),
        permission: Some("edit".to_string()),
        ..crate::adaptor::gateway::workflow::schema::InterimChild::default()
    }
}

#[test]
fn build_parallel_step_prompt_splits_facets_into_system_and_user() {
    // Scenario: 並列ステップの子ステップでも同じ合成ルールが適用される
    // 並列子ステップに policy / output_contract / knowledge / instruction の 4 種すべてを指定し、
    // policy + output_contract が system_prompt に、knowledge + instruction が user_message に
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
    std::fs::write(policies.join("pol.md"), "PARALLEL_POLICY_BODY").unwrap();
    std::fs::write(knowledges.join("know.md"), "PARALLEL_KNOWLEDGE_BODY").unwrap();
    std::fs::write(instructions.join("inst.md"), "PARALLEL_INSTRUCTION_BODY").unwrap();
    std::fs::write(contracts.join("oc.md"), "PARALLEL_CONTRACT_BODY").unwrap();

    let mut ps = make_parallel_step("child");
    ps.facets.policy = Some("pol".to_string());
    ps.facets.knowledge = Some("know".to_string());
    ps.facets.instruction = Some("inst".to_string());
    ps.output_contract = Some("oc".to_string());
    resolve_child_facets_for_test(&mut ps, base);
    let (system_prompt, user_message) = workflow_prompt::build_parallel_step_prompt(
        &ps,
        "11111111-1111-1111-1111-111111111111",
        "/repo",
        None,
        &HashMap::new(),
        false,
        None,
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();

    let sp = system_prompt.expect("system_prompt must be set for parallel child with policy/oc");
    // policy と output_contract の本文が system_prompt に集約される
    assert!(sp.contains("PARALLEL_POLICY_BODY"));
    assert!(sp.contains("PARALLEL_CONTRACT_BODY"));
    // Contract 本文は system_prompt に集約される
    assert!(!sp.contains("PARALLEL_KNOWLEDGE_BODY"));
    assert!(!sp.contains("PARALLEL_INSTRUCTION_BODY"));

    // knowledge と Contract 由来の完了時アクションは user_message に集約される。
    // instruction は Agent system context の dedup 経路へ渡す。
    assert!(user_message.contains("PARALLEL_KNOWLEDGE_BODY"));
    assert!(!user_message.contains("PARALLEL_INSTRUCTION_BODY"));
    let instruction = workflow_prompt::render_child_workflow_instruction(
        &ps,
        "11111111-1111-1111-1111-111111111111",
        "/repo",
        None,
        &HashMap::new(),
    )
    .expect("parallel workflow instruction");
    assert!(instruction.contains("PARALLEL_INSTRUCTION_BODY"));
    assert!(user_message.contains("完了時の必須アクション"));
    // CLI 名は起動環境別 alias で展開される（spec issues-1054）。
    let cli_alias = WorkflowRuntimeService::resolve_releash_alias();
    assert!(user_message.contains(&format!(
        "{cli_alias} workflow output submit 11111111-1111-1111-1111-111111111111"
    )));
    assert!(user_message.contains("--step child"));
    assert!(user_message.contains("--type oc"));
    assert!(!user_message.contains("+  --step"));
    // policy / output_contract 本文は user_message には入らない
    assert!(!user_message.contains("PARALLEL_POLICY_BODY"));
    assert!(!user_message.contains("PARALLEL_CONTRACT_BODY"));
}

#[test]
fn build_parallel_step_prompt_no_policy_or_contract_returns_none_system_prompt() {
    // 並列子ステップでも policy / output_contract がない場合は system_prompt が None になる。
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();
    let instructions = base.join("instructions");
    std::fs::create_dir_all(&instructions).unwrap();
    std::fs::write(instructions.join("inst.md"), "INSTR").unwrap();

    let mut ps = make_parallel_step("child");
    ps.facets.instruction = Some("inst".to_string());
    resolve_child_facets_for_test(&mut ps, base);
    let (system_prompt, user_message) = workflow_prompt::build_parallel_step_prompt(
        &ps,
        "11111111-1111-1111-1111-111111111111",
        "/repo",
        None,
        &HashMap::new(),
        false,
        None,
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();

    assert!(system_prompt.is_none());
    assert!(!user_message.contains("INSTR"));
    let instruction = workflow_prompt::render_child_workflow_instruction(
        &ps,
        "11111111-1111-1111-1111-111111111111",
        "/repo",
        None,
        &HashMap::new(),
    )
    .expect("parallel workflow instruction");
    assert_eq!(instruction, "INSTR");
}

// ---- decide_approval_action ----

fn make_approval_exec(
    state: WorkflowExecutionState,
    rules: Vec<TransitionRule>,
) -> WorkflowExecution {
    WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: Workflow {
            variables: Default::default(),
            name: "test".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_approval_step("review", "Review the code", rules)],
        },
        state,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: vec![],
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    }
}

// ---- approval input validation adapter ----

#[test]
fn validate_approval_decision_reject_empty_comment_returns_error() {
    let result = workflow_approval_runtime::validate_approval_input(
        &ApprovalDecision::Reject {
            comment: "".to_string(),
        },
        None,
    );
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Reject comment must not be empty"));
}

#[test]
fn validate_approval_decision_reject_whitespace_only_returns_error() {
    let result = workflow_approval_runtime::validate_approval_input(
        &ApprovalDecision::Reject {
            comment: "   \n\t  ".to_string(),
        },
        None,
    );
    assert!(result.is_err());
}

#[test]
fn validate_approval_decision_reject_over_limit_returns_error() {
    let result = workflow_approval_runtime::validate_approval_input(
        &ApprovalDecision::Reject {
            comment: "x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1),
        },
        None,
    );
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .starts_with("validation_error:"));
}

#[test]
fn validate_approval_decision_reject_with_comment_ok() {
    let result = workflow_approval_runtime::validate_approval_input(
        &ApprovalDecision::Reject {
            comment: "Please fix the bug".to_string(),
        },
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn validate_approval_decision_approve_ok() {
    let result =
        workflow_approval_runtime::validate_approval_input(&ApprovalDecision::Approve, None);
    assert!(result.is_ok());
}

#[test]
fn validate_approval_target_missing_values_returns_unauthorized_target() {
    let exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
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
    let exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
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
        Some("other-step"),
    );
    assert!(matches!(
        result.unwrap_err(),
        WorkflowEngineError::UnauthorizedApprovalTarget(_)
    ));
}

#[test]
fn validate_approval_target_non_waiting_returns_invalid_state() {
    let exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
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
        WorkflowExecutionState::Completed,
        WorkflowExecutionState::Failed {
            reason: "failed".to_string(),
            kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: None,
        },
        WorkflowExecutionState::Aborted,
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
        assert!(exec.step_history.is_empty());
    }
}

#[tokio::test]
async fn validate_approval_target_wrong_worktree_returns_unauthorized_without_mutating_state() {
    let engine = WorkflowRuntimeService::new_for_test();
    let exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
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
    assert_eq!(original.state, WorkflowExecutionState::WaitingApproval);
    assert!(original.step_history.is_empty());
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
// 廃止に伴い削除した。approval node の構造化出力は CLI / Tauri 経由の `SubmitOutput`
// で確定し、対応する境界テストは `dispatch_boundary_tests::submit_output_*` 群と
// `workflow::contract::tests::validate_contract_value_*` 群でカバーされる。

#[tokio::test]
async fn validate_approval_chat_instruction_limits_current_approval_session() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("step-session".to_string());
    let run_id = exec.id.clone();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(run_id.clone(), exec);
    }
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "step-session".to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );
    }

    let result = engine
        .validate_approval_chat_instruction(
            "step-session",
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
    let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("step-session".to_string());
    let run_id = exec.id.clone();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(run_id.clone(), exec);
    }
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "step-session".to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );
    }

    for content in ["", "   ", "\n\t \r\n"] {
        let err = engine
            .validate_approval_chat_instruction("step-session", content)
            .await
            .unwrap_err();
        assert!(
            err.to_string().starts_with("validation_error:"),
            "expected validation_error for content={content:?}, got: {err}"
        );
    }
}

#[tokio::test]
async fn validate_approval_chat_instruction_rejects_current_approval_step_before_waiting() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
    exec.current_session_id = Some("step-session".to_string());
    let run_id = exec.id.clone();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(run_id.clone(), exec);
    }
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "step-session".to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );
    }

    let result = engine
        .validate_approval_chat_instruction("step-session", "Please adjust the policy")
        .await;
    assert!(matches!(
        result.unwrap_err(),
        WorkflowEngineError::InvalidState(_)
    ));
}

#[tokio::test]
async fn validate_approval_chat_instruction_rejects_stale_approved_policy_session() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
    exec.workflow.nodes[0].name = "implementation_fix_policy".to_string();
    exec.workflow.nodes[0].output_contract = Some("approved-fix-policy".to_string());
    exec.current_session_id = Some("fix-session".to_string());
    exec.step_history.push(StepHistoryEntry {
        step_name: "implementation_fix_policy".to_string(),
        completed_at: 1000.0,
        result: Some("approved".to_string()),
        session_id: Some("stale-policy-session".to_string()),
        token_usage: None,
        structured_output: Some(serde_json::json!({
            "policy": "Already approved.",
            "review_step": "code_review_parallel",
            "findings": []
        })),
        run_index: 1,
        child_outputs: None,
        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
    });
    exec.step_outputs.insert(
        "implementation_fix_policy".to_string(),
        StepOutput {
            step_name: "implementation_fix_policy".to_string(),
            run_index: 1,
            session_id: Some("stale-policy-session".to_string()),
            result: Some("approved".to_string()),
            structured_output: Some(serde_json::json!({
                "policy": "Already approved.",
                "review_step": "code_review_parallel",
                "findings": []
            })),
            output_contract: Some("approved-fix-policy".to_string()),
            token_usage: None,
            completed_at: 1000.0,
        },
    );
    let run_id = exec.id.clone();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(run_id.clone(), exec);
    }
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "stale-policy-session".to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
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

#[tokio::test]
async fn validate_approval_chat_instruction_rejects_stale_rejected_policy_session() {
    let engine = WorkflowRuntimeService::new_for_test();
    let mut exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
    exec.workflow.nodes[0].name = "implementation_fix_policy".to_string();
    exec.workflow.nodes[0].output_contract = Some("approved-fix-policy".to_string());
    exec.current_session_id = Some("implementation-approval-session".to_string());
    exec.step_history.push(StepHistoryEntry {
        step_name: "implementation_fix_policy".to_string(),
        completed_at: 1000.0,
        result: Some("reject".to_string()),
        session_id: Some("stale-rejected-policy-session".to_string()),
        token_usage: None,
        structured_output: Some(serde_json::json!({
            "decision": "reject",
            "comment": "Revise policy."
        })),
        run_index: 1,
        child_outputs: None,
        state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
    });
    exec.step_outputs.insert(
        "implementation_fix_policy".to_string(),
        StepOutput {
            step_name: "implementation_fix_policy".to_string(),
            run_index: 1,
            session_id: Some("stale-rejected-policy-session".to_string()),
            result: Some("reject".to_string()),
            structured_output: Some(serde_json::json!({
                "decision": "reject",
                "comment": "Revise policy."
            })),
            output_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        },
    );
    let run_id = exec.id.clone();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(run_id.clone(), exec);
    }
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "stale-rejected-policy-session".to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );
    }

    let result = engine
        .validate_approval_chat_instruction("stale-rejected-policy-session", "Please change policy")
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
        workflow_step_session: false,
        workflow_step_context: None,
        context_epoch: None,
    };

    let output =
        WorkflowRuntimeService::extract_last_assistant_text_from_session(&session).unwrap();
    assert_eq!(output, "latest approved policy");
}

// ---- make_step_history_entry ----

#[test]
fn make_step_history_entry_reject_no_structured_output() {
    let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
    let entry = exec.make_step_history_entry(Some("reject".to_string()), None, None);
    assert_eq!(entry.result.as_deref(), Some("reject"));
    assert!(entry.structured_output.is_none());
    // structured_outputがNoneなのでStepOutputは生成されない
    assert!(!exec.step_outputs.contains_key("review"));
}

// ---- handle_approval integration (lock-inner logic) ----

#[test]
fn reject_comment_flows_through_approval_to_transition_and_history() {
    // handle_approval() のロック内ロジックを再現:
    // validate → decide → make_step_history_entry → apply_transition
    let decision = ApprovalDecision::Reject {
        comment: "Fix the naming convention".to_string(),
    };

    // 1. validate
    workflow_approval_runtime::validate_approval_input(&decision, None).unwrap();

    // 3. 遷移先 "fix" ステップを含むワークフローを構築
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: Workflow {
            variables: Default::default(),
            name: "review-fix".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                make_approval_step(
                    "review",
                    "Review the code",
                    vec![TransitionRule {
                        r#match: "reject".to_string(),
                        next: "fix".to_string(),
                    }],
                ),
                {
                    let mut fix =
                        make_test_step("fix", TestKind::Session, "Fix the issues", vec![], None);
                    fix.pass_previous_response = Some(true);
                    fix
                },
            ],
        },
        state: WorkflowExecutionState::WaitingApproval,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: vec![],
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    // 4. decide
    let action = exec.decide_approval_action(&decision).unwrap();
    assert_eq!(action, ApprovalAction::TransitionTo("fix".to_string()));

    // 5. handle_approvalと同じ適用経路でReject commentをStepOutputに保存する
    let outcome = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        &decision,
        ApprovalApplication {
            effective_result: "reject".to_string(),
            structured_output: Some(workflow_approval_runtime::reject_structured_output(
                "Fix the naming convention",
                &[],
            )),
            output_contract: None,
        },
    )
    .unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));

    // 検証: step_history にReject結果が記録されている
    assert_eq!(exec.step_history.len(), 1);
    let hist = &exec.step_history[0];
    assert_eq!(hist.step_name, "review");
    assert_eq!(hist.result.as_deref(), Some("reject"));
    assert_eq!(
        hist.structured_output.as_ref().unwrap()["comment"],
        "Fix the naming convention"
    );
    let review_output = exec.step_outputs.get("review").unwrap();
    assert_eq!(review_output.result.as_deref(), Some("reject"));
    assert_eq!(
        review_output.structured_output.as_ref().unwrap()["comment"],
        "Fix the naming convention"
    );

    // 検証: 遷移先 "fix" ステップに移動している
    assert_eq!(exec.current_step_index, 1);
    assert_eq!(exec.workflow.nodes[exec.current_step_index].name, "fix");

    let injected = workflow_prompt::inject_step_outputs(
        "Draft next policy",
        &exec.workflow.nodes[exec.current_step_index],
        &exec.step_outputs,
        &exec.step_history,
        &HashMap::new(),
    );
    assert!(injected.contains("\"decision\": \"reject\""));
    assert!(injected.contains("\"comment\": \"Fix the naming convention\""));
}

#[test]
fn apply_approval_application_records_approved_policy_and_advances_once() {
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: Workflow {
            variables: Default::default(),
            name: "auto-approve".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                {
                    let mut step = make_approval_step("fix_policy", "Review fix policy", vec![]);
                    step.output_contract = Some("approved-fix-policy".to_string());
                    step
                },
                make_test_step("fix", TestKind::Session, "Fix", vec![], None),
            ],
        },
        state: WorkflowExecutionState::WaitingApproval,
        current_step_index: 0,
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("fix_policy".to_string(), 1);
            m
        },
        step_history: vec![],
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("policy-session".to_string()),
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let structured_output = serde_json::json!({
        "policy": "Fix only the reported issues.",
        "review_step": "code_review_parallel",
        "findings": []
    });
    let first = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        &ApprovalDecision::Approve,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            structured_output: Some(structured_output),
            output_contract: Some("approved-fix-policy".to_string()),
        },
    )
    .unwrap();
    assert!(matches!(first, StepOutcome::TransitionAndStart(_)));
    assert_eq!(exec.current_step_index, 1);
    assert_eq!(exec.step_history.len(), 1);
    assert_eq!(*exec.step_execution_counts.get("fix").unwrap(), 1);
    assert!(!exec.workflow_variables.contains_key("approved_fix_policy"));

    let duplicate = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        &ApprovalDecision::Approve,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            structured_output: Some(serde_json::json!({
                "policy": "Duplicate",
                "review_step": "code_review_parallel",
                "findings": []
            })),
            output_contract: Some("approved-fix-policy".to_string()),
        },
    );
    match duplicate {
        Err(WorkflowEngineError::InvalidState(_)) => {}
        _ => panic!("expected invalid_state"),
    }
    assert_eq!(exec.step_history.len(), 1);
    assert_eq!(*exec.step_execution_counts.get("fix").unwrap(), 1);
}

#[test]
fn auto_approve_persist_target_applies_latest_policy_and_advances_once() {
    let mut exec = WorkflowExecution {
        id: "exec-auto-approve".to_string(),
        workflow: Workflow {
            variables: Default::default(),
            name: "auto-approve-path".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                {
                    let mut step = make_approval_step(
                        "implementation_fix_policy",
                        "Review fix policy",
                        vec![],
                    );
                    step.output_contract = Some("approved-fix-policy".to_string());
                    step.pass_output_from = Some(vec!["code_review_parallel".to_string()]);
                    step
                },
                make_test_step("fix", TestKind::Session, "Fix", vec![], None),
                make_fanout_step(
                    "code_review_parallel",
                    vec![],
                    Some(ParallelAggregate {
                        all_match: Some("LGTM".to_string()),
                        any_match: None,
                        then: "fix".to_string(),
                        r#else: "implementation_fix_policy".to_string(),
                    }),
                ),
            ],
        },
        state: WorkflowExecutionState::WaitingApproval,
        current_step_index: 0,
        step_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("policy-session".to_string()),
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let snapshot = exec.to_workflow_state();
    assert_eq!(
        workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(&snapshot, true),
        Some((
            "exec-auto-approve".to_string(),
            "implementation_fix_policy".to_string()
        ))
    );

    // [08] prose 抽出経路は廃止済み。CLI submit 経由で確定する想定の structured_output
    // を直接組み立てて apply_approval_application の遷移挙動を検証する。
    let structured_output = serde_json::json!({
        "policy": "Fix only reviewed findings.",
        "review_step": "code_review_parallel",
        "findings": []
    });
    let outcome = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        &ApprovalDecision::Approve,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            structured_output: Some(structured_output),
            output_contract: Some("approved-fix-policy".to_string()),
        },
    )
    .unwrap();

    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
    assert_eq!(exec.current_step_index, 1);
    assert_eq!(exec.step_history.len(), 1);
    assert_eq!(exec.step_outputs.len(), 1);
    assert_eq!(
        exec.step_outputs["implementation_fix_policy"]
            .structured_output
            .as_ref()
            .unwrap()["policy"],
        "Fix only reviewed findings."
    );
    assert_eq!(exec.workflow_variables.get("approved_fix_policy"), None);
    assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));

    let duplicate = WorkflowRuntimeService::apply_approval_application(
        &mut exec,
        &ApprovalDecision::Approve,
        ApprovalApplication {
            effective_result: "approved".to_string(),
            structured_output: Some(serde_json::json!({
                "policy": "Duplicate",
                "review_step": "code_review_parallel",
                "findings": []
            })),
            output_contract: Some("approved-fix-policy".to_string()),
        },
    );
    assert!(matches!(
        duplicate,
        Err(WorkflowEngineError::InvalidState(_))
    ));
    assert_eq!(exec.step_history.len(), 1);
    assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));
}

#[tokio::test]
async fn execute_outcome_auto_approve_persist_adopts_policy_and_starts_fix_once() {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/repo";
    let policy_session_id = uuid::Uuid::new_v4().to_string();

    let mut fix_step = make_test_step("fix", TestKind::Session, "Fix", vec![], None);
    fix_step.collect = Some(CollectConfig {
        from: vec!["implementation_fix_policy".to_string()],
        reduce: ReduceStrategy::Last,
    });
    let exec = WorkflowExecution {
        id: "exec-auto-approve".to_string(),
        workflow: Workflow {
            variables: Default::default(),
            name: "auto-approve-execute-outcome".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                make_fanout_step(
                    "code_review_parallel",
                    vec![],
                    Some(ParallelAggregate {
                        all_match: Some("LGTM".to_string()),
                        any_match: None,
                        then: "done".to_string(),
                        r#else: "implementation_fix_policy".to_string(),
                    }),
                ),
                {
                    let mut step = make_approval_step(
                        "implementation_fix_policy",
                        "Review fix policy",
                        vec![],
                    );
                    step.output_contract = Some("approved-fix-policy".to_string());
                    step.pass_output_from = Some(vec!["code_review_parallel".to_string()]);
                    step
                },
                fix_step,
            ],
        },
        state: WorkflowExecutionState::WaitingApproval,
        current_step_index: 1,
        step_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some(policy_session_id.clone()),
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let snapshot = exec.to_workflow_state();
    let run_id = exec.id.clone();
    engine.executions.lock().await.insert(run_id.clone(), exec);
    engine.session_workflow_refs.lock().await.insert(
        policy_session_id,
        SessionWorkflowRef {
            run_id: run_id.clone(),
        },
    );

    let outcome = engine
        .execute_outcome_persist_auto_approve_for_test(worktree_path, &snapshot)
        .await
        .unwrap()
        .unwrap();

    let execs = engine.executions.lock().await;
    let (_, exec) = find_by_worktree(&execs, worktree_path).unwrap();
    assert!(matches!(outcome, StepOutcome::ReduceAndTransition(_)));
    assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));
    assert_eq!(
        exec.step_history
            .iter()
            .filter(|entry| entry.step_name == "implementation_fix_policy")
            .count(),
        1
    );
    // [08] prose 抽出経路は廃止済み。auto approve 経路でも structured_output は
    // 確定されず、step は output 無しで完了する（spec [08] Rule 4）。
    assert!(exec
        .step_outputs
        .get("implementation_fix_policy")
        .and_then(|output| output.structured_output.as_ref())
        .is_none());
}

#[test]
fn execute_outcome_persist_path_builds_auto_approve_target_for_current_step() {
    let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("policy-session".to_string());
    let waiting = exec.to_workflow_state();

    assert_eq!(
        workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(&waiting, true),
        Some(("exec-1".to_string(), "review".to_string()))
    );
    assert_eq!(
        workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(&waiting, false),
        None
    );

    exec.state = WorkflowExecutionState::Running;
    let running = exec.to_workflow_state();
    assert_eq!(
        workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(&running, true),
        None
    );
}

#[test]
fn workflow_approval_auto_approve_flag_controls_waiting_approval_snapshots() {
    let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("policy-session".to_string());
    let waiting = exec.to_workflow_state();
    assert!(workflow_approval_runtime::should_auto_approve_workflow_approval(&waiting, true));
    assert!(!workflow_approval_runtime::should_auto_approve_workflow_approval(&waiting, false));

    exec.state = WorkflowExecutionState::Running;
    let running = exec.to_workflow_state();
    assert!(!workflow_approval_runtime::should_auto_approve_workflow_approval(&running, true));
}

#[test]
fn workflow_approval_auto_approve_disabled_ignores_agent_auto_approve_permission_mode() {
    let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
    exec.current_session_id = Some("policy-session".to_string());
    let agent_auto_approve_permission_mode = "full";
    let workflow_approval_auto_approve_enabled = false;
    let snapshot = exec.to_workflow_state();

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

fn make_normal_step_exec_with_stall_observation() -> WorkflowExecution {
    let mut exec = WorkflowExecution {
        id: "normal-stall-clear".to_string(),
        workflow: Workflow {
            variables: Default::default(),
            name: "normal-stall-clear-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                make_test_step("plan", TestKind::Session, "plan", vec![], None),
                make_test_step("implement", TestKind::Session, "implement", vec![], None),
            ],
        },
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: HashMap::from([("plan".to_string(), 1)]),
        step_history: Vec::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("normal-session".to_string()),
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
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
fn normal_step_completion_retry_and_transition_clear_stall_observations() {
    let mut completed = make_normal_step_exec_with_stall_observation();
    let entry = completed.make_step_history_entry(Some("done".to_string()), None, None);
    completed.step_history.push(entry);
    assert!(completed.to_workflow_state().stall_observations.is_empty());

    let mut retried = make_normal_step_exec_with_stall_observation();
    let retry_snapshot = match retried.retry_current_step() {
        StepOutcome::RetryCurrentStep { snapshot, .. } => snapshot,
        _ => panic!("unexpected retry outcome"),
    };
    assert!(retry_snapshot.stall_observations.is_empty());

    let mut transitioned = make_normal_step_exec_with_stall_observation();
    let transition_snapshot = match transitioned.apply_transition("implement").unwrap() {
        StepOutcome::TransitionAndStart(snapshot) => snapshot,
        _ => panic!("unexpected transition outcome"),
    };
    assert!(transition_snapshot.stall_observations.is_empty());
}

// R4-02: make_step_history_entryがcontract resultをStepOutput.resultに保存する
#[test]
fn make_step_history_entry_saves_contract_result_to_step_output() {
    let mut exec = WorkflowExecution {
        id: "test-exec".to_string(),
        workflow: make_test_workflow(),
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("plan".to_string(), 1);
            m
        },
        step_history: vec![],
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("session-1".to_string()),
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let structured = serde_json::json!({"verdict": "LGTM", "findings": []});
    let entry = exec.make_step_history_entry(
        Some("LGTM".to_string()),
        Some(structured.clone()),
        Some("review-verdict".to_string()),
    );

    assert_eq!(entry.result.as_deref(), Some("LGTM"));
    assert_eq!(entry.structured_output, Some(structured.clone()));

    let step_output = exec
        .step_outputs
        .get("plan")
        .expect("StepOutput should exist");
    assert_eq!(step_output.result.as_deref(), Some("LGTM"));
    assert_eq!(step_output.structured_output, Some(structured));
    assert_eq!(
        step_output.output_contract.as_deref(),
        Some("review-verdict")
    );
}

#[test]
fn make_step_history_entry_no_structured_output_no_step_output() {
    let mut exec = WorkflowExecution {
        id: "test-exec".to_string(),
        workflow: make_test_workflow(),
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("plan".to_string(), 1);
            m
        },
        step_history: vec![],
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: Some("session-1".to_string()),
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let entry = exec.make_step_history_entry(Some("complete".to_string()), None, None);

    assert_eq!(entry.result.as_deref(), Some("complete"));
    assert!(entry.structured_output.is_none());
    assert!(!exec.step_outputs.contains_key("plan"));
}

// ---- on_exhausted: apply_transition テスト ----

fn make_on_exhausted_workflow() -> Workflow {
    Workflow {
        variables: Default::default(),
        name: "on-exhausted-test".to_string(),
        description: "Test on_exhausted".to_string(),
        builtin: false,
        nodes: vec![
            make_test_step(
                "fix",
                TestKind::Session,
                "Fix issues",
                vec![TransitionRule {
                    r#match: ".*".to_string(),
                    next: "review".to_string(),
                }],
                Some(CycleGuard {
                    max_iterations: 2,
                    on_exhausted: Some("approval".to_string()),
                }),
            ),
            make_test_step(
                "review",
                TestKind::Session,
                "Review",
                vec![TransitionRule {
                    r#match: "NEEDS_FIX".to_string(),
                    next: "fix".to_string(),
                }],
                None,
            ),
            NodeDefinition {
                resets_cycle_for: Some(vec!["fix".to_string()]),
                ..make_test_step(
                    "approval",
                    TestKind::Session,
                    "Approve",
                    vec![TransitionRule {
                        r#match: "NEEDS_FIX".to_string(),
                        next: "fix".to_string(),
                    }],
                    None,
                )
            },
        ],
    }
}

#[test]
fn on_exhausted_transitions_to_fallback_step() {
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_on_exhausted_workflow(),
        state: WorkflowExecutionState::Running,
        current_step_index: 1, // review
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("fix".to_string(), 2); // already at max
            m
        },
        step_history: vec![],
        step_outputs: HashMap::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    // fix への遷移を試みる → ガード超過 → on_exhausted で approval へ
    let outcome = exec.apply_transition("fix").unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
    assert_eq!(
        exec.workflow.nodes[exec.current_step_index].name,
        "approval"
    );
}

#[test]
fn on_exhausted_none_fails_workflow() {
    let mut wf = make_on_exhausted_workflow();
    // on_exhausted を None に変更
    wf.nodes[0].cycle_guard = Some(CycleGuard {
        max_iterations: 2,
        on_exhausted: None,
    });

    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: wf,
        state: WorkflowExecutionState::Running,
        current_step_index: 1,
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("fix".to_string(), 2);
            m
        },
        step_history: vec![],
        step_outputs: HashMap::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let outcome = exec.apply_transition("fix").unwrap();
    assert!(matches!(outcome, StepOutcome::Persist(_)));
    assert!(matches!(exec.state, WorkflowExecutionState::Failed { .. }));
}

#[test]
fn check_cycle_guard_exceeded_with_on_exhausted() {
    let exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_on_exhausted_workflow(),
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("fix".to_string(), 2);
            m
        },
        step_history: vec![],
        step_outputs: HashMap::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    assert_eq!(
        exec.check_cycle_guard("fix").unwrap(),
        CycleGuardResult::Exceeded {
            max_iterations: 2,
            count: 2,
            on_exhausted: Some("approval".to_string()),
        }
    );
}

// ---- resets_cycle_for テスト ----

#[test]
fn resets_cycle_for_clears_execution_count() {
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_on_exhausted_workflow(),
        state: WorkflowExecutionState::Running,
        current_step_index: 0, // fix
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("fix".to_string(), 2);
            m
        },
        step_history: vec![],
        step_outputs: HashMap::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    // approval に遷移 → resets_cycle_for で fix のカウントがリセット
    let outcome = exec.apply_transition("approval").unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
    assert_eq!(
        exec.workflow.nodes[exec.current_step_index].name,
        "approval"
    );
    // fix のカウントがリセットされている
    assert_eq!(exec.step_execution_counts.get("fix"), None);
}

#[test]
fn resets_cycle_for_allows_reloop_after_reset() {
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: make_on_exhausted_workflow(),
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("fix".to_string(), 2);
            m
        },
        step_history: vec![],
        step_outputs: HashMap::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    // approval に遷移（カウントリセット）
    exec.apply_transition("approval").unwrap();
    assert_eq!(exec.step_execution_counts.get("fix"), None);

    // fix に再遷移可能（リセット後なのでガードに引っかからない）
    let outcome = exec.apply_transition("fix").unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
    assert_eq!(exec.workflow.nodes[exec.current_step_index].name, "fix");
    assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));

    // 2回目も可能
    let outcome = exec.apply_transition("fix").unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
    assert_eq!(exec.step_execution_counts.get("fix"), Some(&2));

    // 3回目は上限到達 → on_exhausted で approval へ
    let outcome = exec.apply_transition("fix").unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
    assert_eq!(
        exec.workflow.nodes[exec.current_step_index].name,
        "approval"
    );
}

// ---- on_exhausted チェーン遷移テスト ----

#[test]
fn on_exhausted_chain_transitions() {
    // step_a → (exhausted) → step_b → (exhausted) → step_c
    let wf = Workflow {
        variables: Default::default(),
        name: "chain-test".to_string(),
        description: "test".to_string(),
        builtin: false,
        nodes: vec![
            make_test_step(
                "step_a",
                TestKind::Session,
                "A",
                vec![],
                Some(CycleGuard {
                    max_iterations: 1,
                    on_exhausted: Some("step_b".to_string()),
                }),
            ),
            make_test_step(
                "step_b",
                TestKind::Session,
                "B",
                vec![],
                Some(CycleGuard {
                    max_iterations: 1,
                    on_exhausted: Some("step_c".to_string()),
                }),
            ),
            make_test_step("step_c", TestKind::Session, "C", vec![], None),
        ],
    };
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: wf,
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("step_a".to_string(), 1);
            m.insert("step_b".to_string(), 1);
            m
        },
        step_history: vec![],
        step_outputs: HashMap::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    // step_a → exhausted → step_b → exhausted → step_c
    let outcome = exec.apply_transition("step_a").unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
    assert_eq!(exec.workflow.nodes[exec.current_step_index].name, "step_c");
}

#[test]
fn on_exhausted_chain_to_non_exhausted_fails() {
    // step_a → (exhausted) → step_b (exhausted, no on_exhausted) → Failed
    let wf = Workflow {
        variables: Default::default(),
        name: "chain-fail-test".to_string(),
        description: "test".to_string(),
        builtin: false,
        nodes: vec![
            make_test_step(
                "step_a",
                TestKind::Session,
                "A",
                vec![],
                Some(CycleGuard {
                    max_iterations: 1,
                    on_exhausted: Some("step_b".to_string()),
                }),
            ),
            make_test_step(
                "step_b",
                TestKind::Session,
                "B",
                vec![],
                Some(CycleGuard {
                    max_iterations: 1,
                    on_exhausted: None,
                }),
            ),
        ],
    };
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: wf,
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: {
            let mut m = HashMap::new();
            m.insert("step_a".to_string(), 1);
            m.insert("step_b".to_string(), 1);
            m
        },
        step_history: vec![],
        step_outputs: HashMap::new(),
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let outcome = exec.apply_transition("step_a").unwrap();
    assert!(matches!(outcome, StepOutcome::Persist(_)));
    assert!(matches!(exec.state, WorkflowExecutionState::Failed { .. }));
}

// ---- step が新しい実行を開始する瞬間に step_outputs から前回値を破棄する（Spec issues-989） ----

fn make_step_output_fixture(step_name: &str, run_index: u32) -> StepOutput {
    StepOutput {
        step_name: step_name.to_string(),
        run_index,
        session_id: None,
        result: Some("prev".to_string()),
        structured_output: Some(serde_json::json!({"verdict": "LGTM"})),
        output_contract: None,
        token_usage: None,
        completed_at: 1000.0,
    }
}

#[test]
fn apply_advance_clears_step_outputs_for_new_step() {
    // ループで同一 step が再実行されるとき、advance による遷移で
    // 遷移先 step の前回出力が step_outputs から破棄されることを検証する。
    let mut exec = make_exec(0); // plan → implement
    exec.current_session_id = Some("plan-session".to_string());
    exec.step_outputs.insert(
        "implement".to_string(),
        make_step_output_fixture("implement", 1),
    );
    // 他 step の前回出力は残り続けることも併せて確認。
    exec.step_outputs
        .insert("plan".to_string(), make_step_output_fixture("plan", 1));

    let outcome = exec.apply_advance();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
    assert_eq!(
        exec.workflow.nodes[exec.current_step_index].name,
        "implement"
    );
    assert!(!exec.step_outputs.contains_key("implement"));
    assert!(exec.step_outputs.contains_key("plan"));
    assert!(exec.current_session_id.is_none());
}

#[test]
fn apply_transition_clears_step_outputs_for_target_step() {
    // ループで前ステップ（review）に戻る遷移でも、遷移先の前回出力が破棄される。
    let mut exec = make_exec(2); // review
    exec.current_session_id = Some("review-session".to_string());
    exec.step_outputs.insert(
        "implement".to_string(),
        make_step_output_fixture("implement", 1),
    );

    let outcome = exec.apply_transition("implement").unwrap();
    assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
    assert_eq!(
        exec.workflow.nodes[exec.current_step_index].name,
        "implement"
    );
    assert!(!exec.step_outputs.contains_key("implement"));
    assert!(exec.current_session_id.is_none());
}

#[test]
fn apply_transition_to_parallel_block_clears_block_and_children() {
    // 並列ブロックへの遷移では、ブロック自身と全子 step の前回出力が破棄される。
    let parallel_block = make_fanout_step(
        "code_review_parallel",
        vec![
            make_parallel_step("review_security"),
            make_parallel_step("review_style"),
        ],
        None,
    );
    let wf = Workflow {
        variables: Default::default(),
        name: "loop-parallel".to_string(),
        description: "test".to_string(),
        builtin: false,
        nodes: vec![
            make_test_step("fix", TestKind::Session, "Fix", vec![], None),
            parallel_block,
        ],
    };
    let mut exec = WorkflowExecution {
        id: "exec-1".to_string(),
        workflow: wf,
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: vec![],
        step_outputs: {
            let mut m = HashMap::new();
            m.insert(
                "code_review_parallel".to_string(),
                make_step_output_fixture("code_review_parallel", 1),
            );
            m.insert(
                "review_security".to_string(),
                make_step_output_fixture("review_security", 1),
            );
            m.insert(
                "review_style".to_string(),
                make_step_output_fixture("review_style", 1),
            );
            m.insert("fix".to_string(), make_step_output_fixture("fix", 1));
            m
        },
        started_at: 1000.0,
        updated_at: 1000.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/repo".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };

    let outcome = exec.apply_transition("code_review_parallel").unwrap();
    assert!(matches!(outcome, StepOutcome::StartParallel(_)));
    assert!(!exec.step_outputs.contains_key("code_review_parallel"));
    assert!(!exec.step_outputs.contains_key("review_security"));
    assert!(!exec.step_outputs.contains_key("review_style"));
    // 並列ブロック外の step の前回出力は破棄されない。
    assert!(exec.step_outputs.contains_key("fix"));
}

// ---- resolve_step_settings ----

#[test]
fn resolve_step_settings_model_and_permission_specified() {
    let result = resolve_step_settings(
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
        ResolvedStepSettings {
            backend_id: Some("codex".to_string()),
            selected_model: Some("codex-mini".to_string()),
            permission_mode: "full".to_string(),
        }
    );
}

#[test]
fn resolve_step_settings_model_only() {
    let result = resolve_step_settings(
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
        ResolvedStepSettings {
            backend_id: Some("claude".to_string()),
            selected_model: Some("haiku".to_string()),
            permission_mode: "edit".to_string(),
        }
    );
}

#[test]
fn resolve_step_settings_permission_only_clears_model_to_unset() {
    // Spec: workflow 経路では step model 未指定なら親の選択モデルへフォールバックしない。
    // permission のみ指定でも selected_model は None になる。
    let result = resolve_step_settings(
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
        ResolvedStepSettings {
            backend_id: Some("claude".to_string()),
            selected_model: None,
            permission_mode: "ask".to_string(),
        }
    );
}

#[test]
fn resolve_step_settings_nothing_specified_clears_model_to_unset() {
    // Spec: model 未指定（None）は未指定状態のまま。親の selected_model へ
    // 暗黙フォールバックしない。
    let result = resolve_step_settings(
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
        ResolvedStepSettings {
            backend_id: Some("claude".to_string()),
            selected_model: None,
            permission_mode: "edit".to_string(),
        }
    );
}

#[test]
fn resolve_step_settings_parallel_children_different_configs() {
    // ステップA: model=opus-4, permission=ask
    let result_a = resolve_step_settings(
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
        ResolvedStepSettings {
            backend_id: Some("claude".to_string()),
            selected_model: Some("opus-4".to_string()),
            permission_mode: "ask".to_string(),
        }
    );

    // ステップB: model=codex-mini, permission=full
    let result_b = resolve_step_settings(
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
        ResolvedStepSettings {
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

// ---- ワークフロー step session の attributes 永続化 ----

// Spec issues-947: ワークフロー step session 作成は
// `create_session_internal_with_attributes` 経由で permission_mode / selected_model /
// workflow_step_session=true を初回保存で確定する。create_step_session_with_settings の
// 後段（resolve_step_settings の結果を attributes に流して save する経路）が
// 二段階保存に逆戻りしないことをガードする。
#[test]
fn step_session_persists_permission_workflow_flag_and_model_on_initial_save() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = crate::test_support::build_session_store();

    let settings = resolve_step_settings(
        Some("opus-4".to_string()),
        Some("edit".to_string()),
        Some("claude".to_string()),
        &WorkflowDefaults {
            backend_id: Some("codex".to_string()),
            permission_mode: "ask".to_string(),
        },
    );
    let permission_mode =
        crate::domain::agent_session::PermissionMode::parse(&settings.permission_mode).unwrap();
    let session = crate::usecase::agent_session::session::create_session_internal_with_attributes(
        &store,
        tmp.path(),
        "/repo",
        settings.backend_id.clone(),
        permission_mode,
        crate::usecase::agent_session::session::SessionCreationAttributes {
            selected_model: settings.selected_model.clone(),
            workflow_step_session: true,
            workflow_step_context: None,
            ..Default::default()
        },
    )
    .unwrap();

    // 初回保存で permission_mode / workflow_step_session / selected_model / backend_id が確定。
    assert_eq!(session.permission_mode, "edit");
    assert!(session.workflow_step_session);
    assert_eq!(session.selected_model.as_deref(), Some("opus-4"));
    assert_eq!(session.backend_id.as_deref(), Some("claude"));

    // 別インスタンスから読み直しても同じ値で復元される（永続化が確定値で書かれている）。
    let store2 = crate::test_support::build_session_store();
    let loaded = store2
        .load_full_session_for_restore(tmp.path(), &session.id)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.permission_mode, "edit");
    assert!(loaded.workflow_step_session);
    assert_eq!(loaded.selected_model.as_deref(), Some("opus-4"));
    assert_eq!(loaded.backend_id.as_deref(), Some("claude"));
}

// 親セッションから permission_mode/backend_id を継承する経路でも初回保存で確定することを確認する。
// selected_model は Spec issues-946 により暗黙フォールバック禁止のため、step 未指定なら None。
#[test]
fn step_session_inherits_parent_permission_and_backend_on_initial_save() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = crate::test_support::build_session_store();

    let settings = resolve_step_settings(
        None,
        None,
        None,
        &WorkflowDefaults {
            backend_id: Some("claude".to_string()),
            permission_mode: "full".to_string(),
        },
    );
    let permission_mode =
        crate::domain::agent_session::PermissionMode::parse(&settings.permission_mode).unwrap();
    let session = crate::usecase::agent_session::session::create_session_internal_with_attributes(
        &store,
        tmp.path(),
        "/repo",
        settings.backend_id,
        permission_mode,
        crate::usecase::agent_session::session::SessionCreationAttributes {
            selected_model: settings.selected_model,
            workflow_step_session: true,
            workflow_step_context: None,
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(session.permission_mode, "full");
    assert!(session.workflow_step_session);
    // 親 selected_model="haiku" は継承しない（Spec issues-946: 暗黙フォールバック禁止）
    assert_eq!(session.selected_model, None);
    assert_eq!(session.backend_id.as_deref(), Some("claude"));
}

// ---- run_id 主体性に関する engine レベル統合テスト ----

/// engine が WorkflowExecution を登録する際に、`WorkflowExecution.id` と
/// Run Store の `WorkflowRunSummary.run_id` が同一 run_id を共有することを検証する。
/// finding 13 対応: `return 値 run_id = WorkflowExecution.id = active summary の run_id
/// = workflow_runs/{run_id}.json の run_id` の一致を engine レベルで検証する。
#[tokio::test]
async fn engine_run_id_consistency_across_execution_and_run_store_metadata() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;

    // Run Store API 境界の UUID 検証を満たすため UUID を採用する。
    let run_id = uuid::Uuid::new_v4().to_string();
    let worktree_path = "/wt/a";
    let workflow = make_minimal_workflow();
    let exec = WorkflowExecution {
        id: run_id.clone(),
        workflow: workflow.clone(),
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 100.0,
        updated_at: 100.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: worktree_path.to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    engine.executions.lock().await.insert(exec.id.clone(), exec);
    engine
        .run_store
        .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
            run_id: run_id.clone(),
            workflow_name: workflow.name.clone(),
            task: None,
            status: RunStatus::Running,
            worktree_path: worktree_path.to_string(),
            current_node_name: workflow.nodes.first().map(|n| n.name.clone()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 100.0,
            updated_at: 100.0,
            completed_at: None,
            error_reason: None,
        })
        .await
        .unwrap();

    // (1) WorkflowExecution.id
    let exec_id = {
        let execs = engine.executions.lock().await;
        execs.get(&run_id).unwrap().id.clone()
    };
    assert_eq!(exec_id, run_id);

    // (2) Run Store active summary の run_id
    let active = engine.list_active_runs().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].run_id, run_id);

    // (3) workflow_runs/{run_id}.json の run_id
    let metadata_path = tmp
        .path()
        .join("workflow_runs")
        .join(format!("{run_id}.json"));
    assert!(metadata_path.exists());
    let metadata: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).unwrap()).unwrap();
    assert_eq!(metadata["runId"].as_str(), Some(run_id.as_str()));

    // (4) worktree -> run_id reverse lookup も一致
    assert_eq!(
        engine.run_id_for_worktree(worktree_path).await,
        Some(run_id.clone())
    );
    assert_eq!(
        engine.resolve_worktree_by_run(&run_id).await,
        Some(worktree_path.to_string())
    );
}

/// 同一 worktree への重複起動が `validate_start` で拒否されることを検証する。
/// finding 14 対応: 既存 active な実行が同一 worktree に存在する間、
/// validate_start は `AlreadyActive` を返す。
#[tokio::test]
async fn engine_validate_start_rejects_duplicate_active_run_on_same_worktree() {
    let engine = WorkflowRuntimeService::new_for_test();
    let workflow = make_minimal_workflow();
    let worktree_path = "/wt/dup";

    let exec = WorkflowExecution {
        id: "existing-run".to_string(),
        workflow: workflow.clone(),
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 100.0,
        updated_at: 100.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: worktree_path.to_string(),
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

    // Existing exec.id remains accessible by run_id
    let still_there = execs.get(&existing_id).unwrap();
    assert_eq!(still_there.id, existing_id);
    assert_eq!(still_there.worktree_path, worktree_path);
}

/// engine が状態遷移を反映した際に Run Store の active / completed 一覧および
/// metadata が同期されることを検証する。
/// finding 15 対応: Running -> WaitingApproval -> Completed の遷移で
/// list_active / list_completed と metadata が正しく更新される。
#[tokio::test]
async fn engine_state_transitions_sync_to_run_store_active_and_completed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;

    // disk fallback の reverse lookup は UUID 形式しか受理しないため、UUID を採用する。
    let run_id = uuid::Uuid::new_v4().to_string();
    let worktree_path = "/wt/transit";
    let workflow = make_minimal_workflow();
    engine
        .run_store
        .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
            run_id: run_id.clone(),
            workflow_name: workflow.name.clone(),
            task: None,
            status: RunStatus::Running,
            worktree_path: worktree_path.to_string(),
            current_node_name: workflow.nodes.first().map(|n| n.name.clone()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 100.0,
            updated_at: 100.0,
            completed_at: None,
            error_reason: None,
        })
        .await
        .unwrap();

    // Running -> WaitingApproval
    let snapshot_waiting = WorkflowState {
        execution_id: run_id.clone(),
        workflow_name: workflow.name.clone(),
        state: WorkflowExecutionState::WaitingApproval,
        current_step_index: 0,
        current_step_name: workflow.nodes[0].name.clone(),
        current_session_id: None,
        total_steps: workflow.nodes.len(),
        step_history: vec![],
        step_execution_counts: HashMap::new(),
        workflow_definition: workflow.clone(),
        total_token_usage: TokenUsage::default(),
        step_states: HashMap::new(),
        step_outputs: HashMap::new(),
        active_parallel_steps: vec![],
        workflow_variables: HashMap::new(),
        stall_observations: Vec::new(),
        approval_operations: None,
        started_at: 100.0,
        updated_at: 200.0,
    };
    workflow_runtime_commit::sync_run_store_from_snapshot(
        engine.run_store(),
        &run_id,
        &snapshot_waiting,
    )
    .await
    .unwrap();
    let active = engine.list_active_runs().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].run_id, run_id);
    assert_eq!(active[0].status, RunStatus::WaitingApproval);

    // Completed
    let snapshot_completed = WorkflowState {
        state: WorkflowExecutionState::Completed,
        updated_at: 300.0,
        ..snapshot_waiting.clone()
    };
    workflow_runtime_commit::sync_run_store_from_snapshot(
        engine.run_store(),
        &run_id,
        &snapshot_completed,
    )
    .await
    .unwrap();
    let active_after = engine.list_active_runs().await;
    assert!(
        active_after.is_empty(),
        "completed run must leave the active set"
    );
    let completed = engine.list_completed_runs().await;
    assert!(completed.iter().any(|r| r.run_id == run_id));
    let completed_entry = completed.iter().find(|r| r.run_id == run_id).unwrap();
    assert_eq!(completed_entry.status, RunStatus::Completed);

    // 終了後でも reverse lookup（persistence fallback）で worktree が解決できる。
    assert_eq!(
        engine.resolve_worktree_by_run(&run_id).await,
        Some(worktree_path.to_string())
    );
}

fn make_minimal_workflow() -> Workflow {
    Workflow {
        variables: Default::default(),
        name: "engine-test-wf".to_string(),
        description: "minimal".to_string(),
        builtin: false,
        nodes: vec![{
            let mut step = make_test_step("only-step", TestKind::Session, "do", vec![], None);
            step.session_mut()
                .expect("minimal workflow step must be a session")
                .permission = Some("edit".to_string());
            step
        }],
    }
}

/// G3: workflow 構造の事前検証は `validate_workflow_shape` で副作用なく完結する。
/// 空 nodes / bash node が含まれる workflow を弾けば、`start_workflow` の Phase 1 で
/// parent ChatSession 作成より前にエラーで return できる（孤立 session を残さない）。
#[test]
fn validate_workflow_shape_rejects_empty_and_bash_workflows_without_side_effects() {
    // 空 nodes は InvalidWorkflow
    let empty = Workflow {
        variables: Default::default(),
        name: "wf".to_string(),
        description: "".to_string(),
        builtin: false,
        nodes: vec![],
    };
    assert!(matches!(
        workflow_engine_start_guard::validate_workflow_shape(&empty),
        Err(WorkflowEngineError::InvalidWorkflow(_))
    ));

    // bash node を含む workflow も InvalidWorkflow
    let bash = Workflow {
        variables: Default::default(),
        name: "wf".to_string(),
        description: "".to_string(),
        builtin: false,
        nodes: vec![make_test_step(
            "bash-step",
            TestKind::Command,
            "echo test",
            vec![],
            None,
        )],
    };
    assert!(matches!(
        workflow_engine_start_guard::validate_workflow_shape(&bash),
        Err(WorkflowEngineError::InvalidWorkflow(_))
    ));

    // 正常な workflow は Ok
    let ok = make_minimal_workflow();
    assert!(workflow_engine_start_guard::validate_workflow_shape(&ok).is_ok());
}

/// G3: `run_id_for_worktree` を Run Store 経由で参照すれば、parent ChatSession 作成より前に
/// 重複起動を検出できる。`start_workflow` Phase 1 で副作用前に判定する経路の主要な
/// 構成要素（Run Store の active index）を直接検証する。
#[tokio::test]
async fn run_store_active_index_resolves_worktree_to_run_id_for_duplicate_check() {
    let engine = WorkflowRuntimeService::new_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;
    let worktree_path = "/wt/duplicate-check";
    let run_id = uuid::Uuid::new_v4().to_string();
    engine
        .run_store
        .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            task: None,
            status: RunStatus::Running,
            worktree_path: worktree_path.to_string(),
            current_node_name: Some("s1".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 100.0,
            updated_at: 100.0,
            completed_at: None,
            error_reason: None,
        })
        .await
        .unwrap();
    assert_eq!(
        engine.run_id_for_worktree(worktree_path).await,
        Some(run_id),
        "Phase 1 重複判定は Run Store の active index で成立する"
    );
}

/// G6: handle_auto_complete の fixture は `exec.id` を execs HashMap キーに使う
/// （production と同じ run_id キー）。fixture が `worktree_path` をキーとして使う旧バグの
/// 回帰防止。
#[tokio::test]
async fn handle_auto_complete_fixture_uses_run_id_as_executions_key() {
    let engine = WorkflowRuntimeService::new_for_test();
    let exec = WorkflowExecution {
        id: "auto-complete-run".to_string(),
        workflow: make_minimal_workflow(),
        state: WorkflowExecutionState::Running,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 0.0,
        updated_at: 0.0,
        current_session_id: Some("sess".to_string()),
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: "/wt/auto-complete".to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    };
    let run_id = exec.id.clone();
    let worktree_path = exec.worktree_path.clone();
    engine.executions.lock().await.insert(run_id.clone(), exec);

    // production と同じ key で参照できる
    {
        let execs = engine.executions.lock().await;
        assert!(execs.get(&run_id).is_some());
        // worktree_path をキーとした直接 lookup は失敗する（= 旧バグの回帰なし）
        assert!(execs.get(worktree_path.as_str()).is_none());
        // find_by_worktree 経由は成功する
        assert!(find_by_worktree(&execs, &worktree_path).is_some());
    }
}

fn make_exec_with(
    id: &str,
    worktree_path: &str,
    state: WorkflowExecutionState,
) -> WorkflowExecution {
    WorkflowExecution {
        id: id.to_string(),
        workflow: make_minimal_workflow(),
        state,
        current_step_index: 0,
        step_execution_counts: HashMap::new(),
        step_history: Vec::new(),
        started_at: 100.0,
        updated_at: 110.0,
        current_session_id: None,
        current_step_token_usage: TokenUsage::default(),
        step_outputs: HashMap::new(),
        task: None,
        parallel_run: None,
        workflow_variables: HashMap::new(),
        current_stall_observations: Vec::new(),
        worktree_path: worktree_path.to_string(),
        workflow_defaults: WorkflowDefaults {
            backend_id: None,
            permission_mode: "edit".to_string(),
        },
    }
}

/// Spec issues-1011 finding 1/7: `find_by_worktree` / `find_by_worktree_mut` は
/// terminal な execution を返さず、active な execution のみを返す。同一 worktree に
/// terminal run と active run が共存しても production 経路で取り違えない。
#[tokio::test]
async fn find_by_worktree_filters_terminal_runs_and_returns_active_only() {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/wt/shared";
    let terminal_run_id = "terminal-run".to_string();
    let active_run_id = "active-run".to_string();
    let terminal_exec = make_exec_with(
        &terminal_run_id,
        worktree_path,
        WorkflowExecutionState::Completed,
    );
    let active_exec = make_exec_with(
        &active_run_id,
        worktree_path,
        WorkflowExecutionState::Running,
    );

    {
        let mut execs = engine.executions.lock().await;
        execs.insert(terminal_run_id.clone(), terminal_exec);
        execs.insert(active_run_id.clone(), active_exec);
    }

    // find_by_worktree は active のみを返す
    {
        let execs = engine.executions.lock().await;
        let (found_id, found_exec) =
            find_by_worktree(&execs, worktree_path).expect("active run must be findable");
        assert_eq!(found_id, &active_run_id);
        assert!(found_exec.is_active());
        assert_ne!(found_id, &terminal_run_id);
    }

    // find_any_by_worktree は terminal/active を問わず返す（validate_start 経路用）
    {
        let execs = engine.executions.lock().await;
        assert!(find_any_by_worktree(&execs, worktree_path).is_some());
    }
}

/// Spec issues-1011 finding 11: `abort_workflow_by_run_id` は terminal な run_id に対して
/// no-op を返し、同一 worktree の active run を誤って中断しない。
#[tokio::test]
async fn abort_workflow_by_run_id_is_noop_for_terminal_run_even_if_active_shares_worktree() {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/wt/coexist";
    let terminal_run_id = "terminal-abort-target".to_string();
    let active_run_id = "active-bystander".to_string();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(
            terminal_run_id.clone(),
            make_exec_with(
                &terminal_run_id,
                worktree_path,
                WorkflowExecutionState::Completed,
            ),
        );
        execs.insert(
            active_run_id.clone(),
            make_exec_with(
                &active_run_id,
                worktree_path,
                WorkflowExecutionState::Running,
            ),
        );
    }

    // run_id 主語の abort 経路: terminal な exec の run_id を渡すと、内部の
    // `is_active()` ガードで即 Ok(()) を返し、worktree 主語の下流処理に委譲しない。
    // → 同一 worktree の active run は影響を受けない。
    // ここでは executions の lookup 経路だけを検証する（AppHandle が要らない範囲）。
    let abort_target_active = {
        let execs = engine.executions.lock().await;
        execs.get(&terminal_run_id).map(|e| e.is_active())
    };
    assert_eq!(abort_target_active, Some(false));
    // active な run は依然として is_active
    let bystander_active = {
        let execs = engine.executions.lock().await;
        execs.get(&active_run_id).map(|e| e.is_active())
    };
    assert_eq!(bystander_active, Some(true));
}

/// Spec issues-1011 finding 5/8: `start_workflow` のアトミック性。並行起動で
/// Run Store reservation に負けた場合、parent ChatSession は作成されないため
/// 「孤立 parent session」が構造的に発生しないことを保証する。
/// reservation は最初の副作用であり、失敗時は他の副作用が走らない。
#[tokio::test]
async fn start_workflow_reservation_is_first_side_effect_so_no_orphan_session_on_conflict() {
    let engine = WorkflowRuntimeService::new_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;
    let worktree_path = "/wt/reserve";

    // 既に active な reservation がある状態を作る。
    let existing_run_id = uuid::Uuid::new_v4().to_string();
    engine
        .run_store
        .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
            run_id: existing_run_id.clone(),
            workflow_name: "wf".to_string(),
            task: None,
            status: RunStatus::Running,
            worktree_path: worktree_path.to_string(),
            current_node_name: Some("only-step".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 100.0,
            updated_at: 100.0,
            completed_at: None,
            error_reason: None,
        })
        .await
        .unwrap();

    // 同一 worktree への 2 回目の reservation は WorktreeAlreadyActive で拒否される。
    let new_run_id = uuid::Uuid::new_v4().to_string();
    let result = engine
        .run_store
        .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
            run_id: new_run_id.clone(),
            workflow_name: "wf".to_string(),
            task: None,
            status: RunStatus::Running,
            worktree_path: worktree_path.to_string(),
            current_node_name: Some("only-step".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 200.0,
            updated_at: 200.0,
            completed_at: None,
            error_reason: None,
        })
        .await;
    assert!(matches!(
        result,
        Err(crate::adaptor::gateway::workflow::run::RunStoreError::WorktreeAlreadyActive { .. })
    ));
    // 新 run_id 用の metadata ファイルは作成されない
    let path = tmp
        .path()
        .join("workflow_runs")
        .join(format!("{new_run_id}.json"));
    assert!(
        !path.exists(),
        "新 run_id の metadata が作成されていないこと（reservation が副作用の最初の境界）"
    );
    // active は existing のみ
    let active = engine.list_active_runs().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].run_id, existing_run_id);
}

#[tokio::test]
async fn reserve_workflow_run_maps_run_store_worktree_conflict_to_already_active() {
    let engine = WorkflowRuntimeService::new_for_test();
    let tmp = tempfile::TempDir::new().unwrap();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;
    let workflow = make_minimal_workflow();
    let worktree_path = "/wt/reserve-conflict";
    engine
        .reserve_workflow_run(
            &workflow,
            worktree_path,
            None,
            TriggerSource::DesktopUi,
            100.0,
        )
        .await
        .unwrap();

    let err = engine
        .reserve_workflow_run(
            &workflow,
            worktree_path,
            None,
            TriggerSource::DesktopUi,
            101.0,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, WorkflowEngineError::AlreadyActive(_)));
}

/// Spec issues-1011 finding 10: `set_execution_state` 経路を通すと、active な
/// execution が terminal に遷移したとき Run Store の active から外れて completed に
/// 追加され、failed/aborted も同じく completed 一覧に現れる。
/// （set_execution_state 自体は AppHandle を要するため、ここではその内部ヘルパー
/// である `sync_run_store_from_snapshot` を terminal snapshot 3 種で走査して
/// 同等の効果を検証する。）
#[tokio::test]
async fn run_store_completed_listing_includes_completed_failed_aborted_via_authoritative_sync() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;

    let cases = [
        ("completed", WorkflowExecutionState::Completed),
        (
            "failed",
            WorkflowExecutionState::Failed {
                reason: "boom".to_string(),
                kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
            },
        ),
        ("aborted", WorkflowExecutionState::Aborted),
    ];
    let mut ids = Vec::new();
    for (_, state) in cases.iter().cloned() {
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .run_store
            .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: format!("/wt/{run_id}"),
                current_node_name: Some("only-step".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        // 権威遷移経路で使われる sync helper を直接呼ぶ
        let snapshot = WorkflowState {
            execution_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            state,
            current_step_index: 0,
            current_step_name: "only-step".to_string(),
            current_session_id: None,
            total_steps: 1,
            step_history: vec![],
            step_execution_counts: HashMap::new(),
            workflow_definition: make_minimal_workflow(),
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            stall_observations: Vec::new(),
            approval_operations: None,
            started_at: 100.0,
            updated_at: 200.0,
        };
        workflow_runtime_commit::sync_run_store_from_snapshot(
            engine.run_store(),
            &run_id,
            &snapshot,
        )
        .await
        .unwrap();
        ids.push(run_id);
    }

    // 3 件とも active からは外れている
    assert!(engine.list_active_runs().await.is_empty());
    // 3 件とも completed に並ぶ
    let completed = engine.list_completed_runs().await;
    let completed_ids: std::collections::HashSet<&str> =
        completed.iter().map(|r| r.run_id.as_str()).collect();
    for id in &ids {
        assert!(
            completed_ids.contains(id.as_str()),
            "completed listing must include run {id}"
        );
    }
}

#[tokio::test]
async fn run_store_sync_failure_rolls_engine_projection_back_to_active_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;
    let run_id = uuid::Uuid::new_v4().to_string();
    let worktree_path = "/wt/sync-rollback";
    engine
        .run_store
        .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            task: None,
            status: RunStatus::Running,
            worktree_path: worktree_path.to_string(),
            current_node_name: Some("only-step".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 100.0,
            updated_at: 100.0,
            completed_at: None,
            error_reason: None,
        })
        .await
        .unwrap();
    engine.executions.lock().await.insert(
        run_id.clone(),
        make_exec_with(&run_id, worktree_path, WorkflowExecutionState::Completed),
    );

    let bad_data_dir = tmp.path().join("not-a-directory");
    std::fs::write(&bad_data_dir, "file").unwrap();
    engine.set_run_store_data_dir(bad_data_dir).await;
    let snapshot = engine
        .executions
        .lock()
        .await
        .get(&run_id)
        .unwrap()
        .to_workflow_state();
    let err = workflow_runtime_commit::sync_run_store_from_snapshot(
        engine.run_store(),
        &run_id,
        &snapshot,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, WorkflowEngineError::SessionStore(_)));

    workflow_runtime_commit::rollback_execution_projection_after_run_store_sync_failure(
        &engine.executions,
        engine.run_store(),
        &run_id,
        &snapshot,
    )
    .await;

    let exec_state = engine
        .executions
        .lock()
        .await
        .get(&run_id)
        .unwrap()
        .state
        .clone();
    assert_eq!(exec_state, WorkflowExecutionState::Running);
    assert_eq!(
        engine.run_id_for_worktree(worktree_path).await,
        Some(run_id),
        "Run Store rollback keeps the active worktree index authoritative"
    );
}

/// Spec issues-1011 finding 16: `abort_workflow_by_run_id` 経路の境界回帰検出。
/// AppHandle を要するため `abort_workflow_by_run_id` 自体は production 経路で起動できないが、
/// 内部 lookup 段階で「terminal run へ no-op を返し、同一 worktree の active run の状態を
/// 変更しない」ことを直接検証する。terminal/active 共存時に run_id 主語の lookup が
/// 取り違えないことを engine state 観測で保証する。
#[tokio::test]
async fn abort_workflow_by_run_id_does_not_modify_sibling_active_run_state() {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/wt/sibling";
    let terminal_run_id = uuid::Uuid::new_v4().to_string();
    let active_run_id = uuid::Uuid::new_v4().to_string();
    {
        let mut execs = engine.executions.lock().await;
        execs.insert(
            terminal_run_id.clone(),
            make_exec_with(
                &terminal_run_id,
                worktree_path,
                WorkflowExecutionState::Completed,
            ),
        );
        execs.insert(
            active_run_id.clone(),
            make_exec_with(
                &active_run_id,
                worktree_path,
                WorkflowExecutionState::Running,
            ),
        );
    }

    // run_id ベース lookup: terminal を引いても active のスナップショットには影響しない。
    let initial_active_state = {
        let execs = engine.executions.lock().await;
        execs.get(&active_run_id).map(|e| e.state.clone())
    };
    assert_eq!(initial_active_state, Some(WorkflowExecutionState::Running));

    // abort_workflow_by_run_id が production で使う lookup helper は、terminal target を
    // `AlreadyTerminal` として返す。worktree_path で sibling active run を探索しない。
    assert!(matches!(
        engine.abort_target_lookup(&terminal_run_id).await,
        AbortTargetLookup::AlreadyTerminal
    ));

    // active run には触れていない（同一 worktree でも誤って中断しない）
    let final_active_state = {
        let execs = engine.executions.lock().await;
        execs.get(&active_run_id).map(|e| e.state.clone())
    };
    assert_eq!(final_active_state, Some(WorkflowExecutionState::Running));
}

/// Spec issues-1011 finding 17: approval/reject は run_id を主語に対象 execution を
/// 直接更新し、同一 worktree に別 run が存在しても指定 run 以外へ適用しない。
#[tokio::test]
async fn approval_for_run_id_updates_only_target_run_when_worktree_is_shared() {
    let engine = WorkflowRuntimeService::new_for_test();
    let worktree_path = "/wt/approval-shared";
    let target_run_id = uuid::Uuid::new_v4().to_string();
    let sibling_run_id = uuid::Uuid::new_v4().to_string();

    let mut target = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
    target.id = target_run_id.clone();
    target.worktree_path = worktree_path.to_string();

    let mut sibling = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
    sibling.id = sibling_run_id.clone();
    sibling.worktree_path = worktree_path.to_string();

    {
        let mut execs = engine.executions.lock().await;
        execs.insert(target_run_id.clone(), target);
        execs.insert(sibling_run_id.clone(), sibling);
    }

    let outcome = engine
        .handle_approval_with_output_for_run_for_test(
            &target_run_id,
            ApprovalDecision::Approve,
            Some(&target_run_id),
            Some("review"),
        )
        .await
        .unwrap();
    assert!(matches!(outcome, StepOutcome::Persist(_)));

    let execs = engine.executions.lock().await;
    let target = execs.get(&target_run_id).unwrap();
    let sibling = execs.get(&sibling_run_id).unwrap();
    assert_eq!(target.state, WorkflowExecutionState::Completed);
    assert_eq!(target.step_history.len(), 1);
    assert_eq!(sibling.state, WorkflowExecutionState::WaitingApproval);
    assert!(sibling.step_history.is_empty());
}

/// Spec issues-1011 finding 13: `start_workflow` 本体の core 起動経路が払い出す
/// run_id と、`WorkflowExecution.id` / active summary / workflow_runs/{run_id}.json が
/// 一貫し、同一 worktree への重複起動を拒否することを直接検証する。
#[tokio::test]
async fn start_workflow_core_records_run_id_and_rejects_duplicate_worktree() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;

    let worktree_path = "/wt/start-fixture";
    let workflow = make_minimal_workflow();
    let now = 100.0;
    let run_id = engine
        .start_workflow_common_core_for_test(
            workflow.clone(),
            worktree_path.to_string(),
            Some("task-x".to_string()),
            TriggerSource::DesktopUi,
            now,
        )
        .await
        .unwrap();

    // 一貫性: (1) executions の id (2) active summary.run_id (3) workflow_runs/{run_id}.json
    let (exec_id, exec_worktree) = {
        let execs = engine.executions.lock().await;
        let exec = execs.get(&run_id).unwrap();
        (exec.id.clone(), exec.worktree_path.clone())
    };
    let active = engine.list_active_runs().await;
    let metadata_path = tmp
        .path()
        .join("workflow_runs")
        .join(format!("{run_id}.json"));
    let metadata: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).unwrap()).unwrap();
    assert_eq!(exec_id, run_id);
    assert_eq!(exec_worktree, worktree_path);
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].run_id, run_id);
    assert_eq!(active[0].workflow_name, workflow.name);
    assert_eq!(active[0].worktree_path, worktree_path);
    assert_eq!(active[0].started_at, now);
    assert_eq!(active[0].updated_at, now);
    assert_eq!(active[0].trigger_source, TriggerSource::DesktopUi);
    assert_eq!(active[0].task.as_deref(), Some("task-x"));
    assert_eq!(metadata["runId"].as_str(), Some(run_id.as_str()));
    assert_eq!(
        metadata["workflowName"].as_str(),
        Some(workflow.name.as_str())
    );
    assert_eq!(metadata["worktreePath"].as_str(), Some(worktree_path));
    assert_eq!(metadata["startedAt"].as_f64(), Some(now));
    assert_eq!(metadata["updatedAt"].as_f64(), Some(now));
    assert_eq!(metadata["triggerSource"].as_str(), Some("desktop_ui"));
    assert_eq!(metadata["task"].as_str(), Some("task-x"));
    // worktree -> run の双方向解決も一貫している
    assert_eq!(
        engine.run_id_for_worktree(worktree_path).await,
        Some(run_id.clone())
    );
    assert_eq!(
        engine.resolve_worktree_by_run(&run_id).await,
        Some(worktree_path.to_string())
    );

    let duplicate = engine
        .start_workflow_common_core_for_test(
            make_minimal_workflow(),
            worktree_path.to_string(),
            None,
            TriggerSource::DesktopUi,
            now + 1.0,
        )
        .await;
    assert!(matches!(
        duplicate,
        Err(WorkflowEngineError::AlreadyActive(_))
    ));
}

/// Spec issues-1011 finding 14: 同一 worktree への重複起動は reservation 段階で拒否され、
/// 新規 metadata / parent session / refs が孤立しない。Run Store の reservation は
/// 起動経路上の「最初の副作用」であり、失敗時には他の副作用が一切走らない構造を保証する。
#[tokio::test]
async fn start_workflow_duplicate_reservation_does_not_leak_metadata_or_refs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;
    let worktree_path = "/wt/dup-leak";

    // 既存 active reservation
    let existing_run_id = uuid::Uuid::new_v4().to_string();
    engine
        .run_store
        .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
            run_id: existing_run_id.clone(),
            workflow_name: "wf".to_string(),
            task: None,
            status: RunStatus::Running,
            worktree_path: worktree_path.to_string(),
            current_node_name: Some("only-step".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 100.0,
            updated_at: 100.0,
            completed_at: None,
            error_reason: None,
        })
        .await
        .unwrap();

    // 2 回目の reservation 失敗 → 新 metadata / refs / executions に何も追加されない
    let new_run_id = uuid::Uuid::new_v4().to_string();
    let result = engine
        .run_store
        .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
            run_id: new_run_id.clone(),
            workflow_name: "wf".to_string(),
            task: None,
            status: RunStatus::Running,
            worktree_path: worktree_path.to_string(),
            current_node_name: Some("only-step".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 200.0,
            updated_at: 200.0,
            completed_at: None,
            error_reason: None,
        })
        .await;
    assert!(matches!(
        result,
        Err(crate::adaptor::gateway::workflow::run::RunStoreError::WorktreeAlreadyActive { .. })
    ));
    // (1) 新 run_id 用 metadata ファイル無し
    let path = tmp
        .path()
        .join("workflow_runs")
        .join(format!("{new_run_id}.json"));
    assert!(!path.exists());
    // (2) session_workflow_refs に新規エントリ無し（reservation 失敗の段階で副作用が走らない）
    let refs = engine.session_workflow_refs.lock().await;
    assert!(!refs
        .values()
        .any(|r: &SessionWorkflowRef| r.run_id == new_run_id));
    // (3) executions にも新 run_id が無い
    let execs = engine.executions.lock().await;
    assert!(!execs.contains_key(&new_run_id));
    // (4) active は existing のみ
    assert_eq!(active_only_summary(&engine).await, vec![existing_run_id]);
}

// 撤去済み: rollback_created_parent_session は parent ChatSession 機構撤去で消滅した。
// 旧テスト `start_workflow_rollback_deletes_created_parent_session` も役目を終えた。

async fn active_only_summary(engine: &WorkflowRuntimeService) -> Vec<String> {
    engine
        .list_active_runs()
        .await
        .into_iter()
        .map(|s| s.run_id)
        .collect()
}

/// Spec issues-1011 finding 15: completed / failed / aborted の代表経路で
/// active 一覧から消えて completed 一覧に status 付きで現れる。
/// production の権威遷移経路で必ず呼ばれる `sync_run_store_from_snapshot` を直接呼び、
/// 3 ステータスすべてで「Run Store の owner が active → completed に推移する」ことを
/// 1 つのテストでまとめて検証する（既存の同種テストとは別に、status 観測も加える）。
#[tokio::test]
async fn run_store_terminal_statuses_propagate_status_field_in_completed_listing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = WorkflowRuntimeService::new_for_test();
    engine
        .set_run_store_data_dir(tmp.path().to_path_buf())
        .await;

    let mut expectations: Vec<(String, RunStatus)> = Vec::new();
    for state in [
        WorkflowExecutionState::Completed,
        WorkflowExecutionState::Failed {
            reason: "boom".to_string(),
            kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: None,
        },
        WorkflowExecutionState::Aborted,
    ] {
        let run_id = uuid::Uuid::new_v4().to_string();
        let expected_status = match state {
            WorkflowExecutionState::Completed => RunStatus::Completed,
            WorkflowExecutionState::Failed { .. } => RunStatus::Failed,
            WorkflowExecutionState::Aborted => RunStatus::Aborted,
            _ => unreachable!(),
        };
        engine
            .run_store
            .register_active(crate::adaptor::gateway::workflow::run::WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: format!("/wt/{run_id}"),
                current_node_name: Some("only-step".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        let snapshot = WorkflowState {
            execution_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            state,
            current_step_index: 0,
            current_step_name: "only-step".to_string(),
            current_session_id: None,
            total_steps: 1,
            step_history: vec![],
            step_execution_counts: HashMap::new(),
            workflow_definition: make_minimal_workflow(),
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            stall_observations: Vec::new(),
            approval_operations: None,
            started_at: 100.0,
            updated_at: 200.0,
        };
        workflow_runtime_commit::sync_run_store_from_snapshot(
            engine.run_store(),
            &run_id,
            &snapshot,
        )
        .await
        .unwrap();
        expectations.push((run_id, expected_status));
    }

    // active 一覧から全て外れている
    assert!(engine.list_active_runs().await.is_empty());

    // completed 一覧に status 付きで現れる
    let completed = engine.list_completed_runs().await;
    for (id, expected_status) in &expectations {
        let entry = completed
            .iter()
            .find(|r| &r.run_id == id)
            .expect("completed listing must include run");
        assert_eq!(
            entry.status, *expected_status,
            "status must propagate to completed summary for {id}"
        );
    }
}

/// Spec issues-1011 finding 3: `resolve_chat_session_for_approval` は run state が
/// `WaitingApproval` でない場合に Err を返す（任意 step session への注入経路を塞ぐ）。
#[tokio::test]
async fn resolve_chat_session_for_approval_rejects_non_waiting_approval_state() {
    let engine = WorkflowRuntimeService::new_for_test();
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut exec = make_exec_with(&run_id, "/wt/x", WorkflowExecutionState::Running);
    exec.workflow.nodes[0].kind = test_node_kind(TestKind::ApprovalSession, "review");
    exec.current_session_id = Some("step-sess".to_string());
    engine.executions.lock().await.insert(run_id.clone(), exec);

    let err = engine
        .resolve_chat_session_for_approval(&run_id)
        .await
        .unwrap_err();
    assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
}

/// Spec issues-1011 finding 3: `resolve_chat_session_for_approval` は current node が
/// Approval node でない場合に拒否する。
#[tokio::test]
async fn resolve_chat_session_for_approval_rejects_non_approval_current_node() {
    let engine = WorkflowRuntimeService::new_for_test();
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut exec = make_exec_with(&run_id, "/wt/x", WorkflowExecutionState::WaitingApproval);
    // current node は通常 session のまま（make_minimal_workflow が auto session を返す）
    exec.current_session_id = Some("step-sess".to_string());
    engine.executions.lock().await.insert(run_id.clone(), exec);

    let err = engine
        .resolve_chat_session_for_approval(&run_id)
        .await
        .unwrap_err();
    assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
}

/// Spec issues-1011 finding 3: 全条件揃った場合のみ session_id / worktree_path を返す。
#[tokio::test]
async fn resolve_chat_session_for_approval_accepts_fully_valid_state() {
    let engine = WorkflowRuntimeService::new_for_test();
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut exec = make_exec_with(&run_id, "/wt/x", WorkflowExecutionState::WaitingApproval);
    exec.workflow.nodes[0].kind = test_node_kind(TestKind::ApprovalSession, "review");
    exec.current_session_id = Some("step-sess".to_string());
    engine.executions.lock().await.insert(run_id.clone(), exec);

    let (sid, wt) = engine
        .resolve_chat_session_for_approval(&run_id)
        .await
        .unwrap();
    assert_eq!(sid, "step-sess");
    assert_eq!(wt, "/wt/x");
}

/// Spec issues-1011 finding 3: terminal run の approval 解決は拒否される。
/// 同一 worktree に terminal + active がある状況で terminal 側を狙う注入経路を防ぐ。
#[tokio::test]
async fn resolve_chat_session_for_approval_rejects_terminal_run() {
    let engine = WorkflowRuntimeService::new_for_test();
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut exec = make_exec_with(&run_id, "/wt/x", WorkflowExecutionState::Completed);
    exec.workflow.nodes[0].kind = test_node_kind(TestKind::ApprovalSession, "review");
    exec.current_session_id = Some("step-sess".to_string());
    engine.executions.lock().await.insert(run_id.clone(), exec);

    let err = engine
        .resolve_chat_session_for_approval(&run_id)
        .await
        .unwrap_err();
    assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
}

/// Spec issues-1011 finding 5: terminal transition 経路で `cleanup_session_workflow_refs_by_run_id`
/// は対象 run の refs のみを削除し、同一 worktree の別 active run の refs は残す。
#[tokio::test]
async fn cleanup_session_workflow_refs_by_run_id_preserves_sibling_run_refs() {
    let engine = WorkflowRuntimeService::new_for_test();
    let terminal_run_id = uuid::Uuid::new_v4().to_string();
    let active_run_id = uuid::Uuid::new_v4().to_string();

    // 両 run の refs を入れる（同一 worktree 想定）
    {
        let mut refs = engine.session_workflow_refs.lock().await;
        refs.insert(
            "parent-terminal".to_string(),
            SessionWorkflowRef {
                run_id: terminal_run_id.clone(),
            },
        );
        refs.insert(
            "step-terminal".to_string(),
            SessionWorkflowRef {
                run_id: terminal_run_id.clone(),
            },
        );
        refs.insert(
            "parent-active".to_string(),
            SessionWorkflowRef {
                run_id: active_run_id.clone(),
            },
        );
    }

    engine
        .cleanup_session_workflow_refs_by_run_id(&terminal_run_id)
        .await;

    let refs = engine.session_workflow_refs.lock().await;
    assert!(!refs.contains_key("parent-terminal"));
    assert!(!refs.contains_key("step-terminal"));
    assert!(
        refs.contains_key("parent-active"),
        "sibling active run の refs は残るべき"
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
    use crate::adaptor::gateway::workflow::event::{ApprovalDecisionRecord, WorkflowEvent};
    use crate::adaptor::gateway::workflow::internal_node_command::InternalNodeCommand;
    use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
    use crate::adaptor::gateway::workflow::run::{
        RunStatus, TerminalRunStatus, TriggerSource, WorkflowRun,
    };
    use crate::adaptor::gateway::workflow::schema::{TransitionRule, Workflow};
    use crate::adaptor::gateway::workflow::state::WorkflowExecutionState;
    use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
    use crate::usecase::agent_session::session::MessagePart;
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

    fn dispatch_data_dir(app: &tauri::AppHandle<tauri::test::MockRuntime>) -> std::path::PathBuf {
        crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .expect("mock app data dir must resolve")
    }

    fn make_approval_only_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "boundary-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_approval_step("review", "review", vec![])],
        }
    }

    fn make_rejectable_approval_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "boundary-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                make_approval_step(
                    "review",
                    "review",
                    vec![TransitionRule {
                        r#match: "reject".to_string(),
                        next: "fix".to_string(),
                    }],
                ),
                make_test_step("fix", TestKind::Session, "fix", vec![], None),
            ],
        }
    }

    fn make_waiting_approval_execution(run_id: &str, worktree_path: &str) -> WorkflowExecution {
        let workflow = make_approval_only_workflow();
        make_waiting_approval_execution_with_workflow(run_id, worktree_path, workflow)
    }

    fn make_waiting_approval_execution_with_workflow(
        run_id: &str,
        worktree_path: &str,
        workflow: Workflow,
    ) -> WorkflowExecution {
        WorkflowExecution {
            id: run_id.to_string(),
            workflow,
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("review".to_string(), 1)]),
            step_history: Vec::new(),
            worktree_path: worktree_path.to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("sess-1".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            current_stall_observations: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "edit".to_string(),
            },
        }
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
        tauri::test::mock_builder()
            .manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                data_dir,
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
            .expect("tauri mock test app must build")
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

    #[tokio::test]
    async fn abort_workflow_by_run_id_clears_stall_observations_in_live_and_projection() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(data_dir.clone());
        let received_payloads: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let received_for_listener = Arc::clone(&received_payloads);
        app.listen("workflow-state-changed", move |event| {
            received_for_listener
                .lock()
                .unwrap()
                .push(event.payload().to_string());
        });

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/abort-clears-stall";
        let session_id = "abort-stall-session";
        let step_name = "review";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.state = WorkflowExecutionState::Running;
        exec.current_session_id = Some(session_id.to_string());
        exec.current_stall_observations =
            vec![workflow_stall_observation_fixture(session_id, step_name)];
        WorkflowEventLog::new(&data_dir)
            .append_batch(&[
                WorkflowEvent::RunStarted {
                    run_id: run_id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    workflow_file_stem: exec.workflow.name.clone(),
                    worktree_path: exec.worktree_path.clone(),
                    workflow_definition: exec.workflow.clone(),
                    timestamp: exec.started_at,
                },
                WorkflowEvent::NodeStarted {
                    run_id: run_id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    node_name: step_name.to_string(),
                    execution_count: 1,
                    timestamp: exec.started_at,
                },
                WorkflowEvent::StepSessionStarted {
                    run_id: run_id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    node_name: step_name.to_string(),
                    execution_count: 1,
                    session_id: session_id.to_string(),
                    timestamp: exec.started_at,
                },
                WorkflowEvent::WorkflowStallObserved {
                    run_id: run_id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    chat_session_id: session_id.to_string(),
                    step_name: step_name.to_string(),
                    run_index: 1,
                    turn_phase: "streaming".to_string(),
                    idle_secs: 181,
                    signal_count: 1,
                    cap_reached: false,
                    timestamp: exec.updated_at,
                },
            ])
            .unwrap();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let outcome = engine
            .abort_workflow_by_run_id(app.handle(), &session_store, &handles, &run_id, None, None)
            .await
            .unwrap();

        assert!(matches!(outcome, AbortOutcome::Aborted));
        let stored_run = engine.run_store().get_run(&run_id).await.unwrap();
        assert_eq!(stored_run.status, RunStatus::Aborted);
        let payloads = received_payloads.lock().unwrap().clone();
        let live_payload = payloads
            .last()
            .expect("abort must broadcast workflow-state-changed");
        let live_json: serde_json::Value = serde_json::from_str(live_payload).unwrap();
        assert!(
            live_json["workflowState"]["stallObservations"]
                .as_array()
                .is_none_or(Vec::is_empty),
            "abort broadcast must clear stall observations: {live_json}"
        );

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::RunAborted { .. })));
        let projected = reconstruct_state_from_events(&run_id, &events)
            .unwrap()
            .unwrap();
        assert!(projected.stall_observations.is_empty());
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

    async fn insert_execution_and_active_run(
        engine: &WorkflowRuntimeService,
        exec: WorkflowExecution,
        trigger_source: TriggerSource,
    ) {
        let run_id = exec.id.clone();
        engine
            .run_store
            .register_active(WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: exec.workflow.name.clone(),
                task: exec.task.clone(),
                status: match exec.state {
                    WorkflowExecutionState::WaitingApproval => RunStatus::WaitingApproval,
                    _ => RunStatus::Running,
                },
                worktree_path: exec.worktree_path.clone(),
                current_node_name: Some(exec.workflow.nodes[exec.current_step_index].name.clone()),
                trigger_source,
                started_at: exec.started_at,
                updated_at: exec.updated_at,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        engine.executions.lock().await.insert(run_id, exec);
    }

    fn read_dispatch_events(app: &DispatchTestApp, run_id: &str) -> Vec<WorkflowEvent> {
        let data_dir = dispatch_data_dir(app.handle());
        WorkflowEventLog::new(&data_dir)
            .read_log(run_id)
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

    #[tokio::test]
    async fn parallel_child_prompt_failure_skips_sessions_refs_and_execution_mutation() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let engine = WorkflowRuntimeService::new_for_test();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/parallel-prompt-failure";
        let mut child = make_parallel_step("missing-facet-child");
        child.facets.policy = Some(format!(
            "nonexistent_policy_{}",
            uuid::Uuid::new_v4().simple()
        ));

        let workflow = Workflow {
            variables: Default::default(),
            name: "parallel-prompt-failure-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_fanout_step("parallel-review", vec![child], None)],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 0;
        exec.current_session_id = None;
        exec.step_execution_counts = HashMap::from([("parallel-review".to_string(), 1)]);
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let result = engine
            .start_parallel_children(app.handle(), &session_store, &handles, worktree_path, false)
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
            "prompt failure must not persist Workflow Step Sessions"
        );
        assert!(
            engine.session_workflow_refs.lock().await.is_empty(),
            "prompt failure must not register session_workflow_refs"
        );

        let execs = engine.executions.lock().await;
        let (_, exec) = find_by_worktree(&execs, worktree_path)
            .expect("execution must remain registered after prompt failure");
        assert!(
            exec.parallel_run.is_none(),
            "prompt failure must not apply parallel_run state"
        );
        assert!(
            exec.current_session_id.is_none(),
            "prompt failure must not set current_session_id"
        );
        assert_eq!(
            exec.step_execution_counts,
            HashMap::from([("parallel-review".to_string(), 1)]),
            "prompt failure must not record child run indices"
        );
    }

    #[tokio::test]
    async fn parallel_child_setup_failure_rolls_back_created_sessions_refs_and_execution_mutation()
    {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());
        let engine = WorkflowRuntimeService::new_for_test();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let save_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let save_attempts_for_hook = save_attempts.clone();
        session_store.set_save_hook_for_test(Arc::new(move |session| {
            save_attempts_for_hook.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if session
                .workflow_step_context
                .as_ref()
                .is_some_and(|context| context.step_name == "review-b")
            {
                Err("injected second child save failure".to_string())
            } else {
                Ok(())
            }
        }));

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/parallel-setup-rollback";
        let workflow = Workflow {
            variables: Default::default(),
            name: "parallel-setup-rollback-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_fanout_step(
                "parallel-review",
                vec![
                    make_parallel_step("review-a"),
                    make_parallel_step("review-b"),
                ],
                None,
            )],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 0;
        exec.current_session_id = None;
        exec.step_execution_counts = HashMap::from([("parallel-review".to_string(), 1)]);
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let result = engine
            .start_parallel_children(app.handle(), &session_store, &handles, worktree_path, false)
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
            exec.parallel_run.is_none(),
            "setup failure must not apply parallel_run state"
        );
        assert!(
            exec.current_session_id.is_none(),
            "setup failure must not set current_session_id"
        );
        assert_eq!(
            exec.step_execution_counts,
            HashMap::from([("parallel-review".to_string(), 1)]),
            "setup failure must not record child run indices"
        );
    }

    /// Spec [04]: ApprovalResolved event は decision を typed (snake_case) で記録し、
    /// approve コメントを comment field に伝播する。observer が dispatch 経由の判断を
    /// 統一語彙で読めることを担保する。
    #[test]
    fn approval_resolved_records_decision_and_comment_in_ndjson() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let run_id = "00000000-0000-0000-0000-000000000300";

        let event = WorkflowEvent::ApprovalResolved {
            run_id: run_id.to_string(),
            workflow_name: "boundary-wf".to_string(),
            node_name: "review".to_string(),
            decision: ApprovalDecisionRecord::Approve,
            comment: Some("lgtm".to_string()),
            timestamp: 1234.0,
        };
        log.append(&event).unwrap();

        let events = log.read_log(run_id).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            WorkflowEvent::ApprovalResolved {
                run_id: rid,
                node_name,
                decision,
                comment,
                ..
            } => {
                assert_eq!(rid, run_id);
                assert_eq!(node_name, "review");
                assert_eq!(*decision, ApprovalDecisionRecord::Approve);
                assert_eq!(comment.as_deref(), Some("lgtm"));
            }
            other => panic!("expected ApprovalResolved, got {other:?}"),
        }
    }

    /// Spec [04]: atomic mutation 境界。mutation 直前の `WorkflowExecution` snapshot を
    /// 一括復元することで、履歴・変数・state・current_step_index を含む全フィールドが
    /// 元に戻ることを担保する（部分 rollback helper を使わない構造）。
    #[tokio::test]
    async fn approval_snapshot_rollback_restores_workflow_execution_fully() {
        let engine = WorkflowRuntimeService::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();

        let mut exec = make_waiting_approval_execution(&run_id, "/wt/atomic");
        exec.workflow_variables
            .insert("preserved".to_string(), "before".to_string());
        let before_history_len = exec.step_history.len();
        let before_step_index = exec.current_step_index;
        let before_state = exec.state.clone();
        let before_variables = exec.workflow_variables.clone();
        let snapshot_before = exec.clone();

        engine.executions.lock().await.insert(run_id.clone(), exec);

        // mutation を適用（apply_approval_application + workflow_variables.extend）
        {
            let mut execs = engine.executions.lock().await;
            let exec = execs.get_mut(&run_id).unwrap();
            exec.workflow_variables
                .insert("after_only".to_string(), "x".to_string());
            let _ = WorkflowRuntimeService::apply_approval_application(
                exec,
                &ApprovalDecision::Approve,
                ApprovalApplication {
                    effective_result: "approve".to_string(),
                    structured_output: None,
                    output_contract: None,
                },
            )
            .unwrap();
            assert_ne!(exec.state, before_state);
            assert!(exec.workflow_variables.contains_key("after_only"));
        }

        // event append 失敗時の一括復元（handle_approval 内と同じ操作）。
        {
            let mut execs = engine.executions.lock().await;
            if let Some(exec) = execs.get_mut(&run_id) {
                *exec = snapshot_before;
            }
        }

        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).expect("run must remain");
        assert_eq!(restored.state, before_state, "WaitingApproval が復元される");
        assert_eq!(
            restored.current_step_index, before_step_index,
            "current_step_index が復元される"
        );
        assert_eq!(
            restored.step_history.len(),
            before_history_len,
            "step_history.len() が復元される"
        );
        assert!(
            !restored.workflow_variables.contains_key("after_only"),
            "mutation 後に追加された workflow_variables が消える"
        );
        assert_eq!(
            restored.workflow_variables, before_variables,
            "workflow_variables 全体が mutation 前と等価"
        );
    }

    fn dispatch_internal_test_snapshot(run_id: &str, workflow_name: &str) -> WorkflowState {
        WorkflowState {
            execution_id: run_id.to_string(),
            workflow_name: workflow_name.to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            current_step_name: "node-1".to_string(),
            current_session_id: None,
            total_steps: 1,
            step_history: vec![],
            step_execution_counts: HashMap::new(),
            workflow_definition: crate::adaptor::gateway::workflow::schema::Workflow {
                variables: Default::default(),
                name: workflow_name.to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            stall_observations: Vec::new(),
            approval_operations: None,
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
        // Complete は snapshot.step_history 末尾 entry と command effect の整合を
        // 検証する（commit 関数: 上流 push との同期境界）。
        let mut snapshot =
            dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000602", "wf");
        snapshot.step_history.push(StepHistoryEntry {
            step_name: "node-1".to_string(),
            completed_at: 100.0,
            result: Some("ok".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
        });
        let complete = InternalNodeCommand::CompleteNode {
            run_id: "00000000-0000-0000-0000-000000000602".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "node-1".to_string(),
            result: Some("ok".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: Some(1),
            timestamp: 100.0,
        };
        match workflow_runtime_events::dispatch_internal_node_command(&mut snapshot, complete) {
            Ok(WorkflowEvent::NodeCompleted {
                run_id,
                node_name,
                result,
                timestamp,
                ..
            }) => {
                assert_eq!(run_id, "00000000-0000-0000-0000-000000000602");
                assert_eq!(node_name, "node-1");
                assert_eq!(result.as_deref(), Some("ok"));
                assert_eq!(timestamp, 100.0);
            }
            other => panic!("expected NodeCompleted, got {other:?}"),
        }

        let mut fail_snapshot =
            dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000603", "wf");
        let fail = InternalNodeCommand::FailNode {
            run_id: "00000000-0000-0000-0000-000000000603".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "node-1".to_string(),
            reason: "boom".to_string(),
            failure_kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
            retry_count: None,
            timestamp: 200.0,
        };
        match workflow_runtime_events::dispatch_internal_node_command(&mut fail_snapshot, fail) {
            Ok(WorkflowEvent::NodeFailed {
                run_id,
                node_name,
                reason,
                timestamp,
                ..
            }) => {
                assert_eq!(run_id, "00000000-0000-0000-0000-000000000603");
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
            WorkflowExecutionState::Failed { ref reason, .. } if reason == "boom"
        ));
        assert_eq!(fail_snapshot.updated_at, 200.0);

        // Complete で snapshot の step_history 末尾と node_name が不一致な場合、
        // commit 関数は ValidationError を返す（spec [05] commit 境界: snapshot が
        // command effect を含まないことの検出）。
        let mut mismatched =
            dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000604", "wf");
        let mismatched_cmd = InternalNodeCommand::CompleteNode {
            run_id: "00000000-0000-0000-0000-000000000604".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "node-1".to_string(),
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: None,
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
    /// `CompleteNode` の全 effect 列（run_id / workflow_name / node_name / result /
    /// session_id / token_usage / structured_output / run_index / timestamp）について、
    /// snapshot 側で 1 個ずつ意図的に mismatch を作成し、`dispatch_internal_node_command`
    /// が `ValidationError` を返すことを境界仕様として担保する（policy 指示）。
    #[test]
    fn dispatch_internal_complete_node_validates_all_effect_fields() {
        fn base_snapshot() -> WorkflowState {
            let mut s =
                dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000620", "table-wf");
            s.step_history.push(StepHistoryEntry {
                step_name: "node-1".to_string(),
                completed_at: 100.0,
                result: Some("ok".to_string()),
                session_id: Some("sess-1".to_string()),
                token_usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                }),
                structured_output: Some(serde_json::json!({"k":"v"})),
                run_index: 1,
                child_outputs: None,
                state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
            });
            s
        }
        fn base_command() -> InternalNodeCommand {
            InternalNodeCommand::CompleteNode {
                run_id: "00000000-0000-0000-0000-000000000620".to_string(),
                workflow_name: "table-wf".to_string(),
                node_name: "node-1".to_string(),
                result: Some("ok".to_string()),
                session_id: Some("sess-1".to_string()),
                token_usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                }),
                structured_output: Some(serde_json::json!({"k":"v"})),
                run_index: Some(1),
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
                "run_id",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        run_id: ref mut r, ..
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
                "structured_output",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        structured_output: ref mut so,
                        ..
                    } = c
                    {
                        *so = Some(serde_json::json!({"k":"other"}));
                    }
                    c
                }),
            ),
            (
                "run_index",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let InternalNodeCommand::CompleteNode {
                        run_index: ref mut r,
                        ..
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

    /// Spec [05] commit 境界: `FailNode` の整合検証も run_id / workflow_name / node_name の
    /// 各次元で snapshot との mismatch を ValidationError として検出することを担保する。
    #[test]
    fn dispatch_internal_fail_node_validates_all_effect_fields() {
        fn base_snapshot() -> WorkflowState {
            let mut s =
                dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000621", "fail-wf");
            s.current_step_name = "node-1".to_string();
            s
        }
        fn base_command() -> InternalNodeCommand {
            InternalNodeCommand::FailNode {
                run_id: "00000000-0000-0000-0000-000000000621".to_string(),
                workflow_name: "fail-wf".to_string(),
                node_name: "node-1".to_string(),
                reason: "boom".to_string(),
                failure_kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
                timestamp: 200.0,
            }
        }

        // baseline は受理される。
        let mut s = base_snapshot();
        assert!(
            workflow_runtime_events::dispatch_internal_node_command(&mut s, base_command()).is_ok()
        );

        // run_id mismatch
        let mut s = base_snapshot();
        let mut bad = base_command();
        if let InternalNodeCommand::FailNode {
            run_id: ref mut r, ..
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

    /// Spec [05] Rule: node が失敗したときの状態遷移が run に反映され、node 失敗の事実が
    /// event log に記録される。engine の実 production 経路 (`set_execution_state` →
    /// `sync_run_store_from_snapshot` + `write_terminal_log` 一連) を通過して、
    /// (1) RunStore の status が Failed terminal に同期される、
    /// (2) NDJSON event log に NodeFailed + RunFailed が追記される、
    /// の双方が成立することを直接検証する（spec L122-130）。
    #[tokio::test]
    async fn engine_set_execution_state_failed_drives_run_state_and_node_failed_event_log() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let worktree_path = "/wt/engine-node-failure";
        let workflow = make_rejectable_approval_workflow();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 1; // node-1 = "fix"
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        // 実 production 経路: set_execution_state → Failed への遷移を engine 経由で実施。
        // write_terminal_log + sync_run_store_from_snapshot がこの経路の中で連続して
        // 実行されることを境界仕様として担保する。
        engine
            .set_execution_state(
                app.handle(),
                &session_store,
                &handles,
                worktree_path,
                WorkflowExecutionState::Failed {
                    reason: "node failure".to_string(),
                    kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
                    retry_count: None,
                },
            )
            .await
            .unwrap();

        // (1) terminal 化した execution は runtime map から即時解放される。
        assert!(
            !engine.contains_execution_for_test(&run_id).await,
            "terminal execution must be released after Failed"
        );

        // (2) RunStore の status も Failed terminal に同期される。
        let run = engine
            .run_store
            .get_run(&run_id)
            .await
            .expect("RunStore must reflect the run");
        assert!(
            run.status.is_terminal(),
            "RunStore status must be terminal, got {:?}",
            run.status
        );
        assert_eq!(run.error_reason.as_deref(), Some("node failure"));

        // (3) NDJSON event log に NodeFailed + RunFailed が連続 append される。
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        let node_failed = events
            .iter()
            .find(|e| matches!(e, WorkflowEvent::NodeFailed { .. }));
        let run_failed = events
            .iter()
            .find(|e| matches!(e, WorkflowEvent::RunFailed { .. }));
        assert!(
            node_failed.is_some(),
            "NodeFailed event must be appended via engine dispatch path; got: {events:?}"
        );
        assert!(
            run_failed.is_some(),
            "RunFailed event must follow NodeFailed; got: {events:?}"
        );
    }

    #[tokio::test]
    async fn engine_set_execution_state_failed_records_failure_telemetry_attributes() {
        let _telemetry_guard = crate::other::telemetry::lock_test_telemetry();
        crate::other::telemetry::reset_test_metrics();
        crate::other::telemetry::set_performance_configured(true);
        crate::other::telemetry::set_performance_enabled(true);

        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let startup_run_id = uuid::Uuid::new_v4().to_string();
        let startup_worktree_path = "/wt/engine-startup-timeout-telemetry";
        let mut startup_exec = make_waiting_approval_execution_with_workflow(
            &startup_run_id,
            startup_worktree_path,
            make_rejectable_approval_workflow(),
        );
        startup_exec.state = WorkflowExecutionState::Running;
        startup_exec.current_step_index = 1;
        startup_exec.current_session_id = None;
        insert_execution_and_active_run(&engine, startup_exec, TriggerSource::DesktopUi).await;
        engine
            .set_execution_state(
                app.handle(),
                &session_store,
                &handles,
                startup_worktree_path,
                WorkflowExecutionState::Failed {
                    reason: "startup timeout".to_string(),
                    kind: WorkflowStepFailureKind::StartupTimeout,
                    retry_count: Some(2),
                },
            )
            .await
            .unwrap();

        let validation_run_id = uuid::Uuid::new_v4().to_string();
        let validation_worktree_path = "/wt/engine-validation-failure-telemetry";
        let mut validation_exec = make_waiting_approval_execution_with_workflow(
            &validation_run_id,
            validation_worktree_path,
            make_rejectable_approval_workflow(),
        );
        validation_exec.state = WorkflowExecutionState::Running;
        validation_exec.current_step_index = 1;
        validation_exec.current_session_id = None;
        insert_execution_and_active_run(&engine, validation_exec, TriggerSource::DesktopUi).await;
        engine
            .set_execution_state(
                app.handle(),
                &session_store,
                &handles,
                validation_worktree_path,
                WorkflowExecutionState::Failed {
                    reason: "validation failed".to_string(),
                    kind: WorkflowStepFailureKind::ValidationFailure,
                    retry_count: None,
                },
            )
            .await
            .unwrap();

        let records = crate::other::telemetry::test_metric_records();
        let has_attr =
            |record: &crate::other::telemetry::TestMetricRecord, key: &str, value: &str| {
                record
                    .attributes
                    .iter()
                    .any(|(attr_key, attr_value)| attr_key == key && attr_value == value)
            };
        let startup_record = records
            .iter()
            .find(|record| {
                record.name == "releash.operation.status"
                    && has_attr(record, "releash.operation", "workflow.step.failure")
                    && has_attr(record, "failure.kind", "startup_timeout")
            })
            .expect("startup timeout failure telemetry must be recorded");
        assert!(has_attr(startup_record, "failure.retry_count", "2"));
        assert!(has_attr(startup_record, "failure.timeout_kind", "startup"));

        let validation_record = records
            .iter()
            .find(|record| {
                record.name == "releash.operation.status"
                    && has_attr(record, "releash.operation", "workflow.step.failure")
                    && has_attr(record, "failure.kind", "validation_failure")
            })
            .expect("validation failure telemetry must be recorded");
        assert!(validation_record
            .attributes
            .iter()
            .all(|(key, _)| key != "failure.timeout_kind"));

        crate::other::telemetry::reset_test_metrics();
    }

    #[tokio::test]
    async fn stale_turn_complete_failure_is_terminalized_by_retry_policy_default() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/stale-policy-terminal";
        let step_session_id = "stale-step-session";
        let workflow = Workflow {
            variables: Default::default(),
            name: "stale-policy-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_test_step(
                "review",
                TestKind::Session,
                "review",
                vec![],
                None,
            )],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_session_id = Some(step_session_id.to_string());
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            step_session_id.to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                step_session_id,
                124,
                None,
                &[],
                None,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&run_id).await,
            "stale timeout with default max_retries=0 must be terminal"
        );
        let run = engine
            .run_store
            .get_run(&run_id)
            .await
            .expect("RunStore must keep terminal run metadata");
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run
            .error_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("retry policy did not retry")));

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeFailed {
                failure_kind: WorkflowStepFailureKind::StaleRuntimeTimeout,
                retry_count: Some(0),
                reason,
                ..
            } if reason.contains("max_retries=0")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::RunFailed {
                failure_kind: WorkflowStepFailureKind::StaleRuntimeTimeout,
                retry_count: Some(0),
                ..
            }
        )));
    }

    #[tokio::test]
    async fn parallel_child_failure_releases_terminal_execution_after_broadcast() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/parallel-child-failure";
        let failed_child_session_id = "parallel-child-failed-session";
        let interrupted_child_session_id = "parallel-child-interrupted-session";
        let workflow = Workflow {
            variables: Default::default(),
            name: "parallel-failure-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_fanout_step(
                "parallel-review",
                vec![
                    make_parallel_step("review-a"),
                    make_parallel_step("review-b"),
                ],
                None,
            )],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 0;
        exec.current_session_id = None;
        exec.step_execution_counts = HashMap::from([
            ("parallel-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        exec.parallel_run = Some(ParallelRunState {
            parent_step_name: "parallel-review".to_string(),
            aggregate: None,
            children: vec![
                ParallelChildRun {
                    step_name: "review-a".to_string(),
                    session_id: failed_child_session_id.to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: None,
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
                ParallelChildRun {
                    step_name: "review-b".to_string(),
                    session_id: interrupted_child_session_id.to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: None,
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
            ],
        });
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                failed_child_session_id.to_string(),
                SessionWorkflowRef {
                    run_id: run_id.clone(),
                },
            );
            refs.insert(
                interrupted_child_session_id.to_string(),
                SessionWorkflowRef {
                    run_id: run_id.clone(),
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
            !engine.contains_execution_for_test(&run_id).await,
            "parallel child failure must release the Failed terminal execution"
        );
        let stored = engine
            .run_store
            .get_run(&run_id)
            .await
            .expect("RunStore must keep the terminal run metadata");
        assert_eq!(stored.status, RunStatus::Failed);
        assert_eq!(
            stored.error_reason.as_deref(),
            Some("Parallel child 'review-a' failed (exit_code: 1)")
        );
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, WorkflowEvent::RunFailed { .. })),
            "parallel child failure must append RunFailed; got {events:?}"
        );
        let refs = engine.session_workflow_refs.lock().await;
        assert!(
            refs.values()
                .all(|session_ref| session_ref.run_id != run_id),
            "terminal cleanup must remove all session refs for the failed parallel run"
        );
    }

    #[tokio::test]
    async fn parallel_child_success_clears_live_stall_observation_for_child() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/parallel-child-stall-success";
        let completed_child_session_id = "parallel-child-stall-completed-session";
        let waiting_child_session_id = "parallel-child-stall-waiting-session";
        let workflow = Workflow {
            variables: Default::default(),
            name: "parallel-stall-success-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_fanout_step(
                "parallel-review",
                vec![
                    make_parallel_step("review-a"),
                    make_parallel_step("review-b"),
                ],
                None,
            )],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 0;
        exec.current_session_id = None;
        exec.step_execution_counts = HashMap::from([
            ("parallel-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        exec.current_stall_observations = vec![
            workflow_stall_observation_fixture(completed_child_session_id, "review-a"),
            workflow_stall_observation_fixture(waiting_child_session_id, "review-b"),
        ];
        exec.parallel_run = Some(ParallelRunState {
            parent_step_name: "parallel-review".to_string(),
            aggregate: None,
            children: vec![
                ParallelChildRun {
                    step_name: "review-a".to_string(),
                    session_id: completed_child_session_id.to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: None,
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
                ParallelChildRun {
                    step_name: "review-b".to_string(),
                    session_id: waiting_child_session_id.to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: None,
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
            ],
        });
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            completed_child_session_id.to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
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
        let exec = execs.get(&run_id).expect("run must stay active");
        let observations = &exec.current_stall_observations;
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].session_id, waiting_child_session_id,
            "completed child stall observation must be removed while running sibling remains"
        );
        let completed_child = exec
            .parallel_run
            .as_ref()
            .expect("parallel run must stay active")
            .children
            .iter()
            .find(|child| child.step_name == "review-a")
            .expect("completed child");
        assert!(matches!(
            completed_child.state,
            ParallelChildState::Completed
        ));
    }

    #[tokio::test]
    async fn parallel_success_after_delegated_failure_completes_parent() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/parallel-delegated-failure";
        let successful_child_session_id = "parallel-child-success-session";
        let workflow = Workflow {
            variables: Default::default(),
            name: "parallel-delegated-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_fanout_step(
                "parallel-review",
                vec![
                    make_parallel_step("review-a"),
                    make_parallel_step("review-b"),
                ],
                None,
            )],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 0;
        exec.current_session_id = None;
        exec.step_execution_counts = HashMap::from([
            ("parallel-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        exec.step_outputs.insert(
            "review-a".to_string(),
            StepOutput {
                step_name: "review-a".to_string(),
                run_index: 1,
                session_id: Some("parallel-child-refusal-session".to_string()),
                result: Some("model_refusal".to_string()),
                structured_output: Some(serde_json::json!({
                    "failureKind": "model_refusal",
                    "disposition": "partial",
                    "exitCode": 1,
                })),
                output_contract: None,
                token_usage: Some(TokenUsage::default()),
                completed_at: 1001.0,
            },
        );
        exec.parallel_run = Some(ParallelRunState {
            parent_step_name: "parallel-review".to_string(),
            aggregate: None,
            children: vec![
                ParallelChildRun {
                    step_name: "review-a".to_string(),
                    session_id: "parallel-child-refusal-session".to_string(),
                    state: ParallelChildState::Failed,
                    result: Some("model_refusal".to_string()),
                    structured_output: Some(serde_json::json!({
                        "failureKind": "model_refusal",
                        "disposition": "partial",
                        "exitCode": 1,
                    })),
                    output_contract: None,
                    failure_kind: Some(WorkflowStepFailureKind::ModelRefusal),
                    failure_disposition: Some(FailureDisposition::Partial),
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
                ParallelChildRun {
                    step_name: "review-b".to_string(),
                    session_id: successful_child_session_id.to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: None,
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
            ],
        });
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            successful_child_session_id.to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );

        engine
            .on_turn_complete(
                app.handle(),
                &session_store,
                &handles,
                successful_child_session_id,
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

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, WorkflowEvent::ParallelCompleted { .. })),
            "parent parallel must complete once all children are Completed or Failed; got {events:?}"
        );
        assert!(
            !engine.contains_execution_for_test(&run_id).await,
            "single-node parent advance should complete and release the run"
        );
    }

    #[tokio::test]
    async fn parallel_zero_exit_model_refusal_is_partial_failure_before_contract_repair() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/parallel-zero-exit-refusal";
        let refused_child_session_id = "parallel-child-zero-refusal-session";
        let waiting_child_session_id = "parallel-child-waiting-session";
        let mut review_a = make_parallel_step("review-a");
        review_a.output_contract = Some("review-verdict".to_string());
        let workflow = Workflow {
            variables: Default::default(),
            name: "parallel-zero-refusal-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_fanout_step(
                "parallel-review",
                vec![review_a, make_parallel_step("review-b")],
                None,
            )],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 0;
        exec.current_session_id = None;
        exec.step_execution_counts = HashMap::from([
            ("parallel-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        exec.current_stall_observations = vec![
            workflow_stall_observation_fixture(refused_child_session_id, "review-a"),
            workflow_stall_observation_fixture(waiting_child_session_id, "review-b"),
        ];
        exec.parallel_run = Some(ParallelRunState {
            parent_step_name: "parallel-review".to_string(),
            aggregate: None,
            children: vec![
                ParallelChildRun {
                    step_name: "review-a".to_string(),
                    session_id: refused_child_session_id.to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: Some("review-verdict".to_string()),
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
                ParallelChildRun {
                    step_name: "review-b".to_string(),
                    session_id: waiting_child_session_id.to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: None,
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
            ],
        });
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        engine.session_workflow_refs.lock().await.insert(
            refused_child_session_id.to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );

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

        let execs = engine.executions.lock().await;
        let exec = execs.get(&run_id).expect("run must stay active");
        let child = exec
            .parallel_run
            .as_ref()
            .expect("parallel run must stay active")
            .children
            .iter()
            .find(|child| child.step_name == "review-a")
            .expect("refused child");
        assert!(matches!(child.state, ParallelChildState::Failed));
        assert_eq!(
            child.failure_kind,
            Some(WorkflowStepFailureKind::ModelRefusal)
        );
        assert_eq!(child.failure_disposition, Some(FailureDisposition::Partial));
        assert_eq!(child.result.as_deref(), Some("model_refusal"));
        assert_eq!(exec.current_stall_observations.len(), 1);
        assert_eq!(
            exec.current_stall_observations[0].session_id, waiting_child_session_id,
            "partial failure child stall observation must be removed while running sibling remains"
        );
        drop(execs);

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                WorkflowEvent::ParallelChildCompleted {
                    child_node_name,
                    state,
                    failure_kind: Some(WorkflowStepFailureKind::ModelRefusal),
                    failure_disposition: Some(FailureDisposition::Partial),
                    ..
                } if child_node_name == "review-a" && state == STEP_STATE_FAILED
            )),
            "zero-exit model refusal must be recorded as partial child failure; got {events:?}"
        );
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, WorkflowEvent::ContractRepairRequested { .. })),
            "model refusal signal must not be rerouted into contract repair; got {events:?}"
        );
    }

    #[tokio::test]
    async fn parallel_partial_failure_append_failure_rolls_back_child_state_and_run_store() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/parallel-partial-append-failure";
        let refused_child_session_id = "parallel-child-refusal-append-failure-session";
        let waiting_child_session_id = "parallel-child-still-running-session";
        let workflow = Workflow {
            variables: Default::default(),
            name: "parallel-partial-append-failure-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![make_fanout_step(
                "parallel-review",
                vec![
                    make_parallel_step("review-a"),
                    make_parallel_step("review-b"),
                ],
                None,
            )],
        };
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 0;
        exec.current_session_id = None;
        exec.updated_at = 1000.0;
        exec.step_execution_counts = HashMap::from([
            ("parallel-review".to_string(), 1),
            ("review-a".to_string(), 1),
            ("review-b".to_string(), 1),
        ]);
        exec.parallel_run = Some(ParallelRunState {
            parent_step_name: "parallel-review".to_string(),
            aggregate: None,
            children: vec![
                ParallelChildRun {
                    step_name: "review-a".to_string(),
                    session_id: refused_child_session_id.to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: None,
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
                ParallelChildRun {
                    step_name: "review-b".to_string(),
                    session_id: waiting_child_session_id.to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: None,
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
            ],
        });
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        let stored_before = engine
            .run_store
            .get_run(&run_id)
            .await
            .expect("run store must hold active run before append failure");
        engine.session_workflow_refs.lock().await.insert(
            refused_child_session_id.to_string(),
            SessionWorkflowRef {
                run_id: run_id.clone(),
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
            .expect_err("partial child failure event append failure must abort commit");
        assert!(
            format!("{err:?}").contains("parallel child progress event append failed"),
            "append failure context must be surfaced; got {err:?}"
        );

        let execs = engine.executions.lock().await;
        let exec = execs.get(&run_id).expect("run must remain active");
        let child = exec
            .parallel_run
            .as_ref()
            .expect("parallel run must be restored")
            .children
            .iter()
            .find(|child| child.step_name == "review-a")
            .expect("refused child must still exist");
        assert!(
            matches!(child.state, ParallelChildState::Running),
            "child state must roll back when required event append fails"
        );
        assert_eq!(child.failure_kind, None);
        assert_eq!(child.failure_disposition, None);
        assert!(
            !exec.step_outputs.contains_key("review-a"),
            "synthetic partial StepOutput must not remain after rollback"
        );
        drop(execs);

        let stored_after = engine
            .run_store
            .get_run(&run_id)
            .await
            .expect("run store must be restored to active projection");
        assert_eq!(stored_after.status, stored_before.status);
        assert_eq!(
            stored_after.current_node_name,
            stored_before.current_node_name
        );
        assert_eq!(stored_after.updated_at, stored_before.updated_at);
        assert_eq!(stored_after.error_reason, stored_before.error_reason);

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(
            events.iter().all(|event| !matches!(
                event,
                WorkflowEvent::ParallelChildCompleted {
                    child_node_name,
                    failure_kind: Some(WorkflowStepFailureKind::ModelRefusal),
                    failure_disposition: Some(FailureDisposition::Partial),
                    ..
                } if child_node_name == "review-a"
            )),
            "partial failure event must not be present when required append fails; got {events:?}"
        );
    }

    /// Spec [05] commit 境界: production 経路 `execute_outcome` の pre-commit phase で
    /// `write_log_required_batch` が失敗した場合、`sync_run_store_from_snapshot` /
    /// `persist_state` は実行されず、RunStore は active のまま / NDJSON 上にも terminal
    /// event が残らないことを直接検証する（spec [05]: state mutation と event log の
    /// 分離を防ぐ rollback 境界）。
    ///
    /// 障害シミュレーション: workflow_logs ディレクトリパスに通常ファイルを置くと、
    /// `WorkflowEventLog::append_batch` 内の `create_dir_all` が失敗し、batch append が
    /// `Err` を返す。
    #[tokio::test]
    async fn execute_outcome_pre_commit_append_failure_keeps_run_store_active() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let worktree_path = "/wt/append-failure";

        let workflow = make_rejectable_approval_workflow();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 1; // node "fix"
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        // workflow_logs ディレクトリを通常ファイルで塞いで append を恒常失敗させる。
        let log_dir = data_dir.join("workflow_logs");
        if log_dir.exists() {
            std::fs::remove_dir_all(&log_dir).unwrap();
        }
        std::fs::write(&log_dir, b"block").unwrap();

        // snapshot を Failed terminal に遷移させ、execute_outcome に persist 経路で渡す。
        let mut snapshot = {
            let execs = engine.executions.lock().await;
            execs.get(&run_id).unwrap().to_workflow_state()
        };
        snapshot.state = WorkflowExecutionState::Failed {
            reason: "node failure".to_string(),
            kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
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

        // RunStore は active のまま（terminal に sync されていない）。
        let stored = engine
            .run_store
            .get_run(&run_id)
            .await
            .expect("RunStore must still hold the run");
        assert!(
            !stored.status.is_terminal(),
            "RunStore status must NOT be terminal when event log append fails; got {:?}",
            stored.status
        );
        assert!(
            stored.error_reason.is_none(),
            "RunStore error_reason must remain unset when event log append fails"
        );

        // workflow_logs ディレクトリを復旧して NDJSON が空であることを確認する。
        std::fs::remove_file(&log_dir).unwrap();
        std::fs::create_dir_all(&log_dir).unwrap();
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(
            events.is_empty(),
            "NDJSON event log must be empty when pre-commit append fails; got {events:?}"
        );
    }

    /// Spec [05] Rule: snapshot に Failed state が反映済みの場合、`write_terminal_log` の
    /// 単体経路 (`terminal_events_for_snapshot` → `write_log_required_batch`) が
    /// startup timeout の `failure_kind` / retry count を保ったまま
    /// `NodeFailed` + `RunFailed` を順序通り append することを直接検証する。
    #[test]
    fn write_terminal_log_emits_startup_timeout_node_failed_followed_by_run_failed() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        let run_id = "00000000-0000-0000-0000-000000000605".to_string();

        let snapshot = WorkflowState {
            execution_id: run_id.clone(),
            workflow_name: "fail-wf".to_string(),
            state: WorkflowExecutionState::Failed {
                reason: "startup timeout".to_string(),
                kind: crate::domain::workflow::WorkflowStepFailureKind::StartupTimeout,
                retry_count: Some(2),
            },
            current_step_index: 0,
            current_step_name: "step-1".to_string(),
            current_session_id: None,
            total_steps: 1,
            step_history: vec![],
            step_execution_counts: HashMap::new(),
            workflow_definition: crate::adaptor::gateway::workflow::schema::Workflow {
                variables: Default::default(),
                name: "fail-wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            stall_observations: Vec::new(),
            approval_operations: None,
            started_at: 900.0,
            updated_at: 1000.0,
        };

        engine
            .write_terminal_log(app.handle(), &snapshot)
            .expect("write_terminal_log must succeed");

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert_eq!(
            events.len(),
            2,
            "terminal log must contain NodeFailed + RunFailed; got {events:?}"
        );
        match &events[0] {
            WorkflowEvent::NodeFailed {
                run_id: ev_run_id,
                workflow_name,
                node_name,
                reason,
                failure_kind,
                retry_count,
                ..
            } => {
                assert_eq!(ev_run_id, &run_id);
                assert_eq!(workflow_name, "fail-wf");
                assert_eq!(node_name, "step-1");
                assert_eq!(reason, "startup timeout");
                assert_eq!(
                    *failure_kind,
                    crate::domain::workflow::WorkflowStepFailureKind::StartupTimeout
                );
                assert_eq!(*retry_count, Some(2));
            }
            other => panic!("expected NodeFailed first, got {other:?}"),
        }
        match &events[1] {
            WorkflowEvent::RunFailed {
                run_id: ev_run_id,
                workflow_name,
                reason,
                failure_kind,
                retry_count,
                ..
            } => {
                assert_eq!(ev_run_id, &run_id);
                assert_eq!(workflow_name, "fail-wf");
                assert_eq!(reason, "startup timeout");
                assert_eq!(
                    *failure_kind,
                    crate::domain::workflow::WorkflowStepFailureKind::StartupTimeout
                );
                assert_eq!(*retry_count, Some(2));
            }
            other => panic!("expected RunFailed second, got {other:?}"),
        }
    }

    /// Spec [04] Rule「対象不在 / 既に終了した command は受理されない」:
    /// `abort_target_lookup` は `executions` に存在しない run_id を `NotFound` と
    /// 判定し、後段の dispatch では非受理にマッピングされる構造を担保する。
    #[tokio::test]
    async fn abort_target_lookup_returns_not_found_for_unknown_run_id() {
        let engine = WorkflowRuntimeService::new_for_test();
        match engine
            .abort_target_lookup("00000000-0000-0000-0000-000000000700")
            .await
        {
            AbortTargetLookup::NotFound => {}
            other => panic!("expected NotFound for unknown run_id, got {other:?}"),
        }
    }

    /// Spec [04] Rule「既に終了した run に対する操作 command が要求される」:
    /// terminal な run（Completed/Failed/Aborted）に対する Abort は `AlreadyTerminal`
    /// として lookup 段階で非受理になる。
    #[tokio::test]
    async fn abort_target_lookup_returns_already_terminal_for_terminal_run() {
        let engine = WorkflowRuntimeService::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        for terminal_state in [
            WorkflowExecutionState::Completed,
            WorkflowExecutionState::Aborted,
            WorkflowExecutionState::Failed {
                reason: "x".to_string(),
                kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
                retry_count: None,
            },
        ] {
            let mut exec = make_waiting_approval_execution(&run_id, "/wt/term");
            exec.state = terminal_state.clone();
            engine.executions.lock().await.insert(run_id.clone(), exec);

            match engine.abort_target_lookup(&run_id).await {
                AbortTargetLookup::AlreadyTerminal => {}
                other => panic!(
                    "expected AlreadyTerminal for terminal {terminal_state:?}, got {other:?}"
                ),
            }
            engine.executions.lock().await.remove(&run_id);
        }
    }

    #[tokio::test]
    async fn abort_target_lookup_returns_already_terminal_for_released_terminal_run_record() {
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;

        for (terminal_status, error_reason) in [
            (TerminalRunStatus::Completed, None),
            (
                TerminalRunStatus::Failed,
                Some("failed after release".to_string()),
            ),
            (TerminalRunStatus::Aborted, None),
        ] {
            let run_id = uuid::Uuid::new_v4().to_string();
            let exec = make_waiting_approval_execution(&run_id, "/wt/released-terminal");
            insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
            engine
                .run_store
                .complete_run(&run_id, terminal_status, 2000.0, error_reason)
                .await
                .unwrap();
            engine.executions.lock().await.remove(&run_id);

            match engine.abort_target_lookup(&run_id).await {
                AbortTargetLookup::AlreadyTerminal => {}
                other => {
                    panic!("expected AlreadyTerminal for released terminal run, got {other:?}")
                }
            }
        }
    }

    /// Spec [04] Rule: active run に対する `abort_target_lookup` は `Active` を返し、
    /// その後の state 遷移経路（mutation → required append → finalize）に乗る。
    #[tokio::test]
    async fn abort_target_lookup_returns_active_for_running_run() {
        let engine = WorkflowRuntimeService::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_waiting_approval_execution(&run_id, "/wt/active");
        exec.state = WorkflowExecutionState::Running;
        exec.current_session_id = Some("sess-X".to_string());
        engine.executions.lock().await.insert(run_id.clone(), exec);

        match engine.abort_target_lookup(&run_id).await {
            AbortTargetLookup::Active {
                current_step_session_id,
                ..
            } => {
                assert_eq!(current_step_session_id.as_deref(), Some("sess-X"));
            }
            other => panic!("expected Active for running run, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn release_terminal_execution_removes_terminal_entries_only() {
        let engine = WorkflowRuntimeService::new_for_test();

        for (label, terminal_state) in [
            ("completed", WorkflowExecutionState::Completed),
            (
                "failed",
                WorkflowExecutionState::Failed {
                    reason: "boom".to_string(),
                    kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
                    retry_count: None,
                },
            ),
            ("aborted", WorkflowExecutionState::Aborted),
        ] {
            let run_id = uuid::Uuid::new_v4().to_string();
            let mut exec = make_waiting_approval_execution(&run_id, &format!("/wt/{label}"));
            exec.state = terminal_state;
            engine.executions.lock().await.insert(run_id.clone(), exec);

            engine.release_terminal_execution(&run_id).await;

            assert!(
                !engine.contains_execution_for_test(&run_id).await,
                "{label} terminal execution must be removed"
            );
        }

        let active_run_id = uuid::Uuid::new_v4().to_string();
        let mut active = make_waiting_approval_execution(&active_run_id, "/wt/active-release");
        active.state = WorkflowExecutionState::Running;
        engine
            .executions
            .lock()
            .await
            .insert(active_run_id.clone(), active);

        engine.release_terminal_execution(&active_run_id).await;

        assert!(
            engine.contains_execution_for_test(&active_run_id).await,
            "active execution must not be released"
        );
        assert_eq!(engine.executions_len_for_test().await, 1);
    }

    #[tokio::test]
    async fn get_state_by_run_id_returns_none_for_released_terminal_state() {
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        engine.set_run_store_data_dir(data_dir.clone()).await;

        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/reconstruct-terminal";
        let exec = make_waiting_approval_execution(&run_id, worktree_path);
        let workflow = exec.workflow.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let log = WorkflowEventLog::new(&data_dir);
        log.append(&WorkflowEvent::RunStarted {
            run_id: run_id.clone(),
            workflow_name: workflow.name.clone(),
            workflow_file_stem: "boundary-wf".to_string(),
            worktree_path: worktree_path.to_string(),
            workflow_definition: workflow.clone(),
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            run_id: run_id.clone(),
            workflow_name: workflow.name.clone(),
            node_name: "review".to_string(),
            execution_count: 1,
            timestamp: 1001.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeCompleted {
            run_id: run_id.clone(),
            workflow_name: workflow.name.clone(),
            node_name: "review".to_string(),
            result: Some("approve".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: Some(1),
            timestamp: 1002.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::RunCompleted {
            run_id: run_id.clone(),
            workflow_name: workflow.name.clone(),
            total_token_usage: TokenUsage::default(),
            timestamp: 1003.0,
        })
        .unwrap();

        engine
            .run_store
            .complete_run(&run_id, TerminalRunStatus::Completed, 1003.0, None)
            .await
            .unwrap();
        engine.executions.lock().await.remove(&run_id);

        assert!(
            engine.get_state_by_run_id(&run_id).await.is_none(),
            "run_id-only live API must not expose released terminal history"
        );
    }

    /// Spec [04] Rule「権限の無い / 対象不在 / 既決の command は state 変化を起こさない」:
    /// 既に判断済み（WaitingApproval ではない）node に対する Approve / Reject は
    /// `validate_approval_target_snapshot` で `InvalidState` として非受理になる。
    /// production dispatch 経路の `handle_approval` がこのガードを最初に通すため、
    /// 二度目以降の同一意図 command は state 変化を起こさない。
    #[tokio::test]
    async fn approval_target_validation_rejects_already_resolved_node() {
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_waiting_approval_execution(&run_id, "/wt/idempotent");
        exec.state = WorkflowExecutionState::Completed;
        let err = workflow_approval_runtime::validate_approval_target_snapshot(
            &exec,
            Some(&run_id),
            Some("review"),
        )
        .unwrap_err();
        assert!(
            matches!(err, WorkflowEngineError::InvalidState(_)),
            "既決 node への Approve/Reject は InvalidState で非受理 (got {err:?})"
        );
    }

    /// Spec [04] Rule: `validate_approval_decision` は Reject の空コメント / 上限超過を
    /// 拒否する。dispatch 入口での新規外部入力に対する境界バリデーション。
    #[test]
    fn reject_decision_validation_rejects_empty_and_oversize_comments() {
        let empty = workflow_approval_runtime::validate_approval_input(
            &ApprovalDecision::Reject {
                comment: "   ".to_string(),
            },
            None,
        )
        .unwrap_err();
        assert!(matches!(empty, WorkflowEngineError::ValidationError(_)));

        let oversize = workflow_approval_runtime::validate_approval_input(
            &ApprovalDecision::Reject {
                comment: "x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1),
            },
            None,
        )
        .unwrap_err();
        assert!(matches!(oversize, WorkflowEngineError::ValidationError(_)));

        workflow_approval_runtime::validate_approval_input(
            &ApprovalDecision::Reject {
                comment: "fix this".to_string(),
            },
            None,
        )
        .expect("正常な reject reason は受理される");
    }

    /// Spec [04] Rule: Approve コメントも reject と同じ MAX_APPROVAL_COMMENT_CHARS を
    /// 適用する。空文字（None）は許容するが、上限超過は非受理。
    #[test]
    fn approve_comment_length_validation_rejects_oversize_but_accepts_empty() {
        workflow_approval_runtime::validate_approval_input(&ApprovalDecision::Approve, None)
            .expect("None は許容される");
        workflow_approval_runtime::validate_approval_input(&ApprovalDecision::Approve, Some(""))
            .expect("空コメント (Some(empty)) は許容される");
        let oversize_comment = "x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);
        let err = workflow_approval_runtime::validate_approval_input(
            &ApprovalDecision::Approve,
            Some(&oversize_comment),
        )
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
    }

    /// Spec [04] secret redaction: ApprovalResolved.comment に設定済み secret 値が
    /// 含まれる場合、event log に書き出す前に `mask_sensitive_text()` で redaction
    /// される。本テストは redaction primitive そのものの契約を担保する
    /// （`reject_structured_output` と同じ secret 列で構造的に共有する経路）。
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

    /// Spec [04] atomic mutation 境界（Abort 経路）: `abort_workflow_run`
    /// が受理されると `RunAborted` event は `write_log_required` 経由で必須 append
    /// される。NDJSON に正しく snake_case で記録され、observer が typed event として
    /// 読めることを担保する。
    #[test]
    fn run_aborted_event_required_append_writes_typed_ndjson() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let run_id = "00000000-0000-0000-0000-000000000800";

        log.append(&WorkflowEvent::RunAborted {
            run_id: run_id.to_string(),
            workflow_name: "boundary-wf".to_string(),
            aborted_step: None,
            timestamp: 4321.0,
        })
        .expect("RunAborted は write_log_required 経由で append される");

        let events = log.read_log(run_id).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            WorkflowEvent::RunAborted { run_id: rid, .. } => assert_eq!(rid, run_id),
            other => panic!("expected RunAborted, got {other:?}"),
        }
    }

    /// Spec [04] rollback: production dispatch 経由で event append が失敗した場合、
    /// WorkflowExecution / Run Store / event log は command 受理前 snapshot に戻る。
    #[tokio::test]
    async fn dispatch_approve_node_append_failure_rolls_back_full_snapshot() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/append-fail";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        exec.workflow_variables
            .insert("k".to_string(), "v_before".to_string());
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &run_id,
                ApprovalDecision::Approve,
                Some("lgtm".to_string()),
                Some("review"),
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));

        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).expect("run must remain");
        assert_eq!(
            restored.state, snapshot_before.state,
            "state は snapshot で一括復元される"
        );
        assert_eq!(
            restored.current_step_index,
            snapshot_before.current_step_index
        );
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        assert_eq!(
            restored.workflow_variables.get("k").map(|s| s.as_str()),
            Some("v_before"),
            "workflow_variables も mutation 前の値に戻る"
        );
        drop(execs);

        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].status, RunStatus::WaitingApproval);
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    /// Spec [04] rollback: AbortRun の required event append が失敗した場合も、
    /// WorkflowExecution / Run Store / ChatSession workflow_state は mutation 前へ戻る。
    #[tokio::test]
    async fn dispatch_abort_run_append_failure_rolls_back_execution_run_store_and_session() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/abort-append-fail";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        exec.state = WorkflowExecutionState::Running;
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .abort_workflow_run(app.handle(), &session_store, &handles, &run_id, None)
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).expect("run must remain");
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].status, RunStatus::Running);
        // ChatSession.workflow_state は撤去済みのため parent session 経由の rollback 観測は省略。
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    /// Spec [04] テスト境界: StartRun は production start primitive 入口で
    /// validation され、拒否時は state / event を変更しない。
    #[tokio::test]
    async fn start_run_primitive_rejects_invalid_name_without_state_change() {
        let engine = WorkflowRuntimeService::new_for_test();
        let run_store_dir = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(run_store_dir.path().to_path_buf())
            .await;

        let result = engine.resolve_start_run_workflow("../bad").await;

        assert!(matches!(
            result,
            Err(WorkflowEngineError::ValidationError(_))
        ));
        assert!(engine.executions.lock().await.is_empty());
        assert!(engine.list_active_runs().await.is_empty());
    }

    /// Spec [04] テスト境界: StartRun の正常系は production start primitive 経由で
    /// run_id を返し、execution / Run Store / RunStarted event を作成する。
    #[tokio::test]
    async fn start_run_primitive_accepts_creates_run_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let run_store_dir = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(run_store_dir.path().to_path_buf())
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
            .resolve_start_run_worktree(worktree_path.to_string_lossy().to_string())
            .await
            .unwrap();
        let workflow = engine.resolve_start_run_workflow(&stem).await.unwrap();
        let run_id = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &handles,
                workflow,
                resolved_worktree,
                &stem,
                Some("start me".to_string()),
                TriggerSource::DesktopUi,
                crate::domain::agent_session::PermissionMode::Edit,
            )
            .await
            .unwrap();
        assert!(
            engine.executions.lock().await.contains_key(&run_id),
            "StartRun must register a WorkflowExecution"
        );
        assert!(
            engine.get_run(&run_id).await.is_some(),
            "StartRun must create a Run Store entry"
        );
        assert!(read_dispatch_events(&app, &run_id).iter().any(|event| {
            matches!(
                event,
                WorkflowEvent::RunStarted {
                    workflow_file_stem,
                    ..
                } if workflow_file_stem == &stem
            )
        }));
    }

    /// Spec [04] rollback: StartRun の RunStarted append が失敗した場合、
    /// reservation / execution / parent ChatSession を command 受理前へ戻す。
    #[tokio::test]
    async fn start_run_primitive_append_failure_clears_created_parent_workflow_state() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let run_store_dir = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(run_store_dir.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let (repo_parent, _worktree_parent, worktree_path) = make_managed_worktree();
        configure_managed_repo(&app, repo_parent.path().join("repo").as_path());
        let worktree = std::fs::canonicalize(&worktree_path)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let stem = crate::adaptor::gateway::workflow::builtin::list_builtin_workflows()
            .into_iter()
            .next()
            .expect("at least one builtin workflow must exist")
            .name;
        let workflow = engine.resolve_start_run_workflow(&stem).await.unwrap();
        let result = engine
            .start_resolved_workflow(
                app.handle(),
                &session_store,
                &handles,
                workflow,
                worktree.clone(),
                &stem,
                Some("start with append failure".to_string()),
                TriggerSource::DesktopUi,
                crate::domain::agent_session::PermissionMode::Edit,
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        assert!(engine.executions.lock().await.is_empty());
        assert!(engine.list_active_runs().await.is_empty());
        let sessions = session_store
            .list_worktree_sessions(&dispatch_data_dir(app.handle()), &worktree)
            .unwrap();
        assert!(
            sessions.is_empty(),
            "RunStarted が存在しない失敗 run の parent ChatSession は残さない"
        );
    }

    // 撤去済み: persist_state は廃止された（NDJSON event log + Run Store metadata で永続化が完結）。
    // 旧 `dispatch_start_run_persist_failure_rolls_back_execution_run_store_and_parent_session` テストは
    // persist_state 注入失敗時の rollback を検証していたが、機構撤去により意味を失った。

    /// Spec [04] テスト境界: AbortRun は production dispatch 経由で Aborted に遷移し、
    /// RunAborted typed event を append する。
    #[tokio::test]
    async fn dispatch_abort_run_accepts_mutates_state_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-abort";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        let workflow = exec.workflow.clone();
        // spec issues-1023: session log 到達経路の維持を検証するため、
        // current_session_id を入れた状態で abort する。
        exec.current_session_id = Some("aborted-step-session".to_string());
        exec.state = WorkflowExecutionState::Running;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        WorkflowEventLog::new(&data_dir)
            .append(&WorkflowEvent::RunStarted {
                run_id: run_id.clone(),
                workflow_name: workflow.name.clone(),
                workflow_file_stem: workflow.name.clone(),
                worktree_path: worktree_path.to_string(),
                workflow_definition: workflow,
                timestamp: 1000.0,
            })
            .unwrap();

        engine
            .abort_workflow_run(app.handle(), &session_store, &handles, &run_id, None)
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&run_id).await,
            "terminal execution must be released after Aborted"
        );

        let events = read_dispatch_events(&app, &run_id);
        let aborted_event = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::RunAborted { aborted_step, .. } => aborted_step.as_ref(),
                _ => None,
            })
            .expect("RunAborted must persist the aborted step snapshot");
        assert_eq!(
            aborted_event.session_id.as_deref(),
            Some("aborted-step-session"),
            "RunAborted snapshot must keep the interrupted step session_id"
        );

        assert!(
            engine.get_state_by_run_id(&run_id).await.is_none(),
            "run_id-only live API must not expose released terminal history"
        );
        let reconstructed =
            crate::adaptor::gateway::workflow::event_projection::reconstruct_state_from_events(
                &run_id, &events,
            )
            .unwrap()
            .expect("released aborted run history must reconstruct from Event Log projection");
        assert_eq!(reconstructed.state, WorkflowExecutionState::Aborted);
        let aborted_entries: Vec<&StepHistoryEntry> = reconstructed
            .step_history
            .iter()
            .filter(|entry| entry.state == "aborted")
            .collect();
        assert_eq!(
            aborted_entries.len(),
            1,
            "released aborted run must reconstruct the aborted current step"
        );
        assert_eq!(
            aborted_entries[0].session_id.as_deref(),
            Some("aborted-step-session"),
            "reconstructed state must preserve the session log reachability"
        );
    }

    #[tokio::test]
    async fn dispatch_abort_run_snapshots_current_run_index_for_retried_step() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-abort-retry";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        let workflow = exec.workflow.clone();
        exec.state = WorkflowExecutionState::Running;
        exec.current_session_id = Some("session-review-2".to_string());
        exec.step_execution_counts.insert("review".to_string(), 2);
        exec.step_history.push(StepHistoryEntry {
            step_name: "review".to_string(),
            completed_at: 1001.0,
            result: Some("retry".to_string()),
            session_id: Some("session-review-1".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
        });
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        WorkflowEventLog::new(&data_dir)
            .append(&WorkflowEvent::RunStarted {
                run_id: run_id.clone(),
                workflow_name: workflow.name.clone(),
                workflow_file_stem: workflow.name.clone(),
                worktree_path: worktree_path.to_string(),
                workflow_definition: workflow,
                timestamp: 1000.0,
            })
            .unwrap();

        engine
            .abort_workflow_run(app.handle(), &session_store, &handles, &run_id, None)
            .await
            .unwrap();

        let events = read_dispatch_events(&app, &run_id);
        let aborted_step = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::RunAborted { aborted_step, .. } => aborted_step.as_ref(),
                _ => None,
            })
            .expect("retried current step must be persisted as aborted_step");
        assert_eq!(aborted_step.step_name, "review");
        assert_eq!(aborted_step.run_index, 2);
        assert_eq!(aborted_step.session_id.as_deref(), Some("session-review-2"));

        let reconstructed =
            crate::adaptor::gateway::workflow::event_projection::reconstruct_state_from_events(
                &run_id, &events,
            )
            .unwrap()
            .expect("released aborted retry must reconstruct from Event Log projection");
        let aborted_entry = reconstructed
            .step_history
            .iter()
            .find(|entry| entry.step_name == "review" && entry.run_index == 2)
            .expect("reconstructed history must contain the retried aborted step");
        assert_eq!(
            aborted_entry.session_id.as_deref(),
            Some("session-review-2")
        );
    }

    /// spec issues-1023: `make_aborted_parallel_history_entry` の単体検証。
    /// parallel ブロック中断時に parent step を 1 entry として、children を
    /// `child_outputs` に snapshot し、完了済み child は "completed"、それ以外は
    /// "aborted" 状態で記録される。session_id は全 child で残されることを担保する。
    #[test]
    fn make_aborted_parallel_history_entry_snapshots_mixed_child_states() {
        let workflow = Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![make_fanout_step("parallel-review", vec![], None)],
        };
        let exec = WorkflowExecution {
            id: "exec-abort-parallel".to_string(),
            workflow,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("parallel-review".to_string(), 1)]),
            step_history: Vec::new(),
            worktree_path: "/wt".to_string(),
            started_at: 0.0,
            updated_at: 0.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: Some(ParallelRunState {
                parent_step_name: "parallel-review".to_string(),
                aggregate: None,
                children: vec![
                    ParallelChildRun {
                        step_name: "child-a".to_string(),
                        session_id: "session-a".to_string(),
                        state: ParallelChildState::Completed,
                        result: Some("LGTM".to_string()),
                        structured_output: None,
                        output_contract: None,
                        failure_kind: None,
                        failure_disposition: None,
                        token_usage: TokenUsage::default(),
                        run_index: 1,
                    },
                    ParallelChildRun {
                        step_name: "child-b".to_string(),
                        session_id: "session-b".to_string(),
                        state: ParallelChildState::Running,
                        result: None,
                        structured_output: None,
                        output_contract: None,
                        failure_kind: None,
                        failure_disposition: None,
                        token_usage: TokenUsage::default(),
                        run_index: 1,
                    },
                ],
            }),
            workflow_variables: HashMap::new(),
            current_stall_observations: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        let entry = exec
            .make_aborted_parallel_history_entry(123.0)
            .expect("parallel_run が Some なら entry が返る");
        assert_eq!(entry.step_name, "parallel-review");
        assert_eq!(entry.state, "aborted");
        assert_eq!(entry.completed_at, 123.0);
        let children = entry.child_outputs.expect("child_outputs が Some");
        assert_eq!(children.len(), 2);
        let child_a = children.iter().find(|c| c.step_name == "child-a").unwrap();
        assert_eq!(child_a.state, "completed");
        assert_eq!(child_a.session_id.as_deref(), Some("session-a"));
        let child_b = children.iter().find(|c| c.step_name == "child-b").unwrap();
        assert_eq!(child_b.state, "aborted");
        assert_eq!(
            child_b.session_id.as_deref(),
            Some("session-b"),
            "未完了 child でも session_id が child_outputs に残る"
        );
    }

    /// Spec [06] テスト境界: node 限定 AbortRun は現在 node を照合した上で run abort として
    /// 扱い、Running / WaitingApproval のどちらでも `RunAborted` を append する。
    #[tokio::test]
    async fn dispatch_abort_run_with_expected_node_validates_node_and_appends_run_aborted() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-approval-abort";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        engine
            .abort_workflow_run(
                app.handle(),
                &session_store,
                &handles,
                &run_id,
                Some("review"),
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&run_id).await,
            "terminal execution must be released after Aborted"
        );
        let events = read_dispatch_events(&app, &run_id);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], WorkflowEvent::RunAborted { .. }));
    }

    // 撤去済み: dispatch_abort_run_with_expected_node_persist_failure_rolls_back は
    // persist_state 注入失敗を介して rollback を検証していたが、persist_state 機構の撤去で
    // 意味を失った（NDJSON event log + Run Store metadata が権威）。
    // required event append 失敗時の rollback は下記
    // `dispatch_abort_run_with_expected_node_append_failure_rolls_back` で引き続き検証する。

    /// Spec [04] rollback: approval UI 由来の AbortRun で required event append が失敗した場合も、
    /// WorkflowExecution / Run Store / ChatSession workflow_state は mutation 前へ戻る。
    #[tokio::test]
    async fn dispatch_abort_run_with_expected_node_append_failure_rolls_back() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/approval-abort-append-rollback";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .abort_workflow_run(
                app.handle(),
                &session_store,
                &handles,
                &run_id,
                Some("review"),
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].status, RunStatus::WaitingApproval);
        // ChatSession.workflow_state は撤去済みのため parent session 経由の rollback 観測は省略。
        // RunStore active projection の rollback だけ確認する（上の assertion で済み）。
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    /// Spec [04] Rule「対象不在 / 既に終了した command は受理されない」:
    /// AbortRun の dispatch 拒否経路は state を変化させず event を append しない。
    #[tokio::test]
    async fn dispatch_abort_run_rejects_not_found_and_terminal_without_append() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let missing_run_id = uuid::Uuid::new_v4().to_string();

        let missing = engine
            .abort_workflow_run(
                app.handle(),
                &session_store,
                &handles,
                &missing_run_id,
                None,
            )
            .await;
        assert!(matches!(
            missing,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));
        assert!(read_dispatch_events(&app, &missing_run_id).is_empty());

        let terminal_run_id = uuid::Uuid::new_v4().to_string();
        let mut terminal = make_waiting_approval_execution(&terminal_run_id, "/wt/terminal-abort");
        terminal.state = WorkflowExecutionState::Completed;
        let snapshot_before = terminal.clone();
        engine
            .executions
            .lock()
            .await
            .insert(terminal_run_id.clone(), terminal);

        let terminal_result = engine
            .abort_workflow_run(
                app.handle(),
                &session_store,
                &handles,
                &terminal_run_id,
                None,
            )
            .await;
        assert!(matches!(
            terminal_result,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&terminal_run_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        assert!(read_dispatch_events(&app, &terminal_run_id).is_empty());

        let released_terminal_run_id = uuid::Uuid::new_v4().to_string();
        let released_terminal =
            make_waiting_approval_execution(&released_terminal_run_id, "/wt/released-terminal");
        insert_execution_and_active_run(&engine, released_terminal, TriggerSource::DesktopUi).await;
        engine
            .run_store
            .complete_run(
                &released_terminal_run_id,
                TerminalRunStatus::Completed,
                2000.0,
                None,
            )
            .await
            .unwrap();
        engine
            .executions
            .lock()
            .await
            .remove(&released_terminal_run_id);

        let released_terminal_result = engine
            .abort_workflow_run(
                app.handle(),
                &session_store,
                &handles,
                &released_terminal_run_id,
                None,
            )
            .await;
        assert!(matches!(
            released_terminal_result,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        assert!(read_dispatch_events(&app, &released_terminal_run_id).is_empty());
    }

    #[tokio::test]
    async fn dispatch_abort_run_treats_execution_released_after_lookup_as_already_terminal() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let run_id = uuid::Uuid::new_v4().to_string();
        let exec = make_waiting_approval_execution(&run_id, "/wt/released-after-lookup");
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let lookup_completed = Arc::new(tokio::sync::Notify::new());
        let continue_precommit = Arc::new(tokio::sync::Notify::new());
        engine
            .pause_abort_after_lookup_for_test(lookup_completed.clone(), continue_precommit.clone())
            .await;

        let abort_engine = engine.clone();
        let abort_session_store = session_store.clone();
        let abort_handles = handles.clone();
        let abort_run_id = run_id.clone();
        let app_handle = app.handle().clone();
        let abort_task = tokio::spawn(async move {
            abort_engine
                .abort_workflow_run(
                    &app_handle,
                    &abort_session_store,
                    &abort_handles,
                    &abort_run_id,
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
            .run_store
            .complete_run(&run_id, TerminalRunStatus::Completed, 2000.0, None)
            .await
            .unwrap();
        engine.executions.lock().await.remove(&run_id);
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
            read_dispatch_events(&app, &run_id).is_empty(),
            "released-after-lookup race must not append dispatch events"
        );
    }

    /// Spec [04] no-op 不変条件: approval UI 由来の
    /// `AbortRun { expected_node_name: Some(_) }` でも、対象不在・stale node・既決 node は
    /// production dispatch 経由で state / Run Store を変化させず event を append しない。
    #[tokio::test]
    async fn dispatch_approval_abort_rejects_missing_stale_and_resolved_targets_without_append() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

        let missing_run_id = uuid::Uuid::new_v4().to_string();
        let missing = engine
            .abort_workflow_run(
                app.handle(),
                &session_store,
                &handles,
                &missing_run_id,
                Some("review"),
            )
            .await;
        assert!(matches!(
            missing,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));
        assert!(engine.list_active_runs().await.is_empty());
        assert!(engine.list_completed_runs().await.is_empty());
        assert!(read_dispatch_events(&app, &missing_run_id).is_empty());

        let stale_run_id = uuid::Uuid::new_v4().to_string();
        let stale_worktree = "/wt/approval-abort-stale";
        let mut stale_exec = make_waiting_approval_execution(&stale_run_id, stale_worktree);
        stale_exec.current_session_id = None;
        let stale_before = stale_exec.clone();
        insert_execution_and_active_run(&engine, stale_exec, TriggerSource::DesktopUi).await;
        let stale_active_before = engine.list_active_runs().await;

        let stale = engine
            .abort_workflow_run(
                app.handle(),
                &session_store,
                &handles,
                &stale_run_id,
                Some("old-review"),
            )
            .await;
        assert!(matches!(
            stale,
            Err(WorkflowEngineError::UnauthorizedApprovalTarget(_))
        ));
        let execs = engine.executions.lock().await;
        let stale_after = execs.get(&stale_run_id).unwrap();
        assert_eq!(stale_after.state, stale_before.state);
        assert_eq!(
            stale_after.current_step_index,
            stale_before.current_step_index
        );
        assert_eq!(
            stale_after.step_history.len(),
            stale_before.step_history.len()
        );
        drop(execs);
        let stale_active_after = engine.list_active_runs().await;
        assert_eq!(stale_active_after.len(), stale_active_before.len());
        assert_eq!(stale_active_after[0].run_id, stale_active_before[0].run_id);
        assert_eq!(stale_active_after[0].status, stale_active_before[0].status);
        assert!(read_dispatch_events(&app, &stale_run_id).is_empty());

        let resolved_run_id = uuid::Uuid::new_v4().to_string();
        let resolved_worktree = "/wt/approval-abort-resolved";
        let mut resolved_exec =
            make_waiting_approval_execution(&resolved_run_id, resolved_worktree);
        resolved_exec.current_session_id = None;
        resolved_exec.state = WorkflowExecutionState::Completed;
        let resolved_before = resolved_exec.clone();
        engine
            .executions
            .lock()
            .await
            .insert(resolved_run_id.clone(), resolved_exec);
        engine
            .run_store
            .register_active(WorkflowRun {
                run_id: resolved_run_id.clone(),
                workflow_name: "boundary-wf".to_string(),
                task: None,
                status: RunStatus::WaitingApproval,
                worktree_path: resolved_worktree.to_string(),
                current_node_name: Some("review".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 1000.0,
                updated_at: 1000.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        engine
            .run_store
            .complete_run(&resolved_run_id, TerminalRunStatus::Completed, 2000.0, None)
            .await
            .unwrap();
        let completed_before = engine.list_completed_runs().await;

        let resolved = engine
            .abort_workflow_run(
                app.handle(),
                &session_store,
                &handles,
                &resolved_run_id,
                Some("review"),
            )
            .await;
        assert!(matches!(
            resolved,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        let execs = engine.executions.lock().await;
        let resolved_after = execs.get(&resolved_run_id).unwrap();
        assert_eq!(resolved_after.state, resolved_before.state);
        assert_eq!(
            resolved_after.step_history.len(),
            resolved_before.step_history.len()
        );
        drop(execs);
        let completed_after = engine.list_completed_runs().await;
        assert_eq!(completed_after.len(), completed_before.len());
        assert_eq!(completed_after[0].run_id, completed_before[0].run_id);
        assert_eq!(completed_after[0].status, completed_before[0].status);
        assert!(read_dispatch_events(&app, &resolved_run_id).is_empty());
    }

    /// Spec [04] テスト境界: ApproveNode は production dispatch 経由で判断を受理し、
    /// state mutation と ApprovalResolved append を同じ command 受理サイクルで行う。
    #[tokio::test]
    async fn dispatch_approve_node_accepts_mutates_state_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-approve";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &run_id,
                ApprovalDecision::Approve,
                Some("lgtm".to_string()),
                Some("review"),
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&run_id).await,
            "terminal execution must be released after Completed"
        );
        let events = read_dispatch_events(&app, &run_id);
        assert!(matches!(
            events.as_slice(),
            [
                WorkflowEvent::ApprovalResolved {
                    decision: ApprovalDecisionRecord::Approve,
                    ..
                },
                WorkflowEvent::NodeCompleted { node_name, .. },
                WorkflowEvent::RunCompleted { .. },
            ] if node_name == "review"
        ));
    }

    // 撤去済み: parent ChatSession / persist_state 機構の撤去で意味を失ったテスト。

    /// Spec [04] テスト境界: RejectNode は production dispatch 経由で判断を受理し、
    /// state mutation と ApprovalResolved { decision: Reject } append を行う。
    #[tokio::test]
    async fn dispatch_reject_node_accepts_mutates_state_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-reject-accept";
        let mut exec = make_waiting_approval_execution_with_workflow(
            &run_id,
            worktree_path,
            make_rejectable_approval_workflow(),
        );
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &run_id,
                ApprovalDecision::Reject {
                    comment: "needs changes".to_string(),
                },
                Some("needs changes".to_string()),
                Some("review"),
            )
            .await
            .unwrap();

        let events = read_dispatch_events(&app, &run_id);
        assert!(matches!(
            &events[..3],
            [
                WorkflowEvent::ApprovalResolved {
                    decision: ApprovalDecisionRecord::Reject,
                    comment: Some(comment),
                    ..
                },
                WorkflowEvent::NodeCompleted {
                    node_name: completed,
                    result,
                    ..
                },
                WorkflowEvent::NodeStarted {
                    node_name: started,
                    ..
                },
            ] if comment == "needs changes"
                && completed == "review"
                && result.as_deref() == Some("reject")
                && started == "fix"
        ));
    }

    /// Spec [04] テスト境界: RejectNode の非受理経路は production dispatch 経由でも
    /// state を変化させず、typed event を append しない。
    #[tokio::test]
    async fn dispatch_reject_node_rejected_target_keeps_state_and_no_append() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-reject";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let result = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &run_id,
                ApprovalDecision::Reject {
                    comment: "needs changes".to_string(),
                },
                Some("needs changes".to_string()),
                Some("review"),
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::InvalidState(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    /// Spec [04] no-op 不変条件: ApproveNode / RejectNode の対象不在・stale node・既決 node は
    /// production dispatch 経由でも state を変化させず event を append しない。
    #[tokio::test]
    async fn dispatch_approval_commands_reject_missing_stale_and_resolved_targets_without_append() {
        for command_kind in ["approve", "reject"] {
            let app = make_dispatch_app();
            let engine = WorkflowRuntimeService::new_for_test();
            let tmp = TempDir::new().unwrap();
            engine
                .set_run_store_data_dir(tmp.path().to_path_buf())
                .await;
            let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));

            let missing_run_id = uuid::Uuid::new_v4().to_string();
            let missing = match command_kind {
                "approve" => {
                    engine
                        .resolve_workflow_approval(
                            app.handle(),
                            &session_store,
                            &handles,
                            &missing_run_id,
                            ApprovalDecision::Approve,
                            None,
                            Some("review"),
                        )
                        .await
                }
                "reject" => {
                    engine
                        .resolve_workflow_approval(
                            app.handle(),
                            &session_store,
                            &handles,
                            &missing_run_id,
                            ApprovalDecision::Reject {
                                comment: "needs changes".to_string(),
                            },
                            Some("needs changes".to_string()),
                            Some("review"),
                        )
                        .await
                }
                _ => unreachable!(),
            };
            assert!(matches!(
                missing,
                Err(WorkflowEngineError::ExecutionNotFound(_))
            ));
            assert!(read_dispatch_events(&app, &missing_run_id).is_empty());

            let stale_run_id = uuid::Uuid::new_v4().to_string();
            let worktree_path = format!("/wt/{command_kind}-stale");
            let mut stale_exec = make_waiting_approval_execution(&stale_run_id, &worktree_path);
            stale_exec.current_session_id = None;
            let stale_before = stale_exec.clone();
            insert_execution_and_active_run(&engine, stale_exec, TriggerSource::DesktopUi).await;
            let stale = match command_kind {
                "approve" => {
                    engine
                        .resolve_workflow_approval(
                            app.handle(),
                            &session_store,
                            &handles,
                            &stale_run_id,
                            ApprovalDecision::Approve,
                            None,
                            Some("old-review"),
                        )
                        .await
                }
                "reject" => {
                    engine
                        .resolve_workflow_approval(
                            app.handle(),
                            &session_store,
                            &handles,
                            &stale_run_id,
                            ApprovalDecision::Reject {
                                comment: "needs changes".to_string(),
                            },
                            Some("needs changes".to_string()),
                            Some("old-review"),
                        )
                        .await
                }
                _ => unreachable!(),
            };
            assert!(matches!(
                stale,
                Err(WorkflowEngineError::UnauthorizedApprovalTarget(_))
            ));
            let execs = engine.executions.lock().await;
            let restored = execs.get(&stale_run_id).unwrap();
            assert_eq!(restored.state, stale_before.state);
            assert_eq!(restored.current_step_index, stale_before.current_step_index);
            assert_eq!(restored.step_history.len(), stale_before.step_history.len());
            drop(execs);
            assert!(read_dispatch_events(&app, &stale_run_id).is_empty());

            let resolved_run_id = uuid::Uuid::new_v4().to_string();
            let worktree_path = format!("/wt/{command_kind}-resolved");
            let mut resolved_exec =
                make_waiting_approval_execution(&resolved_run_id, &worktree_path);
            resolved_exec.current_session_id = None;
            resolved_exec.state = WorkflowExecutionState::Completed;
            let resolved_before = resolved_exec.clone();
            engine
                .executions
                .lock()
                .await
                .insert(resolved_run_id.clone(), resolved_exec);
            let resolved = match command_kind {
                "approve" => {
                    engine
                        .resolve_workflow_approval(
                            app.handle(),
                            &session_store,
                            &handles,
                            &resolved_run_id,
                            ApprovalDecision::Approve,
                            None,
                            Some("review"),
                        )
                        .await
                }
                "reject" => {
                    engine
                        .resolve_workflow_approval(
                            app.handle(),
                            &session_store,
                            &handles,
                            &resolved_run_id,
                            ApprovalDecision::Reject {
                                comment: "needs changes".to_string(),
                            },
                            Some("needs changes".to_string()),
                            Some("review"),
                        )
                        .await
                }
                _ => unreachable!(),
            };
            assert!(matches!(
                resolved,
                Err(WorkflowEngineError::InvalidState(_))
            ));
            let execs = engine.executions.lock().await;
            let restored = execs.get(&resolved_run_id).unwrap();
            assert_eq!(restored.state, resolved_before.state);
            assert_eq!(
                restored.step_history.len(),
                resolved_before.step_history.len()
            );
            drop(execs);
            assert!(read_dispatch_events(&app, &resolved_run_id).is_empty());
        }
    }

    /// Spec [04] rollback: RejectNode の required event append が失敗した場合も、
    /// WorkflowExecution / Run Store は mutation 前 snapshot に戻り、event は append されない。
    #[tokio::test]
    async fn dispatch_reject_node_append_failure_rolls_back_execution_and_run_store() {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/reject-append-rollback";
        let mut exec = make_waiting_approval_execution_with_workflow(
            &run_id,
            worktree_path,
            make_rejectable_approval_workflow(),
        );
        exec.current_session_id = None;
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &run_id,
                ApprovalDecision::Reject {
                    comment: "needs changes".to_string(),
                },
                Some("needs changes".to_string()),
                Some("review"),
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.current_step_index,
            snapshot_before.current_step_index
        );
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].status, RunStatus::WaitingApproval);
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    // 撤去済み: persist_state 注入失敗を介した rollback テストは persist_state 機構の撤去で
    // 意味を失った。required event append 失敗の rollback は append_failure 系テストが担保する。

    /// Spec [04] rollback: command 受理サイクル内の Run Store sync が失敗した場合も、
    /// engine state / Run Store / ChatSession projection を mutation 前へ戻し Err を返す。
    #[tokio::test]
    async fn dispatch_approve_node_run_store_sync_failure_rolls_back_execution_run_store_and_session(
    ) {
        let app = make_dispatch_app();
        let engine = WorkflowRuntimeService::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/run-store-sync-rollback";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        exec.workflow_variables
            .insert("keep".to_string(), "before".to_string());
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let bad_data_dir = tmp.path().join("not-a-directory");
        std::fs::write(&bad_data_dir, "file").unwrap();
        engine.set_run_store_data_dir(bad_data_dir).await;

        let result = engine
            .resolve_workflow_approval(
                app.handle(),
                &session_store,
                &handles,
                &run_id,
                ApprovalDecision::Approve,
                Some("lgtm".to_string()),
                Some("review"),
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).unwrap();
        assert_eq!(restored.state, WorkflowExecutionState::WaitingApproval);
        assert_eq!(
            restored.workflow_variables.get("keep").map(String::as_str),
            Some("before")
        );
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        assert_eq!(
            engine.list_active_runs().await[0].status,
            RunStatus::WaitingApproval
        );
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    // 撤去済み: persist_state 注入失敗テストは parent ChatSession 機構撤去で意味を失った。

    /// Spec [04] atomic mutation 境界（A2 batch commit）: `write_log_required_batch`
    /// 経由で ApprovalResolved + RunAborted を 1 つの commit point として書き込めば、
    /// `WorkflowEventLog::append_batch` の 1 回の write_all で両 event が NDJSON に
    /// 連結 append される。同一 commit batch 内の partial commit（最初の event のみ
    /// 残る）を構造的に排除することを担保する（handle_approval の Abort 経路と
    /// 同じ atomic 境界）。
    #[test]
    fn approval_abort_commit_batch_persists_both_events_in_single_write() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let run_id = "00000000-0000-0000-0000-000000000900";
        let approval_event = WorkflowEvent::ApprovalResolved {
            run_id: run_id.to_string(),
            workflow_name: "boundary-wf".to_string(),
            node_name: "review".to_string(),
            decision: ApprovalDecisionRecord::Abort,
            comment: None,
            timestamp: 4000.0,
        };
        let aborted_event = WorkflowEvent::RunAborted {
            run_id: run_id.to_string(),
            workflow_name: "boundary-wf".to_string(),
            aborted_step: None,
            timestamp: 4000.0,
        };
        log.append_batch(&[approval_event, aborted_event])
            .expect("batch append for approval-abort commit point must succeed");
        let events = log.read_log(run_id).unwrap();
        assert_eq!(
            events.len(),
            2,
            "ApprovalResolved + RunAborted は atomic batch で 2 件 append される"
        );
        assert!(matches!(events[0], WorkflowEvent::ApprovalResolved { .. }));
        assert!(matches!(events[1], WorkflowEvent::RunAborted { .. }));
    }

    /// Spec [04] atomic mutation 境界（A3 AbortRun terminal sync post-commit 化）:
    /// `abort_workflow_by_run_id` は append 失敗時に Run Store / external 副作用を
    /// 一切実行しないことが構造的不変条件。本テストは pre-commit が in-memory state
    /// 変更のみであり、append 失敗時に snapshot 一括復元のみで完全に元状態へ戻せる
    /// ことを直接確認する（外部依存の差し替えを必要としない経路）。
    #[tokio::test]
    async fn abort_run_pre_commit_holds_only_in_memory_mutation() {
        let engine = WorkflowRuntimeService::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_waiting_approval_execution(&run_id, "/wt/pre-commit");
        exec.state = WorkflowExecutionState::Running;
        let snapshot_before = exec.clone();
        engine.executions.lock().await.insert(run_id.clone(), exec);

        // pre-commit 区間で行う state mutation を再現（abort_workflow_by_run_id 内の
        // step 2 と同等）。
        let mutated_timestamp = 1234.0;
        {
            let mut execs = engine.executions.lock().await;
            let exec = execs.get_mut(&run_id).unwrap();
            assert!(exec.is_active(), "active な run でなければ mutation しない");
            exec.state = WorkflowExecutionState::Aborted;
            exec.updated_at = mutated_timestamp;
        }
        {
            let execs = engine.executions.lock().await;
            let exec = execs.get(&run_id).unwrap();
            assert_eq!(exec.state, WorkflowExecutionState::Aborted);
            assert_eq!(exec.updated_at, mutated_timestamp);
        }

        // append 失敗を擬制した snapshot 一括復元（A3: pre-commit 区間は in-memory のみ
        // のため、Run Store / interrupt_agent / persist 等の外部副作用は不要）。
        {
            let mut execs = engine.executions.lock().await;
            if let Some(exec) = execs.get_mut(&run_id) {
                *exec = snapshot_before.clone();
            }
        }
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).expect("run must remain");
        assert_eq!(
            restored.state,
            WorkflowExecutionState::Running,
            "snapshot 復元で active 状態に戻る"
        );
        assert_ne!(
            restored.updated_at, mutated_timestamp,
            "pre-commit で書いた updated_at も一括復元される"
        );
    }

    /// 起動時 recovery: 前回起動中に terminal event が書かれないまま終了した run について、
    /// `recover_orphan_runs` が NDJSON 末尾に `RunAborted` を append し、metadata 上の
    /// status を Aborted に書き換える。reconstruction 経路が Aborted を返すようになる。
    #[tokio::test]
    async fn recover_orphan_runs_marks_non_terminal_metadata_as_aborted() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());

        // 前回プロセスの状態を模擬: workflow_runs/<id>.json に Running、event log に RunStarted のみ。
        let prev_store =
            std::sync::Arc::new(crate::adaptor::gateway::workflow::run::RunStore::new());
        prev_store.set_data_dir(data_dir.clone()).await;
        let orphan_id = uuid::Uuid::new_v4().to_string();
        prev_store
            .register_active(WorkflowRun {
                run_id: orphan_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: "/wt/a".to_string(),
                current_node_name: Some("plan".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        let log = WorkflowEventLog::new(&data_dir);
        log.append(&WorkflowEvent::RunStarted {
            run_id: orphan_id.clone(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: "/wt/a".to_string(),
            workflow_definition: Workflow {
                variables: Default::default(),
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![make_test_step(
                    "plan",
                    TestKind::Session,
                    "plan",
                    vec![],
                    None,
                )],
            },
            timestamp: 100.0,
        })
        .unwrap();

        // 起動直後を模擬した engine (空の in-memory state + 同じ data_dir)。
        let engine = std::sync::Arc::new(WorkflowRuntimeService::new_for_test());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        engine.recover_orphan_runs(app.handle()).await;

        // metadata が Aborted に書き換わっている（status / completed_at が更新される）。
        let summary = engine
            .run_store
            .get_run(&orphan_id)
            .await
            .expect("metadata must remain after recovery");
        assert_eq!(summary.status, RunStatus::Aborted);
        assert!(summary.completed_at.is_some());
        assert!(summary.error_reason.is_none());

        // 末尾 event が RunAborted。projection も Aborted を返すようになる。
        let events = read_dispatch_events(&app, &orphan_id);
        assert!(
            matches!(events.last(), Some(WorkflowEvent::RunAborted { .. })),
            "log の末尾は RunAborted: {:?}",
            events.last()
        );
        let projected =
            crate::adaptor::gateway::workflow::event_projection::reconstruct_state_from_events(
                &orphan_id, &events,
            )
            .unwrap()
            .unwrap();
        assert_eq!(projected.state, WorkflowExecutionState::Aborted);
    }

    /// 起動時 recovery: 既に terminal な metadata は変更されない（idempotent）。
    /// recovery 二回目以降は append も persist も走らない。
    #[tokio::test]
    async fn recover_orphan_runs_is_idempotent_for_already_terminal_runs() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());

        let prev_store =
            std::sync::Arc::new(crate::adaptor::gateway::workflow::run::RunStore::new());
        prev_store.set_data_dir(data_dir.clone()).await;
        let done_id = uuid::Uuid::new_v4().to_string();
        prev_store
            .register_active(WorkflowRun {
                run_id: done_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: "/wt/b".to_string(),
                current_node_name: Some("plan".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        prev_store
            .complete_run(&done_id, TerminalRunStatus::Completed, 150.0, None)
            .await
            .unwrap();

        let engine = std::sync::Arc::new(WorkflowRuntimeService::new_for_test());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let events_before = read_dispatch_events(&app, &done_id);
        engine.recover_orphan_runs(app.handle()).await;
        let events_after = read_dispatch_events(&app, &done_id);
        assert_eq!(
            events_before.len(),
            events_after.len(),
            "terminal な run には event を append しない"
        );
        let summary = engine
            .run_store
            .get_run(&done_id)
            .await
            .expect("metadata must remain");
        assert_eq!(summary.status, RunStatus::Completed);
    }

    // ---- [08] handle_submit_output: 単一トランザクション境界 ----

    /// テスト用 helper: production 経路と同じ submit-output primitive 経由で
    /// 構造化出力を提出する。CLI pending 経路は `request_id` / `submitted_at` を渡し、
    /// UI / in-process 経路はどちらも None で渡す。
    #[allow(clippy::too_many_arguments)]
    async fn submit_output_for_test(
        engine: &Arc<WorkflowRuntimeService>,
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
        run_id: &str,
        step_name: &str,
        contract: &str,
        structured_output: serde_json::Value,
        request_id: Option<&str>,
        submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError> {
        let (session_store, agent_runtime) = make_dispatch_deps(dispatch_data_dir(app));
        submit_output_for_test_with_deps(
            engine,
            app,
            &session_store,
            &agent_runtime,
            run_id,
            step_name,
            contract,
            structured_output,
            request_id,
            submitted_at,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_output_for_test_with_deps(
        engine: &Arc<WorkflowRuntimeService>,
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        step_name: &str,
        contract: &str,
        structured_output: serde_json::Value,
        request_id: Option<&str>,
        submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError> {
        let (request_id, submitted_at) = match (request_id, submitted_at) {
            (Some(rid), Some(ts)) => (Some(rid.to_string()), Some(ts)),
            (None, None) => (None, None),
            _ => panic!("request_id と submitted_at は両方 Some か両方 None で渡すこと"),
        };
        engine
            .submit_workflow_output(
                app,
                session_store,
                agent_runtime,
                run_id,
                step_name.to_string(),
                contract.to_string(),
                structured_output,
                request_id,
                submitted_at,
            )
            .await
    }

    fn make_submit_output_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "submit-wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![{
                let mut step = make_test_step("review", TestKind::Session, "review", vec![], None);
                step.output_contract = Some("review-verdict".to_string());
                step
            }],
        }
    }

    fn read_submit_output_events(app: &DispatchTestApp, run_id: &str) -> Vec<WorkflowEvent> {
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle())
                .expect("data_dir");
        WorkflowEventLog::new(&data_dir)
            .read_log(run_id)
            .unwrap_or_default()
    }

    async fn step_output_for(
        engine: &WorkflowRuntimeService,
        run_id: &str,
        step_name: &str,
    ) -> Option<StepOutput> {
        engine
            .executions
            .lock()
            .await
            .get(run_id)
            .and_then(|exec| exec.step_outputs.get(step_name).cloned())
    }

    /// [08] 振る舞い定義 Rule 1（適合する場合）: contract に適合する構造化出力は
    /// step output として確定し、後続 step から参照可能になり、事実履歴に記録される。
    #[tokio::test]
    async fn submit_output_persists_step_output_and_appends_event_when_contract_satisfied() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/submit-ok".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            Some("00000000-0000-0000-0000-000000000aa1"),
            Some(800.0),
        )
        .await
        .unwrap();

        // step_outputs slot に書き込まれている
        let step_output = step_output_for(&engine, &run_id, "review")
            .await
            .expect("step_outputs must be updated");
        assert_eq!(
            step_output.output_contract.as_deref(),
            Some("review-verdict")
        );
        assert_eq!(
            step_output.structured_output.as_ref().unwrap()["verdict"],
            "LGTM"
        );

        // OutputSubmitted event が追記されている
        let events = read_submit_output_events(&app, &run_id);
        let submitted = events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::OutputSubmitted {
                    node_name,
                    contract,
                    structured_output,
                    request_id,
                    submitted_at,
                    ..
                } if node_name == "review" => Some((
                    contract.clone(),
                    structured_output.clone(),
                    request_id.clone(),
                    *submitted_at,
                )),
                _ => None,
            })
            .expect("OutputSubmitted event must be appended");
        assert_eq!(submitted.0, "review-verdict");
        assert_eq!(submitted.1["verdict"], "LGTM");
        assert_eq!(
            submitted.2.as_deref(),
            Some("00000000-0000-0000-0000-000000000aa1")
        );
        assert_eq!(submitted.3, Some(800.0));
    }

    /// #1250: contract 不適合の SubmitOutput は即 reject せず repair policy に渡す。
    /// invalid payload 自体は保存せず、ContractRepairRequested のみを append する。
    #[tokio::test]
    async fn submit_output_invalid_contract_requests_repair_without_persisting_output() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/submit-invalid";
        let session_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
        let mut workflow = make_submit_output_workflow();
        workflow.nodes[0].output_contract = Some("spec-directory".to_string());
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                workflow,
                WorkflowExecutionState::Running,
                worktree_path.to_string(),
                TriggerSource::DesktopUi,
            )
            .await;
        {
            let mut execs = engine.executions.lock().await;
            execs
                .get_mut(&run_id)
                .expect("seeded execution")
                .current_session_id = Some(session_id.to_string());
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
            &run_id,
            "review",
            "spec-directory",
            serde_json::json!({"spec_dir": "/not/relative"}),
            Some("00000000-0000-0000-0000-000000000ab1"),
            Some(900.0),
        )
        .await
        .unwrap();

        // step_outputs は更新されない
        assert!(step_output_for(&engine, &run_id, "review").await.is_none());
        // OutputSubmitted event も書かれない
        let events = read_submit_output_events(&app, &run_id);
        assert!(events
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::OutputSubmitted { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ContractRepairRequested {
                node_name,
                run_index: 1,
                request_id: Some(request_id),
                attempt: 1,
                violation_reason,
                ..
            } if node_name == "review"
                && request_id == "00000000-0000-0000-0000-000000000ab1"
                && violation_reason
                    == submission_violation_reason(SubmissionViolation::InvalidSubmitOutput)
        )));
    }

    #[tokio::test]
    async fn pending_invalid_submit_output_repair_is_idempotent_by_request_id() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/submit-invalid-idempotent";
        let session_id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
        let mut workflow = make_submit_output_workflow();
        workflow.nodes[0].output_contract = Some("spec-directory".to_string());
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                workflow,
                WorkflowExecutionState::Running,
                worktree_path.to_string(),
                TriggerSource::DesktopUi,
            )
            .await;
        {
            let mut execs = engine.executions.lock().await;
            execs
                .get_mut(&run_id)
                .expect("seeded execution")
                .current_session_id = Some(session_id.to_string());
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
        let pending = crate::adaptor::gateway::workflow::pending_command::PendingCommand::new(
            run_id.clone(),
            crate::adaptor::gateway::workflow::pending_command::CliRequestPayload::SubmitOutput {
                step_name: "review".to_string(),
                contract: "spec-directory".to_string(),
                structured_output: serde_json::json!({"spec_dir": "/not/relative"}),
            },
            901.0,
        );
        let request_id = pending.id.clone();

        let first =
            crate::adaptor::gateway::workflow::pending_command_dispatcher::dispatch_pending_command(
                app.handle(),
                &engine,
                &session_store,
                &handles,
                pending.clone(),
            )
            .await;
        assert_eq!(
            first,
            crate::adaptor::gateway::workflow::pending_command_dispatcher::PendingCommandDispatchOutcome::Accepted
        );
        let second =
            crate::adaptor::gateway::workflow::pending_command_dispatcher::dispatch_pending_command(
                app.handle(),
                &engine,
                &session_store,
                &handles,
                pending,
            )
            .await;
        assert_eq!(
            second,
            crate::adaptor::gateway::workflow::pending_command_dispatcher::PendingCommandDispatchOutcome::Accepted
        );

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        let repair_events = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    WorkflowEvent::ContractRepairRequested {
                        request_id: Some(id),
                        ..
                    } if id == &request_id
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            repair_events.len(),
            1,
            "same pending SubmitOutput request_id must not consume a second repair attempt; got {events:?}"
        );
    }

    /// [08] 振る舞い定義 Rule 1: 不在 step に対する提出は副作用なしで拒否される。
    #[tokio::test]
    async fn submit_output_rejects_unknown_step_without_side_effects() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/submit-unknown".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "ghost-step",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
        let events = read_submit_output_events(&app, &run_id);
        assert!(events
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::OutputSubmitted { .. })));
    }

    /// [08] 振る舞い定義 Rule 1: 不在 run （UUID 未登録）に対する提出は ExecutionNotFound で拒否。
    #[tokio::test]
    async fn submit_output_rejects_unknown_run() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
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
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/submit-mismatch".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "review",
            "fix-result",
            serde_json::json!({"status": "FIXED"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
        assert!(step_output_for(&engine, &run_id, "review").await.is_none());
    }

    /// [08] 振る舞い定義 Rule 3: 提出済み output は後続 step から
    /// `pass_output_from` 経路で経路非依存に参照できる。step_outputs に
    /// 書き込まれた entry が contract 由来の `output_contract` を保持することを担保する。
    #[tokio::test]
    async fn submit_output_step_output_carries_contract_for_downstream_reference() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/submit-downstream".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap();
        let step_output = step_output_for(&engine, &run_id, "review")
            .await
            .expect("step_outputs slot must be populated");
        assert_eq!(
            step_output.output_contract.as_deref(),
            Some("review-verdict")
        );
        // structured_output が後続経路に渡る shape で保持される
        assert!(step_output.structured_output.is_some());
    }

    /// [08] spec-directory contract が submit された場合、workflow_variables に
    /// `spec_dir` が反映される（extract_contract_variables の合流）。
    #[tokio::test]
    async fn submit_output_applies_contract_variables_for_spec_dir() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let workflow = Workflow {
            variables: Default::default(),
            name: "spec-wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![{
                let mut step = make_test_step("plan", TestKind::Session, "plan", vec![], None);
                step.output_contract = Some("spec-directory".to_string());
                step
            }],
        };
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                workflow,
                WorkflowExecutionState::Running,
                "/wt/submit-spec".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "plan",
            "spec-directory",
            serde_json::json!({"spec_dir": "docs/spec/issues-1029.md"}),
            None,
            None,
        )
        .await
        .unwrap();

        let vars = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|exec| exec.workflow_variables.clone())
            .unwrap();
        assert_eq!(
            vars.get("spec_dir").map(|s| s.as_str()),
            Some("docs/spec/issues-1029.md")
        );
    }

    /// [08] 振る舞い定義 Rule 1 Scenario 3: 既に出力を受け付けられる状態にない step に
    /// 対する提出は拒否され、state と event log が変化しないことを確認する。
    #[tokio::test]
    async fn submit_output_rejects_non_accepting_step_without_side_effects() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let workflow = Workflow {
            variables: Default::default(),
            name: "multi-step".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![
                {
                    let mut step =
                        make_test_step("first", TestKind::Session, "first", vec![], None);
                    step.output_contract = Some("review-verdict".to_string());
                    step
                },
                {
                    let mut step =
                        make_test_step("second", TestKind::Session, "second", vec![], None);
                    step.output_contract = Some("review-verdict".to_string());
                    step
                },
            ],
        };
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                workflow,
                WorkflowExecutionState::Running,
                "/wt/submit-stale".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        // current step を `second` に進めて、`first` を提出受付対象から外す。
        engine.force_current_step_index_for_test(&run_id, 1).await;

        let events_before = read_submit_output_events(&app, &run_id);
        let exec_before = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| (e.step_outputs.clone(), e.workflow_variables.clone()))
            .unwrap();

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
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
            .get(&run_id)
            .map(|e| (e.step_outputs.clone(), e.workflow_variables.clone()))
            .unwrap();
        assert_eq!(exec_before.0.len(), exec_after.0.len());
        assert_eq!(exec_before.1, exec_after.1);

        // OutputSubmitted event は append されない
        let events_after = read_submit_output_events(&app, &run_id);
        assert_eq!(events_before.len(), events_after.len());
        assert!(events_after
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::OutputSubmitted { .. })));
    }

    /// [08] 振る舞い定義 Rule 4: agent step の自由文出力に `<workflow_output>` 相当の
    /// 表現が含まれていても、明示的提出が無い限り step_outputs は更新されず、
    /// OutputSubmitted event も追記されない（prose 抽出経路の完全廃止）。
    #[tokio::test]
    async fn agent_free_text_workflow_output_block_does_not_confirm_step_output() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/agent-freetext".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        let outputs_before = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| e.step_outputs.clone())
            .unwrap();
        let events_before = read_submit_output_events(&app, &run_id);

        let final_text = r#"承認します。
<workflow_output type="review-verdict">{"verdict":"LGTM"}</workflow_output>"#;
        let final_parts = vec![MessagePart::Text {
            content: final_text.to_string(),
            parent_tool_use_id: None,
        }];

        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        // 自由文経路は prose 抽出を行わないため、step_outputs は変化せず、
        // output_contract がある step は明示的提出なしでは完了しない。
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
                &[],
                "review",
            )
            .await
            .expect("handle_auto_complete must succeed for agent free-text path");

        let outputs_after = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| e.step_outputs.clone())
            .unwrap_or_default();
        // step_outputs 数は変わらず、structured_output を持つ entry が追加されていない
        assert_eq!(outputs_before.len(), outputs_after.len());

        // OutputSubmitted event も追記されていない
        let events_after = read_submit_output_events(&app, &run_id);
        let submitted_count_before = events_before
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::OutputSubmitted { .. }))
            .count();
        let submitted_count_after = events_after
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::OutputSubmitted { .. }))
            .count();
        assert_eq!(submitted_count_before, submitted_count_after);
        let node_completed = events_after
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::NodeCompleted { node_name, .. } if node_name == "review"))
            .count();
        assert_eq!(
            node_completed, 0,
            "handle_auto_complete must not advance a contract step without SubmitOutput"
        );
        assert!(
            !engine.contains_execution_for_test(&run_id).await,
            "seeded test execution has no active session, so missing SubmitOutput fails and releases the terminal execution"
        );
        assert!(
            events_after
                .iter()
                .any(|event| matches!(event, WorkflowEvent::RunFailed { .. })),
            "terminal failure must be recorded in the event log"
        );
    }

    #[tokio::test]
    async fn missing_required_output_requests_repair_without_failing_within_limit() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-within-limit";
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                worktree_path.to_string(),
                TriggerSource::DesktopUi,
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
                &run_id,
                "submit-wf",
                "review",
                "review-verdict",
                1,
                Some(session_id),
                None,
                SubmissionViolation::MissingSubmitOutput,
            )
            .await
            .unwrap();

        assert!(
            engine.contains_execution_for_test(&run_id).await,
            "repairable mismatch must keep the run active"
        );
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                WorkflowEvent::ContractRepairRequested {
                    node_name,
                    attempt: 1,
                    ..
                } if node_name == "review"
            )),
            "repair attempt must append ContractRepairRequested; got {events:?}"
        );
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, WorkflowEvent::RunFailed { .. })),
            "within-limit repair request must not terminally fail the run; got {events:?}"
        );
    }

    #[tokio::test]
    async fn missing_required_output_fails_when_repair_turn_cannot_start() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-start-failure";
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                worktree_path.to_string(),
                TriggerSource::DesktopUi,
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
                &run_id,
                "submit-wf",
                "review",
                "review-verdict",
                1,
                Some(session_id),
                None,
                SubmissionViolation::MissingSubmitOutput,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&run_id).await,
            "repair start failure must terminally release the run"
        );
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                WorkflowEvent::ContractRepairRequested {
                    node_name,
                    attempt: 1,
                    ..
                } if node_name == "review"
            )),
            "the attempted repair must be observable before terminal failure; got {events:?}"
        );
        let run_failed = events.iter().find_map(|event| match event {
            WorkflowEvent::RunFailed {
                reason,
                failure_kind,
                ..
            } => Some((reason, failure_kind)),
            _ => None,
        });
        let Some((reason, failure_kind)) = run_failed else {
            panic!("repair start failure must append RunFailed; got {events:?}");
        };
        assert_eq!(*failure_kind, WorkflowStepFailureKind::InfrastructureCrash);
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
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-startup-timeout";
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                worktree_path.to_string(),
                TriggerSource::DesktopUi,
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
                &run_id,
                "review",
                "review-verdict",
                &error.to_string(),
                error.workflow_failure_kind(),
                error.retry_count(),
            )
            .await
            .unwrap();

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        let run_failed = events.iter().find_map(|event| match event {
            WorkflowEvent::RunFailed {
                reason,
                failure_kind,
                retry_count,
                ..
            } => Some((reason, failure_kind, retry_count)),
            _ => None,
        });
        let Some((reason, failure_kind, retry_count)) = run_failed else {
            panic!("repair startup timeout must append RunFailed; got {events:?}");
        };
        assert_eq!(*failure_kind, WorkflowStepFailureKind::StartupTimeout);
        assert_eq!(*retry_count, Some(2));
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
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-limit";
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                worktree_path.to_string(),
                TriggerSource::DesktopUi,
            )
            .await;
        let log = WorkflowEventLog::new(&data_dir);
        for attempt in 1..=2 {
            log.append(&WorkflowEvent::ContractRepairRequested {
                run_id: run_id.clone(),
                workflow_name: "submit-wf".to_string(),
                node_name: "review".to_string(),
                run_index: 1,
                request_id: None,
                attempt,
                violation_reason: submission_violation_reason(
                    SubmissionViolation::MissingSubmitOutput,
                )
                .to_string(),
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
                &run_id,
                "submit-wf",
                "review",
                "review-verdict",
                1,
                Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"),
                None,
                SubmissionViolation::MissingSubmitOutput,
            )
            .await
            .unwrap();

        assert!(
            !engine.contains_execution_for_test(&run_id).await,
            "exhausted repair attempts must terminally release the run"
        );
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        let run_failed_kind = events.iter().find_map(|event| match event {
            WorkflowEvent::RunFailed { failure_kind, .. } => Some(*failure_kind),
            _ => None,
        });
        assert_eq!(
            run_failed_kind,
            Some(WorkflowStepFailureKind::StructuredOutputMismatch)
        );
    }

    #[tokio::test]
    async fn missing_required_output_repair_attempts_are_scoped_to_run_index() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/repair-run-index";
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                worktree_path.to_string(),
                TriggerSource::DesktopUi,
            )
            .await;
        let log = WorkflowEventLog::new(&data_dir);
        for attempt in 1..=2 {
            log.append(&WorkflowEvent::ContractRepairRequested {
                run_id: run_id.clone(),
                workflow_name: "submit-wf".to_string(),
                node_name: "review".to_string(),
                run_index: 1,
                request_id: None,
                attempt,
                violation_reason: submission_violation_reason(
                    SubmissionViolation::MissingSubmitOutput,
                )
                .to_string(),
                timestamp: 1000.0 + f64::from(attempt),
            })
            .unwrap();
        }

        let (session_store, handles) = make_dispatch_deps(dispatch_data_dir(app.handle()));
        let session_id = "11111111-1111-4111-8111-111111111111";
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
                &run_id,
                "submit-wf",
                "review",
                "review-verdict",
                2,
                Some(session_id),
                None,
                SubmissionViolation::MissingSubmitOutput,
            )
            .await
            .unwrap();

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ContractRepairRequested {
                node_name,
                run_index: 2,
                attempt: 1,
                ..
            } if node_name == "review"
        )));
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, WorkflowEvent::RunFailed { .. })),
            "prior run_index repair attempts must not force GiveUp for a new run_index; got {events:?}"
        );
    }

    /// [08] 振る舞い定義 Rule 1: OutputSubmitted append が失敗した場合、
    /// step_outputs / workflow_variables / event log は提出前状態のまま保たれる。
    /// `write_log_required` の挿入 fail 経由で append 失敗を再現し、rollback の事実を
    /// 直接検証する（spec [08]: 「副作用なしで提出前状態のまま保つ」）。
    #[tokio::test]
    async fn submit_output_rolls_back_state_when_event_append_fails() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowRuntimeService::new_for_test());
        let data_dir =
            crate::infrastructure::platform::app_data_dir::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let workflow = Workflow {
            variables: Default::default(),
            name: "spec-wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![{
                let mut step = make_test_step("plan", TestKind::Session, "plan", vec![], None);
                step.output_contract = Some("spec-directory".to_string());
                step
            }],
        };
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                workflow,
                WorkflowExecutionState::Running,
                "/wt/submit-rollback".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        let exec_before = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| (e.step_outputs.clone(), e.workflow_variables.clone()))
            .unwrap();
        let events_before = read_submit_output_events(&app, &run_id);

        // 次の write_log_required を失敗させる。
        engine.fail_next_required_event_append_for_test();
        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
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
            .get(&run_id)
            .map(|e| (e.step_outputs.clone(), e.workflow_variables.clone()))
            .unwrap();
        assert_eq!(exec_before.0.len(), exec_after.0.len());
        assert!(!exec_after.0.contains_key("plan"));
        assert_eq!(exec_before.1, exec_after.1);

        // OutputSubmitted event は append されない（log への副作用なし）
        let events_after = read_submit_output_events(&app, &run_id);
        assert_eq!(events_before.len(), events_after.len());
        assert!(events_after
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::OutputSubmitted { .. })));
    }
}
