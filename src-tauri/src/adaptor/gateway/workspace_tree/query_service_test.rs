use super::*;
use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::test_support::{
    seed_workflow_session_facts, WorkflowSessionFactSeed,
};
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
use crate::domain::agent_session::repository::AgentSessionRepository;
use crate::domain::local_event::WorkflowExecutionMetadataRecord;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionStatus, TokenUsage, WorkflowExecutionArchiveSnapshot,
    WorkflowExecutionId,
};
use crate::domain::workspace_tree::{
    WorkspaceNodeStatus, WorkspaceNodeStatusClassification, WorkspaceTreeNode,
};
use crate::usecase::agent_session::{
    AgentSessionActivityDto, AgentSessionOperationsDto, AgentSessionProviderDto,
};

struct EmptyArchives;

impl WorkflowExecutionArchiveRepository for EmptyArchives {
    fn archive_manual(
        &self,
        _execution_id: &WorkflowExecutionId,
        _archived_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    fn restore_manual(
        &self,
        _execution_id: &WorkflowExecutionId,
        _restored_at: f64,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }

    fn manual_archive_snapshot_for(
        &self,
        _execution_ids: &[String],
    ) -> Result<WorkflowExecutionArchiveSnapshot, WorkflowError> {
        Ok(WorkflowExecutionArchiveSnapshot {
            records: Vec::new(),
        })
    }
}

#[tokio::test]
async fn launch区分が同じworktreeのworkflow一覧とsession一覧を分ける() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow",
            request: "test",
            worktree_path: "/repo",
            provider: ProviderKind::Codex,
            workflow_execution_id: "workflow-tree",
            node_execution_id: "workflow-node",
            session_id: "workflow-session",
            initial_instruction_admitted: true,
        },
    )
    .unwrap();
    LocalAgentSessionRepository::new(store.clone())
        .create(
            AgentSession::create(
                "standalone-session",
                WorkspaceIdentity::new("/repo"),
                "/repo",
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root("standalone-session").unwrap(),
            )
            .unwrap(),
            "standalone-create",
        )
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query =
        SqliteWorkspaceQueryService::with_repository(repository.clone(), Arc::new(EmptyArchives));

    let workflow_ids = query
        .execution_summaries(Some(&WorkspaceIdentity::new("/repo")), None, None)
        .unwrap()
        .into_iter()
        .map(|summary| summary.execution_id)
        .collect::<Vec<_>>();
    let tree_ids = repository
        .folded_workspace_trees("/repo")
        .unwrap()
        .into_iter()
        .map(|(tree, _)| tree.aggregate.id.clone())
        .collect::<Vec<_>>();
    let session_ids = crate::adaptor::gateway::agent_session::workspace_session_items(
        &repository.fact_backend(),
        &tree_ids,
        "/repo",
    )
    .unwrap()
    .into_iter()
    .map(|session| session.id)
    .collect::<Vec<_>>();

    assert_eq!(workflow_ids, ["workflow-tree"]);
    assert_eq!(session_ids, ["standalone-session"]);
}

#[tokio::test]
async fn test_workspace_tree_query_workspace同定子がworktreeと異なるsessionを一覧と行に含める() {
    // Given: workspace identity と worktree path が異なる Session 起動由来の木
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("workspace-1");
    LocalAgentSessionRepository::new(store.clone())
        .create(
            AgentSession::create(
                "standalone-session",
                workspace.clone(),
                "/repo/.worktrees/feature",
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root("standalone-session").unwrap(),
            )
            .unwrap(),
            "standalone-create",
        )
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);
    let query =
        SqliteWorkspaceQueryService::with_repository(repository.clone(), Arc::new(EmptyArchives));

    // When: workspace identity から snapshot と Session 一覧を取得する
    let snapshot = query.workspace_tree(&workspace).unwrap();
    let tree_ids = repository
        .folded_workspace_trees(workspace.as_str())
        .unwrap()
        .into_iter()
        .map(|(tree, _)| tree.aggregate.id.clone())
        .collect::<Vec<_>>();
    let sessions = crate::adaptor::gateway::agent_session::workspace_session_items(
        &repository.fact_backend(),
        &tree_ids,
        workspace.as_str(),
    )
    .unwrap();

    // Then: root の workspace identity を共通の対象キーとして解決する
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "standalone-session");
    assert_eq!(snapshot.nodes.len(), 1);
    assert!(matches!(
        &snapshot.nodes[0],
        WorkspaceTreeItemDto::Node(node) if node.id == "standalone-session"
    ));
}
fn node() -> WorkspaceTreeNode {
    WorkspaceTreeNode {
        id: "node".to_string(),
        parent_id: None,
        sibling_order: 0,
        kind: WorkspaceNodeKind::WorkflowSession,
        title: "Review".to_string(),
        status: WorkspaceNodeStatus::Waiting,
        status_classification: WorkspaceNodeStatusClassification::Attention,
        error_reason: None,
        updated_at_bits: 1.0f64.to_bits(),
        execution_id: None,
        node_execution_id: None,
        node_name: None,
        attempt: Some(1),
        retry_predecessor_id: None,
        past_attempt_ids: Vec::new(),
        is_retry_history: false,
        completion_signals: Default::default(),
        has_artifact: false,
        session_id: None,
        can_approve: true,
        can_retry: false,
        can_close: false,
        can_stop: false,
        can_resume: false,
        recovery_owner_reason: None,
        resume_unavailable_reason: None,
        can_abort: false,
        can_archive: false,
        display_command: None,
        command_result: None,
        dynamic_fanout: false,
    }
}

