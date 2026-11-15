use super::{
    WorkspaceExecution, WorkspaceIdentity, WorkspaceNodeKind, WorkspaceNodeStatus,
    WorkspaceNodeStatusClassification, WorkspaceTree, WorkspaceTreeNode,
};
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{
    AgentSessionActivity, ExecutionStatus, ExecutionTreeLaunch, NodeCompletionSignalState,
};

pub(super) const WORKSPACE: &str = "/repo";

pub(super) fn node(
    id: &str,
    parent_id: Option<&str>,
    kind: WorkspaceNodeKind,
) -> WorkspaceTreeNode {
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
        model: None,
        provider: None,
        can_rename: false,
        can_approve: false,
        can_retry: false,
        can_resume_session: false,
        can_abort: false,
        can_archive: false,
        display_command: None,
        shell: None,
        cwd: None,
        command_result: None,
        dynamic_fanout: false,
    }
}

pub(super) fn in_execution(
    mut node: WorkspaceTreeNode,
    execution_id: &str,
    order: u64,
) -> WorkspaceTreeNode {
    node.execution_id = Some(execution_id.to_string());
    node.sibling_order = order;
    node
}

pub(super) fn execution(
    execution_id: &str,
    launched_as: ExecutionTreeLaunch,
) -> WorkspaceExecution {
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

pub(super) fn tree(
    nodes: Vec<WorkspaceTreeNode>,
    executions: Vec<WorkspaceExecution>,
) -> WorkspaceTree {
    let mut tree = WorkspaceTree::restore(WORKSPACE, nodes).unwrap();
    tree.record_executions(executions);
    tree
}

pub(super) fn session_execution(
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
