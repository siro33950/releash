use super::*;
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{
    AgentSessionActivity, ExecutionStatus, ExecutionTreeArchiveRecord, NodeCompletionSignalState,
};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceNodeStatus, WorkspaceNodeStatusClassification,
};

const WORKSPACE: &str = "/repo";

fn node(id: &str, parent_id: Option<&str>, kind: WorkspaceNodeKind) -> WorkspaceTreeNode {
    let leaf_or_branch = kind != WorkspaceNodeKind::Workflow;
    WorkspaceTreeNode {
        worktree: None,
        id: id.to_string(),
        parent_id: parent_id.map(str::to_string),
        sibling_order: 0,
        kind,
        title: id.to_string(),
        status: WorkspaceNodeStatus::Running,
        process_presence: Default::default(),
        status_classification: WorkspaceNodeStatusClassification::Active,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: (kind == WorkspaceNodeKind::WorkflowSession).then(AgentSessionActivity::default),
        error_reason: None,
        updated_at_bits: 0.0_f64.to_bits(),
        execution_id: Some("execution".to_string()),
        node_execution_id: leaf_or_branch.then(|| format!("{id}-exec")),
        node_name: leaf_or_branch.then(|| id.to_string()),
        attempt: leaf_or_branch.then_some(1),
        retry_predecessor_id: None,
        past_attempt_ids: Vec::new(),
        is_retry_history: false,
        completion_signals: NodeCompletionSignalState::Pending,
        has_artifact: false,
        session_id: None,
        can_rename: false,
        can_approve: false,
        can_retry: false,
        can_resume_session: false,
        can_abort: false,
        can_archive: false,
        display_command: None,
        command_result: None,
        dynamic_fanout: false,
    }
}

fn in_execution(mut node: WorkspaceTreeNode, execution_id: &str, order: u64) -> WorkspaceTreeNode {
    node.execution_id = Some(execution_id.to_string());
    node.sibling_order = order;
    node
}

fn execution(execution_id: &str, launched_as: ExecutionTreeLaunch) -> WorkspaceExecution {
    WorkspaceExecution {
        execution_id: execution_id.to_string(),
        launched_as,
        worktree_path: WORKSPACE.to_string(),
        workflow_name: "wf".to_string(),
        status: ExecutionStatus::Completed,
        updated_at: 1.0,
        archive: None,
        session: None,
    }
}

fn archived(mut execution: WorkspaceExecution, archived_at: f64) -> WorkspaceExecution {
    execution.archive = Some(ExecutionTreeArchiveRecord {
        execution_id: execution.execution_id.clone(),
        archived_at,
        archive_reason: "manual".to_string(),
    });
    execution
}

/// workflow の実行（sequence → review, fix）と、その retry で置き換えられた過去の試行。
fn workflow_nodes(execution_id: &str) -> Vec<WorkspaceTreeNode> {
    let owner = execution_id;
    let main = format!("{execution_id}-main");
    let mut retried = in_execution(
        node(
            &format!("{execution_id}-fix-1"),
            Some(&main),
            WorkspaceNodeKind::WorkflowSession,
        ),
        execution_id,
        1,
    );
    retried.status = WorkspaceNodeStatus::Aborted;
    let mut fix = in_execution(
        node(
            &format!("{execution_id}-fix-2"),
            Some(&main),
            WorkspaceNodeKind::WorkflowSession,
        ),
        execution_id,
        2,
    );
    fix.retry_predecessor_id = retried.node_execution_id.clone();
    vec![
        in_execution(
            node(owner, None, WorkspaceNodeKind::Workflow),
            execution_id,
            0,
        ),
        in_execution(
            node(&main, Some(owner), WorkspaceNodeKind::Sequence),
            execution_id,
            0,
        ),
        in_execution(
            node(
                &format!("{execution_id}-review"),
                Some(&main),
                WorkspaceNodeKind::WorkflowSession,
            ),
            execution_id,
            0,
        ),
        retried,
        fix,
    ]
}

fn tree(nodes: Vec<WorkspaceTreeNode>, executions: Vec<WorkspaceExecution>) -> WorkspaceTree {
    let mut tree = WorkspaceTree::restore(WORKSPACE, nodes).unwrap();
    tree.record_executions(executions);
    tree
}

#[test]
fn test_画面に出す木_実行を代表する節は実行の識別子と題で公開し子を並び順で返す() {
    // Given
    let tree = tree(
        workflow_nodes("exec-a"),
        vec![execution("exec-a", ExecutionTreeLaunch::Workflow)],
    );

    // When
    let visible = tree.visible();
    let roots = visible.roots();

    // Then: Workflow を表す節は現れず、最初の子が実行を代表する
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id(), "exec-a");
    assert_eq!(roots[0].title(), "exec-a");
    assert_eq!(roots[0].node().kind, WorkspaceNodeKind::Sequence);
    assert_eq!(roots[0].execution_owner().unwrap().id, "exec-a");
    let children = roots[0].children();
    assert_eq!(
        children.iter().map(|child| child.id()).collect::<Vec<_>>(),
        vec!["exec-a-review", "exec-a-fix-2"]
    );
    assert!(children[0].execution_owner().is_none());
    assert_eq!(
        children[1]
            .past_attempts()
            .iter()
            .map(|past| past.id())
            .collect::<Vec<_>>(),
        vec!["exec-a-fix-1"]
    );
}