fn tree_owner(execution_id: &str) -> WorkspaceTreeNode {
    let mut owner = node();
    owner.id = execution_id.to_string();
    owner.kind = WorkspaceNodeKind::Workflow;
    owner.title = "Workflow owner".to_string();
    owner.status = WorkspaceNodeStatus::Running;
    owner.execution_id = Some(execution_id.to_string());
    owner.node_execution_id = None;
    owner.node_name = None;
    owner.attempt = None;
    owner.can_approve = false;
    owner.can_stop = true;
    owner.can_abort = true;
    owner
}

fn child_node(
    id: &str,
    parent_id: &str,
    execution_id: &str,
    kind: WorkspaceNodeKind,
    title: &str,
) -> WorkspaceTreeNode {
    let mut child = node();
    child.id = id.to_string();
    child.parent_id = Some(parent_id.to_string());
    child.kind = kind;
    child.title = title.to_string();
    child.status = WorkspaceNodeStatus::Running;
    child.execution_id = Some(execution_id.to_string());
    child.node_execution_id = Some(format!("{id}-execution"));
    child.node_name = Some(title.to_string());
    child.can_approve = false;
    child
}

fn open_session(id: &str) -> AgentSessionItemDto {
    AgentSessionItemDto {
        id: id.to_string(),
        workspace_identity: "/repo".to_string(),
        worktree_path: "/repo".to_string(),
        provider: AgentSessionProviderDto::Codex,
        tree_location: crate::usecase::agent_session::AgentSessionTreeLocationDto {
            tree_id: id.to_string(),
            node_execution_id: id.to_string(),
        },
        lifecycle: AgentSessionLifecycleDto::Open,
        provider_session_id: None,
        transcript_ref: None,
        operations: AgentSessionOperationsDto {
            can_archive: true,
            can_restore: false,
            can_delete: false,
            can_resume: false,
        },
        activity: AgentSessionActivityDto::Idle,
        last_exit_abnormal: false,
    }
}

#[test]
fn sequence_and_fanout_are_distinct_recursive_branches_under_the_public_root() {
    let execution_id = "workflow-execution";
    let owner = tree_owner(execution_id);
    let sequence = child_node(
        "sequence",
        execution_id,
        execution_id,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let fanout = child_node(
        "fanout",
        "sequence",
        execution_id,
        WorkspaceNodeKind::Fanout,
        "reviews",
    );
    let command = child_node(
        "command",
        "fanout",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "lint",
    );
    let tree = WorkspaceTree::restore("/repo", vec![owner, sequence, fanout, command]).unwrap();

    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    assert_eq!(json[0]["kind"], "sequence");
    assert_eq!(json[0]["id"], execution_id);
    assert_eq!(json[0]["status"], "active");
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], true);
    assert_eq!(json[0]["children"][0]["kind"], "fanout");
    assert_eq!(json[0]["children"][0]["status"], "active");
    assert_eq!(json[0]["children"][0]["children"][0]["kind"], "node");
    assert_eq!(json[0]["children"][0]["children"][0]["status"], "active");
}

#[test]
fn standalone_session_is_a_public_node_root_with_backend_lifecycle_capabilities() {
    let execution_id = "session-tree";
    let owner = tree_owner(execution_id);
    let mut session = child_node(
        "session-node",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "Codex Session",
    );
    session.session_id = Some("session-ref".to_string());
    let tree = WorkspaceTree::restore("/repo", vec![owner, session]).unwrap();

    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::new(),
        &[open_session("session-ref")],
    ))
    .unwrap();

    assert_eq!(json[0]["kind"], "node");
    assert_eq!(json[0]["id"], execution_id);
    assert_eq!(json[0]["contentKind"], "session");
    assert_eq!(json[0]["sessionCapabilities"]["sessionRef"], "session-ref");
    assert_eq!(json[0]["sessionCapabilities"]["canArchive"], true);
    assert!(json[0]["workflowCapabilities"].is_null());
}

#[test]
fn leaf_workflow_root_keeps_workflow_capabilities_on_the_node() {
    let execution_id = "leaf-workflow";
    let owner = tree_owner(execution_id);
    let leaf = child_node(
        "leaf",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "main",
    );
    let tree = WorkspaceTree::restore("/repo", vec![owner, leaf]).unwrap();

    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    assert_eq!(json[0]["kind"], "node");
    assert_eq!(json[0]["id"], execution_id);
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], true);
}

#[test]
fn test_workflow_session_node_detail_session_surfaceを公開する() {
    let mut workflow_session = node();
    workflow_session.session_id = Some("agent-session-1".to_string());

    let detail = serde_json::to_value(node_detail(workflow_session)).unwrap();

    assert_eq!(
        detail["content"]["kind"],
        serde_json::Value::String("session".to_string())
    );
    assert_eq!(
        detail["content"]["sessionId"],
        serde_json::Value::String("agent-session-1".to_string())
    );
}

#[test]
fn workflow_node_detail_exposes_backend_owned_signal_and_capabilities_without_attempt() {
    let mut workflow_session = node();
    workflow_session.node_execution_id = Some("node-execution-1".to_string());
    workflow_session.execution_id = Some("execution-1".to_string());
    workflow_session.node_name = Some("Review".to_string());
    workflow_session.session_id = Some("agent-session-1".to_string());
    workflow_session.completion_signals =
        crate::domain::workflow::NodeCompletionSignalState::StopReceived;
    workflow_session.has_artifact = false;
    workflow_session.can_retry = true;

    let detail = serde_json::to_value(node_detail(workflow_session)).unwrap();

    assert!(detail.get("attempt").is_none());
    assert_eq!(detail["submitReceived"], false);
    assert_eq!(detail["stopReceived"], true);
    assert_eq!(detail["waitingFor"], "submit");
    assert_eq!(detail["hasArtifact"], false);
    assert_eq!(detail["capabilities"]["canRetry"], true);
}

#[test]
fn test_workspaceツリー契約_nodeとsequenceとfanoutは4分類だけを返す() {
    // Given
    let execution_id = "workflow-execution";
    let owner = tree_owner(execution_id);
    let sequence = child_node(
        "sequence",
        execution_id,
        execution_id,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let fanout = child_node(
        "fanout",
        "sequence",
        execution_id,
        WorkspaceNodeKind::Fanout,
        "reviews",
    );
    let mut failed = child_node(
        "failed",
        "fanout",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "lint",
    );
    failed.status = WorkspaceNodeStatus::Failed;
    let tree = WorkspaceTree::restore("/repo", vec![owner, sequence, fanout, failed]).unwrap();

    // When
    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    // Then
    for status in [
        &json[0]["status"],
        &json[0]["children"][0]["status"],
        &json[0]["children"][0]["children"][0]["status"],
    ] {
        assert_eq!(status, "failure");
        assert!(![
            "running",
            "paused",
            "failed",
            "waiting",
            "aborted",
            "completed",
            "interrupted",
        ]
        .contains(&status.as_str().unwrap()));
    }
}

#[test]
fn test_workspaceノード詳細契約_詳細状態と4分類を同時に返す() {
    // Given
    let cases = [
        (
            WorkspaceNodeStatus::Running,
            WorkspaceNodeStatusClassification::Active,
            "running",
            "active",
        ),
        (
            WorkspaceNodeStatus::Paused,
            WorkspaceNodeStatusClassification::Idle,
            "paused",
            "idle",
        ),
        (
            WorkspaceNodeStatus::Failed,
            WorkspaceNodeStatusClassification::Failure,
            "failed",
            "failure",
        ),
        (
            WorkspaceNodeStatus::Waiting,
            WorkspaceNodeStatusClassification::Attention,
            "waiting",
            "attention",
        ),
        (
            WorkspaceNodeStatus::Aborted,
            WorkspaceNodeStatusClassification::Idle,
            "aborted",
            "idle",
        ),
        (
            WorkspaceNodeStatus::Completed,
            WorkspaceNodeStatusClassification::Idle,
            "completed",
            "idle",
        ),
    ];

    // When / Then
    for (status, classification, expected_status, expected_classification) in cases {
        let mut current = node();
        current.status = status;
        current.status_classification = classification;
        let detail = serde_json::to_value(node_detail(current)).unwrap();
        assert_eq!(detail["status"], expected_status);
        assert_eq!(detail["statusClassification"], expected_classification);
        assert_ne!(detail["status"], "interrupted");
        assert_ne!(detail["statusClassification"], "interrupted");
    }
}

#[test]
fn test_workspaceツリー契約_recovery_fenceありでも操作capabilityとresume不能理由を維持する() {
    // Given
    let execution_id = "workflow-execution";
    let owner = tree_owner(execution_id);
    let mut sequence = child_node(
        "sequence",
        execution_id,
        execution_id,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    sequence.status = WorkspaceNodeStatus::Completed;
    let mut approval = child_node(
        "approval",
        "sequence",
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "approval",
    );
    approval.status = WorkspaceNodeStatus::Waiting;
    approval.can_approve = true;
    let mut failed = child_node(
        "failed",
        "sequence",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "failed",
    );
    failed.sibling_order = 1;
    failed.status = WorkspaceNodeStatus::Failed;
    failed.can_retry = true;
    let mut running = child_node(
        "running",
        "sequence",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "running",
    );
    running.sibling_order = 2;
    let mut fenced_paused = child_node(
        "fenced-paused",
        "sequence",
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "fenced-paused",
    );
    fenced_paused.sibling_order = 3;
    fenced_paused.status = WorkspaceNodeStatus::Paused;
    fenced_paused.recovery_owner_reason = Some("recovery fence".to_string());
    let tree = WorkspaceTree::restore(
        "/repo",
        vec![owner, sequence, approval, failed, running, fenced_paused],
    )
    .unwrap();

    // When
    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    // Then
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], true);
    assert_eq!(json[0]["workflowCapabilities"]["canResume"], false);
    assert_eq!(
        json[0]["workflowCapabilities"]["resumeUnavailableReason"],
        "recovery fence"
    );
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], true);
    assert_eq!(json[0]["workflowCapabilities"]["canArchive"], false);
    assert_eq!(json[0]["children"][0]["capabilities"]["canApprove"], true);
    assert_eq!(json[0]["children"][1]["capabilities"]["canRetry"], true);
}