#[test]
fn test_画面に出す木_archive済みの実行を除き残りから既定の選択先を決める() {
    // Given: archive 済みの実行と、実行中の実行
    let mut nodes = workflow_nodes("exec-a");
    nodes.extend(workflow_nodes("exec-b"));
    let tree = tree(
        nodes,
        vec![
            archived(execution("exec-a", ExecutionTreeLaunch::Workflow), 5.0),
            execution("exec-b", ExecutionTreeLaunch::Workflow),
        ],
    );

    // When
    let visible = tree.visible();

    // Then
    assert_eq!(
        visible
            .roots()
            .iter()
            .map(|root| root.id())
            .collect::<Vec<_>>(),
        vec!["exec-b"]
    );
    assert_eq!(
        visible.preferred_node_id().as_deref(),
        Some("exec-b-review")
    );
    assert!(!visible.contains("exec-a-review"));
}

#[test]
fn test_画面に出す木_選択先は葉と過去の試行だけを含み合成子自身は含まない() {
    // Given
    let tree = tree(
        workflow_nodes("exec-a"),
        vec![execution("exec-a", ExecutionTreeLaunch::Workflow)],
    );

    // When
    let visible = tree.visible();

    // Then
    assert!(visible.contains("exec-a-review"));
    assert!(visible.contains("exec-a-fix-1"));
    assert!(!visible.contains("exec-a"));
    assert!(!visible.contains("missing"));
}

fn session_execution(
    session_id: &str,
    archive: bool,
) -> (Vec<WorkspaceTreeNode>, WorkspaceExecution) {
    let workspace = WorkspaceIdentity::new(WORKSPACE);
    let mut session = AgentSession::create(
        session_id,
        workspace,
        WORKSPACE,
        ProviderKind::Codex,
        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
    )
    .unwrap();
    session.take_uncommitted_events();
    if archive {
        session.restore_derived_lifecycle(
            crate::domain::agent_session::aggregates::AgentSessionLifecycle::Archived,
            false,
            AgentSessionActivity::default(),
        );
    }
    let owner = in_execution(
        node(session_id, None, WorkspaceNodeKind::Workflow),
        session_id,
        0,
    );
    let mut leaf = in_execution(
        node(
            &format!("node-{session_id}"),
            Some(session_id),
            WorkspaceNodeKind::WorkflowSession,
        ),
        session_id,
        0,
    );
    leaf.node_execution_id = Some(session_id.to_string());
    leaf.session_id = Some(session_id.to_string());
    let mut execution = execution(session_id, ExecutionTreeLaunch::Session);
    execution.session = Some(session);
    (vec![owner, leaf], execution)
}

#[test]
fn test_画面に出す木_sessionとして起動した実行の根はそのsessionを返す() {
    // Given
    let (nodes, execution) = session_execution("session-b", false);
    let tree = tree(nodes, vec![execution]);

    // When
    let visible = tree.visible();
    let roots = visible.roots();

    // Then
    assert_eq!(roots[0].id(), "session-b");
    assert_eq!(roots[0].session().map(AgentSession::id), Some("session-b"));
    assert!(tree.archived_sessions().is_empty());
}

#[test]
fn test_archive済みsession_識別子順に返しworkflowの実行は含めない() {
    // Given
    let (mut nodes, second) = session_execution("session-b", true);
    let (first_nodes, first) = session_execution("session-a", true);
    nodes.extend(first_nodes);
    nodes.extend(workflow_nodes("exec-a"));
    let tree = tree(
        nodes,
        vec![
            second,
            first,
            archived(execution("exec-a", ExecutionTreeLaunch::Workflow), 5.0),
        ],
    );

    // When
    let archived = tree.archived_sessions();

    // Then
    assert_eq!(
        archived
            .iter()
            .map(|execution| execution.execution_id.as_str())
            .collect::<Vec<_>>(),
        vec!["session-a", "session-b"]
    );
}

#[test]
fn test_archive済みworkflow_archiveの新しい順に返しsessionと未archiveは含めない() {
    // Given
    let (session_nodes, session) = session_execution("session-a", true);
    let mut nodes = session_nodes;
    for execution_id in ["exec-a", "exec-b", "exec-c"] {
        nodes.extend(workflow_nodes(execution_id));
    }
    let tree = tree(
        nodes,
        vec![
            archived(session, 9.0),
            archived(execution("exec-a", ExecutionTreeLaunch::Workflow), 5.0),
            archived(execution("exec-b", ExecutionTreeLaunch::Workflow), 7.0),
            execution("exec-c", ExecutionTreeLaunch::Workflow),
        ],
    );

    // When
    let history = tree.archived_workflows();

    // Then
    assert_eq!(
        history
            .iter()
            .map(|execution| execution.execution_id.as_str())
            .collect::<Vec<_>>(),
        vec!["exec-b", "exec-a"]
    );
}