#[test]
fn test_workspaceツリー契約_pausedでもresume可否とresume不能理由を維持する() {
    // Given
    let execution_id = "paused-workflow-execution";
    let owner = tree_owner(execution_id);
    let mut paused = child_node(
        "paused",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "paused",
    );
    paused.status = WorkspaceNodeStatus::Paused;
    let tree = WorkspaceTree::restore("/repo", vec![owner, paused]).unwrap();

    // When
    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    // Then
    assert_eq!(json[0]["status"], "idle");
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canResume"], true);
    assert!(json[0]["workflowCapabilities"]["resumeUnavailableReason"].is_null());
}

#[test]
fn test_workspaceツリー契約_completedでも終了時capabilityを維持する() {
    // Given
    let execution_id = "completed-workflow-execution";
    let mut owner = tree_owner(execution_id);
    owner.status = WorkspaceNodeStatus::Completed;
    owner.can_stop = false;
    owner.can_abort = false;
    owner.can_archive = true;
    let mut completed = child_node(
        "completed",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "completed",
    );
    completed.status = WorkspaceNodeStatus::Completed;
    let tree = WorkspaceTree::restore("/repo", vec![owner, completed]).unwrap();

    // When
    let json = serde_json::to_value(project_tree(
        &tree,
        &HashSet::new(),
        &HashSet::from([execution_id.to_string()]),
        &[],
    ))
    .unwrap();

    // Then
    assert_eq!(json[0]["status"], "idle");
    assert_eq!(json[0]["workflowCapabilities"]["canStop"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canResume"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canArchive"], true);
}

#[test]
fn execution_summary_rejects_non_finite_timestamp() {
    let record = WorkflowExecutionMetadataRecord {
        execution_id: "execution".to_string(),
        workflow_name: "workflow".to_string(),
        status: ExecutionStatus::Running,
        worktree_path: "/repo".to_string(),
        current_node: None,
        created_from: ExecutionOrigin::DesktopUi,
        started_at_bits: f64::NAN.to_bits(),
        updated_at_bits: 1.0f64.to_bits(),
        completed_at_bits: None,
        error_reason: None,
        interruption_reason: None,
        resume_from_node: None,
        total_token_usage: TokenUsage::default(),
    };
    assert!(matches!(
        execution_summary(record),
        Err(WorkflowError::CorruptStoredState(_))
    ));
}

#[test]
fn test_workspace_query_error_corruptをcorrupt_stored_stateへ写像する() {
    // Given
    let correlation_id = "workspace-corrupt-correlation";

    // When
    let error = query_error(crate::domain::local_event::LocalEventQueryError::Corrupt {
        correlation_id: correlation_id.to_string(),
    });

    // Then
    assert_eq!(
        error,
        WorkflowError::CorruptStoredState(format!(
            "store corrupt (correlation_id={correlation_id})"
        ))
    );
}

#[test]
fn unrepresentable_page_offset_falls_back_to_the_first_record() {
    assert_eq!(
        sqlite_page_bounds(Some(WorkflowPageRequest::new(usize::MAX, usize::MAX))),
        (i64::MAX, 0)
    );
    assert_eq!(sqlite_page_bounds(None), (i64::MAX, 0));
}
