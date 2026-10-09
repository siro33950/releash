use super::*;
use crate::adaptor::presenter::client::from_message;
use crate::domain::agent_session::aggregates::AgentSessionTreeLocation;
use crate::domain::git_host::PrInfo;
use crate::domain::repository::Worktree;
use crate::domain::workflow::{ExecutionStatus, ExecutionTreeArchiveRecord, ExecutionTreeLaunch};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceNodeStatus, WorkspaceNodeStatusClassification,
};
use serde_json::json;

fn node() -> WorkspaceTreeNode {
    WorkspaceTreeNode {
        process_presence: Default::default(),
        can_resume_session: false,
        worktree: None,
        id: "node".to_string(),
        parent_id: None,
        sibling_order: 0,
        kind: WorkspaceNodeKind::WorkflowSession,
        title: "Review".to_string(),
        status: WorkspaceNodeStatus::Waiting,
        status_classification: WorkspaceNodeStatusClassification::Attention,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: Some(crate::domain::workflow::AgentSessionActivity::AwaitingInstruction),
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
        can_rename: false,
        can_approve: true,
        can_retry: false,
        can_abort: false,
        can_archive: false,
        display_command: None,
        command_result: None,
        dynamic_fanout: false,
    }
}

/// 実行木の一覧を転送の形にして、画面が受け取る JSON にする。
fn snapshot_json(tree: &WorkspaceTree) -> serde_json::Value {
    from_message(
        "releash.client.v1.WorkspaceTreeSnapshot",
        &tree_snapshot(tree).unwrap(),
    )
    .unwrap()
}

fn nodes_json(tree: &WorkspaceTree) -> serde_json::Value {
    snapshot_json(tree)["nodes"].clone()
}

fn workflow_execution(execution_id: &str) -> WorkspaceExecution {
    WorkspaceExecution {
        execution_id: execution_id.to_string(),
        launched_as: ExecutionTreeLaunch::Workflow,
        worktree_path: "/repo".to_string(),
        workflow_name: "workflow".to_string(),
        status: ExecutionStatus::Running,
        updated_at: 1.0,
        archive: None,
        session: None,
    }
}

/// Session として起動した実行。
fn session_execution(execution_id: &str, session_id: &str) -> WorkspaceExecution {
    WorkspaceExecution {
        launched_as: ExecutionTreeLaunch::Session,
        session: Some(
            AgentSession::create(
                session_id,
                WorkspaceIdentity::new("/repo"),
                "/repo",
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
            )
            .unwrap(),
        ),
        ..workflow_execution(execution_id)
    }
}

fn tree_owner(execution_id: &str) -> WorkspaceTreeNode {
    let mut owner = node();
    owner.id = execution_id.to_string();
    owner.kind = WorkspaceNodeKind::Workflow;
    owner.activity = None;
    owner.title = "Workflow owner".to_string();
    owner.status = WorkspaceNodeStatus::Running;
    owner.execution_id = Some(execution_id.to_string());
    owner.node_execution_id = None;
    owner.node_name = None;
    owner.attempt = None;
    owner.can_approve = false;
    owner.can_abort = true;
    owner.can_archive = true;
    owner
}

fn tree_owner_with_title(execution_id: &str, title: &str) -> WorkspaceTreeNode {
    let mut owner = tree_owner(execution_id);
    owner.title = title.to_string();
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
    child.activity = (kind == WorkspaceNodeKind::WorkflowSession)
        .then(crate::domain::workflow::AgentSessionActivity::default);
    child.title = title.to_string();
    child.status = WorkspaceNodeStatus::Running;
    child.execution_id = Some(execution_id.to_string());
    child.node_execution_id = Some(format!("{id}-execution"));
    child.node_name = Some(title.to_string());
    child.can_approve = false;
    child
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

    let json = nodes_json(&tree);

    assert_eq!(json[0]["kind"], "sequence");
    assert_eq!(json[0]["id"], execution_id);
    assert_eq!(json[0]["status"], "active");
    assert!(json[0]["workflowCapabilities"].get("canStop").is_none());
    assert!(json[0]["workflowCapabilities"].get("canResume").is_none());
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], true);
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
    let mut tree = WorkspaceTree::restore("/repo", vec![owner, session]).unwrap();
    tree.record_executions(vec![session_execution(execution_id, "session-ref")]);

    let json = nodes_json(&tree);

    assert_eq!(json[0]["kind"], "node");
    assert_eq!(json[0]["id"], execution_id);
    assert_eq!(json[0]["contentKind"], "session");
    assert_eq!(json[0]["sessionCapabilities"]["sessionRef"], "session-ref");
    assert_eq!(json[0]["sessionCapabilities"]["canArchive"], true);
    assert_eq!(json[0]["sessionCapabilities"]["canDelete"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canArchive"], true);
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], true);
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

    let json = nodes_json(&tree);

    assert_eq!(json[0]["kind"], "node");
    assert_eq!(json[0]["id"], execution_id);
    assert!(json[0]["workflowCapabilities"].get("canStop").is_none());
    assert!(json[0]["workflowCapabilities"].get("canResume").is_none());
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], true);
}

fn assert_public_root_title(kind: WorkspaceNodeKind) {
    // Given
    let execution_id = "workflow-execution";
    let owner = tree_owner_with_title(execution_id, "01_author-spec");
    let public_root = child_node("root", execution_id, execution_id, kind, "main");
    let tree = WorkspaceTree::restore("/repo", vec![owner, public_root]).unwrap();

    // When
    let json = nodes_json(&tree);

    // Then
    assert_eq!(json[0]["title"], "01_author-spec");
    assert_ne!(json[0]["title"], "main");
}

#[test]
fn test_workspaceツリー投影_sequenceのpublic_root表示名にworkflow名を返す() {
    assert_public_root_title(WorkspaceNodeKind::Sequence);
}

#[test]
fn test_workspaceツリー投影_fanoutのpublic_root表示名にworkflow名を返す() {
    assert_public_root_title(WorkspaceNodeKind::Fanout);
}

#[test]
fn test_workspaceツリー投影_sessionのpublic_root表示名にworkflow名を返す() {
    assert_public_root_title(WorkspaceNodeKind::WorkflowSession);
}

#[test]
fn test_workspaceツリー投影_commandのpublic_root表示名にworkflow名を返す() {
    assert_public_root_title(WorkspaceNodeKind::WorkflowCommand);
}

#[test]
fn test_workspaceツリー投影_異なるworkflow名の複数実行を表示名で判別できる() {
    // Given
    let execution_a = "execution-a";
    let execution_b = "execution-b";
    let owner_a = tree_owner_with_title(execution_a, "01_author-spec");
    let owner_b = tree_owner_with_title(execution_b, "03_full-review");
    let root_a = child_node(
        "root-a",
        execution_a,
        execution_a,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let root_b = child_node(
        "root-b",
        execution_b,
        execution_b,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let tree = WorkspaceTree::restore("/repo", vec![owner_a, root_a, owner_b, root_b]).unwrap();

    // When
    let json = nodes_json(&tree);

    // Then
    let row_a = json
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == execution_a)
        .unwrap();
    let row_b = json
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == execution_b)
        .unwrap();
    assert_eq!(row_a["title"], "01_author-spec");
    assert_eq!(row_b["title"], "03_full-review");
    assert_ne!(row_a["title"], row_b["title"]);
}

#[test]
fn test_workspaceツリー投影_public_root以外はnode名を表示名に保つ() {
    // Given
    let execution_id = "workflow-execution";
    let owner = tree_owner_with_title(execution_id, "01_author-spec");
    let public_root = child_node(
        "root",
        execution_id,
        execution_id,
        WorkspaceNodeKind::Sequence,
        "main",
    );
    let child_sequence = child_node(
        "child-sequence",
        "root",
        execution_id,
        WorkspaceNodeKind::Sequence,
        "prepare",
    );
    let mut child_fanout = child_node(
        "child-fanout",
        "root",
        execution_id,
        WorkspaceNodeKind::Fanout,
        "reviews",
    );
    child_fanout.sibling_order = 1;
    let mut child_session = child_node(
        "child-session",
        "root",
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "author",
    );
    child_session.sibling_order = 2;
    let mut child_command = child_node(
        "child-command",
        "root",
        execution_id,
        WorkspaceNodeKind::WorkflowCommand,
        "lint",
    );
    child_command.sibling_order = 3;
    let tree = WorkspaceTree::restore(
        "/repo",
        vec![
            owner,
            public_root,
            child_sequence,
            child_fanout,
            child_session,
            child_command,
        ],
    )
    .unwrap();

    // When
    let json = nodes_json(&tree);

    // Then
    assert_eq!(json[0]["title"], "01_author-spec");
    assert_eq!(json[0]["children"][0]["title"], "prepare");
    assert_eq!(json[0]["children"][1]["title"], "reviews");
    assert_eq!(json[0]["children"][2]["title"], "author");
    assert_eq!(json[0]["children"][3]["title"], "lint");
}

#[test]
fn test_workspaceツリー契約_nodeとsequenceとfanoutは3分類だけを返す() {
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
    failed.process_presence = crate::domain::workflow::NodeProcessPresence::ConfirmedAbsent;
    let tree = WorkspaceTree::restore("/repo", vec![owner, sequence, fanout, failed]).unwrap();

    // When
    let json = nodes_json(&tree);

    // Then
    for status in [
        &json[0]["status"],
        &json[0]["children"][0]["status"],
        &json[0]["children"][0]["children"][0]["status"],
    ] {
        assert_eq!(status, "attention");
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
fn test_workspaceツリー契約_bind前sessionを青で公開する() {
    let execution_id = "workflow-execution";
    let owner = tree_owner(execution_id);
    let mut session = child_node(
        "session",
        execution_id,
        execution_id,
        WorkspaceNodeKind::WorkflowSession,
        "session",
    );
    session.status_classification = WorkspaceNodeStatusClassification::Active;
    session.can_rename = true;
    let tree = WorkspaceTree::restore("/repo", vec![owner, session]).unwrap();

    let json = nodes_json(&tree);

    assert_eq!(json[0]["status"], "active");
    assert_eq!(json[0]["capabilities"]["canRename"], false);
    assert!(json[0].get("workflowCapabilities").is_some());
}

#[test]
fn test_workspaceツリー契約_completedでも終了時capabilityを維持する() {
    // Given
    let execution_id = "completed-workflow-execution";
    let mut owner = tree_owner(execution_id);
    owner.status = WorkspaceNodeStatus::Completed;
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
    let json = nodes_json(&tree);

    // Then
    assert_eq!(json[0]["status"], "idle");
    assert!(json[0]["workflowCapabilities"].get("canStop").is_none());
    assert!(json[0]["workflowCapabilities"].get("canResume").is_none());
    assert_eq!(json[0]["workflowCapabilities"]["canAbort"], false);
    assert_eq!(json[0]["workflowCapabilities"]["canArchive"], true);
}

#[test]
fn test_隔離合成子の表示_空のchildrenや終端でもそのattemptのbranchとpathを返す() {
    for kind in [WorkspaceNodeKind::Sequence, WorkspaceNodeKind::Fanout] {
        for status in [
            WorkspaceNodeStatus::Running,
            WorkspaceNodeStatus::Completed,
            WorkspaceNodeStatus::Aborted,
        ] {
            // Given
            let owner = tree_owner("tree");
            let mut composite = child_node("composite", "tree", "tree", kind, "main");
            let expected = crate::domain::workflow::IsolatedWorktree::for_attempt(
                "/repo",
                "composite-execution",
                2,
            );
            composite.attempt = Some(2);
            composite.status = status;
            composite.worktree = Some(expected.clone());
            let tree = WorkspaceTree::restore("/repo", vec![owner, composite]).unwrap();
            // When
            let items = nodes_json(&tree);
            // Then
            assert_eq!(items[0]["worktree"]["branch"], expected.branch);
            assert_eq!(items[0]["worktree"]["path"], expected.path);
            assert!(items[0]["children"].as_array().unwrap().is_empty());
        }
    }
}

#[test]
fn test_delegate_親session配下に発火順の子を表示し子の部分木も保持する() {
    // Given
    let owner = tree_owner("tree");
    let parent = child_node(
        "parent",
        "tree",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "implement",
    );
    let mut first = child_node(
        "first",
        "parent",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "verify",
    );
    first.sibling_order = 0;
    first.attempt = Some(1);
    first.status = WorkspaceNodeStatus::Completed;
    let mut second = child_node(
        "second",
        "parent",
        "tree",
        WorkspaceNodeKind::Sequence,
        "checks",
    );
    second.sibling_order = 1;
    second.attempt = Some(2);
    let nested = child_node(
        "nested",
        "second",
        "tree",
        WorkspaceNodeKind::WorkflowCommand,
        "check",
    );
    let tree = WorkspaceTree::restore("/repo", vec![owner, parent, first, second, nested]).unwrap();
    // When
    let json = nodes_json(&tree);
    // Then
    assert_eq!(json[0]["kind"], "node");
    assert_eq!(json[0]["children"][0]["title"], "verify");
    assert_eq!(json[0]["children"][1]["kind"], "sequence");
    assert_eq!(json[0]["children"][1]["children"][0]["title"], "check");
}

#[test]
fn test_delegate_親の過去attemptに当該attemptのchild部分木を投影する() {
    // Given
    let owner = tree_owner("tree");
    let mut past = child_node(
        "past",
        "tree",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "implement",
    );
    past.attempt = Some(1);
    past.status = WorkspaceNodeStatus::Aborted;
    let mut current = child_node(
        "current",
        "tree",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "implement",
    );
    current.attempt = Some(2);
    current.sibling_order = 1;
    current.retry_predecessor_id = Some("past-execution".into());
    let prior_child = child_node(
        "prior-checks",
        "past",
        "tree",
        WorkspaceNodeKind::Sequence,
        "checks",
    );
    let prior_leaf = child_node(
        "prior-judge",
        "prior-checks",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "judge",
    );
    let current_child = child_node(
        "current-checks",
        "current",
        "tree",
        WorkspaceNodeKind::WorkflowSession,
        "checks",
    );
    let tree = WorkspaceTree::restore(
        "/repo",
        vec![owner, past, current, prior_child, prior_leaf, current_child],
    )
    .unwrap();
    // When
    let result = nodes_json(&tree);
    // Then
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["children"][0]["id"], "current-checks");
    assert_eq!(result[0]["pastAttempts"][0]["id"], "past");
    assert_eq!(result[0]["pastAttempts"][0]["kind"], "node");
    assert_eq!(result[0]["pastAttempts"][0]["contentKind"], "session");
    assert_eq!(
        result[0]["pastAttempts"][0]["children"][0]["id"],
        "prior-checks"
    );
    assert_eq!(
        result[0]["pastAttempts"][0]["children"][0]["children"][0]["id"],
        "prior-judge"
    );
}

#[test]
fn test_過去attempt_子のないsessionとcommandも通常行と同じkindを返す() {
    for (kind, content_kind) in [
        (WorkspaceNodeKind::WorkflowSession, "session"),
        (WorkspaceNodeKind::WorkflowCommand, "command"),
    ] {
        // Given
        let owner = tree_owner("tree");
        let mut past = child_node("past", "tree", "tree", kind, "work");
        past.attempt = Some(1);
        past.status = WorkspaceNodeStatus::Aborted;
        let mut current = child_node("current", "tree", "tree", kind, "work");
        current.attempt = Some(2);
        current.sibling_order = 1;
        current.retry_predecessor_id = Some("past-execution".into());
        let tree = WorkspaceTree::restore("/repo", vec![owner, past, current]).unwrap();
        // When
        let result = nodes_json(&tree);
        // Then
        let past = &result[0]["pastAttempts"][0];
        assert_eq!(past["id"], "past");
        assert_eq!(past["kind"], "node");
        assert_eq!(past["contentKind"], content_kind);
        assert!(past.get("children").is_none());
        assert_eq!(past["pastAttempts"], serde_json::json!([]));
    }
}

fn worktree_row(path: &str, branch: &str, tree: Fetched<WorkspaceTree>) -> WorkspaceListWorktree {
    WorkspaceListWorktree {
        tracking: Fetched::ready(None),
        worktree: Worktree {
            name: branch.to_string(),
            path: path.to_string(),
            branch: branch.to_string(),
            is_main: path == "/repo",
            is_locked: false,
            is_merged: false,
        },
        deleting: false,
        dirty_count: Fetched::ready(0),
        merged: false,
        pull_request: None,
        state_pull_request: None,
        pull_request_error: None,
        pull_request_loaded: true,
        tree,
    }
}

fn list_json(list: &WorkspaceList) -> serde_json::Value {
    from_message(
        "releash.client.v1.WorkspaceListSnapshot",
        &wire::WorkspaceListSnapshot::try_from(list).unwrap(),
    )
    .unwrap()
}

#[test]
fn test_workspaces一覧_持ち主の値と取得の状態を画面が読む名前で並べる() {
    // Given
    let mut main = worktree_row(
        "/repo",
        "main",
        Fetched {
            value: None,
            error: Some(failure("nodes failed")),
        },
    );
    main.pull_request = Some(PrInfo {
        number: 7,
        url: "https://example.com/pr/7".into(),
        state: crate::domain::git_host::PrState::Open,
        draft: false,
    });
    main.state_pull_request = main.pull_request.clone();
    let mut feature = worktree_row(
        "/repo-worktrees/feature",
        "feature",
        Fetched::ready(WorkspaceTree::empty("/repo-worktrees/feature")),
    );
    feature.deleting = true;
    feature.dirty_count = Fetched::ready(3);
    feature.merged = true;
    let list = WorkspaceList {
        repositories: vec![
            WorkspaceListRepository {
                path: "/repo".into(),
                worktrees: Fetched {
                    value: Some(vec![main, feature]),
                    error: Some(failure("scan failed")),
                },
            },
            WorkspaceListRepository {
                path: "/loading".into(),
                worktrees: Fetched::default(),
            },
            WorkspaceListRepository {
                path: "/failed".into(),
                worktrees: Fetched {
                    value: None,
                    error: Some(failure("not a repository")),
                },
            },
            WorkspaceListRepository {
                path: "/empty".into(),
                worktrees: Fetched::ready(Vec::new()),
            },
        ],
    };

    // When
    let json = list_json(&list);

    // Then
    assert_eq!(
        json,
        json!({
            "status": {"state": "ready", "loaded": true, "error": null},
            "repositories": [
                {
                    "path": "/repo",
                    "status": {"state": "refreshFailed", "loaded": true, "error": "scan failed"},
                    "branches": [
                        {
                            "name": "main",
                            "is_main_worktree": true,
                            "is_deleting": false,
                            "worktree_path": "/repo",
                            "dirty_count": 0, "dirty_count_error": null, "pull_request_error": null,
                            "is_merged": false,
                            "has_pr": true,
                            "pr_number": 7,
                            "prState": "open",
                            "prStateNumber": 7,
                            "removalRequiresForce": false,
                            "pr_url": "https://example.com/pr/7"
                        },
                        {
                            "name": "feature",
                            "is_main_worktree": false,
                            "is_deleting": true,
                            "worktree_path": "/repo-worktrees/feature",
                            "dirty_count": 3, "dirty_count_error": null, "pull_request_error": null,
                            "is_merged": true,
                            "has_pr": false,
                            "pr_number": null,
                            "removalRequiresForce": true,
                            "pr_url": null
                        }
                    ],
                    "worktrees": [
                        {
                            "path": "/repo",
                            "status": {
                                "state": "initialFailed",
                                "loaded": false,
                                "error": "nodes failed"
                            },
                            "snapshot": null,
                            "workflowHistory": [],
                            "executions": []
                        },
                        {
                            "path": "/repo-worktrees/feature",
                            "status": {"state": "empty", "loaded": true, "error": null},
                            "snapshot": {"nodes": [], "archivedSessions": []},
                            "workflowHistory": [],
                            "executions": []
                        }
                    ]
                },
                {
                    "path": "/loading",
                    "status": {"state": "loading", "loaded": false, "error": null},
                    "branches": [],
                    "worktrees": []
                },
                {
                    "path": "/failed",
                    "status": {
                        "state": "initialFailed",
                        "loaded": false,
                        "error": "not a repository"
                    },
                    "branches": [],
                    "worktrees": []
                },
                {
                    "path": "/empty",
                    "status": {"state": "empty", "loaded": true, "error": null},
                    "branches": [],
                    "worktrees": []
                }
            ]
        })
    );
    assert_eq!(
        list_json(&WorkspaceList {
            repositories: Vec::new()
        })["status"]["state"],
        "empty"
    );
}

#[test]
fn test_workspaces一覧_読めなくなった実行木は前回の木と失敗を並べて渡す() {
    // Given
    let execution_id = "workflow-execution";
    let tree = WorkspaceTree::restore(
        "/repo",
        vec![
            tree_owner(execution_id),
            child_node(
                "command",
                execution_id,
                execution_id,
                WorkspaceNodeKind::WorkflowCommand,
                "lint",
            ),
        ],
    )
    .unwrap();
    let list = WorkspaceList {
        repositories: vec![WorkspaceListRepository {
            path: "/repo".into(),
            worktrees: Fetched::ready(vec![worktree_row(
                "/repo",
                "main",
                Fetched {
                    value: Some(tree),
                    error: Some(failure("store busy")),
                },
            )]),
        }],
    };

    // When
    let json = list_json(&list);

    // Then
    let worktree = &json["repositories"][0]["worktrees"][0];
    assert_eq!(
        worktree["status"],
        json!({"state": "refreshFailed", "loaded": true, "error": "store busy"})
    );
    assert_eq!(worktree["snapshot"]["nodes"][0]["id"], execution_id);
    assert_eq!(worktree["snapshot"]["preferredNodeId"], execution_id);
}

#[test]
fn test_実行木の変換_archive済みの実行を木から外し履歴とsessionの一覧に載せる() {
    // Given
    let mut archived_workflow = workflow_execution("archived-workflow");
    archived_workflow.workflow_name = "01_author-spec".into();
    archived_workflow.status = ExecutionStatus::Aborted;
    archived_workflow.updated_at = 5.0;
    archived_workflow.archive = Some(ExecutionTreeArchiveRecord {
        execution_id: "archived-workflow".into(),
        archived_at: 12.5,
        archive_reason: "manual".into(),
    });
    let mut archived_session = session_execution("archived-session", "archived-session");
    let session = archived_session.session.as_mut().unwrap();
    session
        .associate_provider_session("provider-session".to_string(), None)
        .unwrap();
    session.archive().unwrap();
    archived_session.archive = Some(ExecutionTreeArchiveRecord {
        execution_id: "archived-session".into(),
        archived_at: 20.0,
        archive_reason: "manual".into(),
    });
    let mut tree = WorkspaceTree::restore(
        "/repo",
        vec![
            tree_owner("archived-workflow"),
            child_node(
                "archived-root",
                "archived-workflow",
                "archived-workflow",
                WorkspaceNodeKind::Sequence,
                "main",
            ),
            tree_owner("live-workflow"),
            child_node(
                "live-root",
                "live-workflow",
                "live-workflow",
                WorkspaceNodeKind::Sequence,
                "main",
            ),
        ],
    )
    .unwrap();
    tree.record_executions(vec![
        archived_workflow,
        archived_session,
        workflow_execution("live-workflow"),
    ]);
    let list = WorkspaceList {
        repositories: vec![WorkspaceListRepository {
            path: "/repo".into(),
            worktrees: Fetched::ready(vec![worktree_row("/repo", "main", Fetched::ready(tree))]),
        }],
    };

    // When
    let json = list_json(&list);

    // Then
    let worktree = &json["repositories"][0]["worktrees"][0];
    assert_eq!(worktree["status"]["state"], "ready");
    assert_eq!(worktree["snapshot"]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(worktree["snapshot"]["nodes"][0]["id"], "live-workflow");
    assert_eq!(
        worktree["workflowHistory"],
        json!([{
            "executionId": "archived-workflow",
            "worktreePath": "/repo",
            "title": "01_author-spec",
            "status": "aborted",
            "updatedAt": 5.0,
            "archivedAt": 12.5,
            "archiveReason": "manual"
        }])
    );
    assert_eq!(
        worktree["snapshot"]["archivedSessions"],
        json!([{
            "id": "archived-session",
            "workspaceIdentity": "/repo",
            "worktreePath": "/repo",
            "workspaceWorktreePath": "/repo",
            "provider": "codex",
            "treeLocation": {
                "treeId": "archived-session",
                "nodeExecutionId": "archived-session"
            },
            "lifecycle": "archived",
            "providerSessionId": "provider-session",
            "transcriptRef": null,
            "operations": {"canArchive": false, "canRestore": true, "canDelete": true},
            "lastExitAbnormal": false
        }])
    );
}

#[test]
fn test_実行木の選択_選択先が木にあるかを木に添える() {
    // Given
    let execution_id = "workflow-execution";
    let tree = WorkspaceTree::restore(
        "/repo",
        vec![
            tree_owner(execution_id),
            child_node(
                "command",
                execution_id,
                execution_id,
                WorkspaceNodeKind::WorkflowCommand,
                "lint",
            ),
        ],
    )
    .unwrap();

    for selected in [true, false] {
        // When
        let json = from_message(
            "releash.client.v1.WorkspaceTreeSelectionSnapshot",
            &selection(&tree, selected).unwrap(),
        )
        .unwrap();

        // Then
        assert_eq!(
            json["reconciliation"],
            json!({"selectionInSnapshot": selected})
        );
        assert_eq!(json["snapshot"], snapshot_json(&tree));
        assert_eq!(json["snapshot"]["nodes"][0]["id"], execution_id);
        assert!(!json.to_string().contains("\"activity\""));
    }
}

#[test]
fn test_ブランチ状態_ブランチ名とworktreeの有無を並べる() {
    // Given
    let branches = vec![
        (Branch::local("main"), true),
        (Branch::local("feature"), false),
    ];

    // When
    let json = from_message(
        "releash.client.v1.ListBranchStatus",
        &branch_status(branches.as_slice()),
    )
    .unwrap();

    // Then
    assert_eq!(
        json,
        json!([
            {"name": "main", "has_worktree": true},
            {"name": "feature", "has_worktree": false}
        ])
    );
}

fn failure(message: &str) -> crate::domain::failure::WorkFailure {
    crate::domain::failure::WorkFailure {
        kind: crate::domain::failure::Failure::Technical(
            crate::domain::failure::TechnicalFailureNature::Other,
        ),
        message: message.into(),
    }
}

#[test]
fn test_pr状態の転送_初回失敗と取得後の失敗を区別する() {
    // Given
    let mut initial = worktree_row("/repo", "main", Fetched::default());
    initial.pull_request_loaded = false;
    initial.pull_request_error = Some(failure("PR denied"));
    let mut retained = worktree_row("/repo-worktrees/feature", "feature", Fetched::default());
    retained.pull_request = Some(PrInfo {
        number: 42,
        url: "https://example.test/pull/42".into(),
        state: crate::domain::git_host::PrState::Open,
        draft: false,
    });
    retained.pull_request_error = Some(failure("PR denied"));
    // When
    let initial = branch(&initial);
    let retained = branch(&retained);
    // Then
    assert_eq!(initial.has_pr, None);
    assert_eq!(initial.pr_number, None);
    assert_eq!(initial.pr_url, None);
    assert_eq!(initial.pull_request_error.as_deref(), Some("PR denied"));
    assert_eq!(retained.has_pr, Some(true));
    assert_eq!(retained.pr_number, Some(42));
    assert_eq!(
        retained.pr_url.as_deref(),
        Some("https://example.test/pull/42")
    );
    assert_eq!(retained.pull_request_error.as_deref(), Some("PR denied"));
}

#[test]
fn test_実行サマリー_providerと行とsessionの状態を配信する() {
    // Given
    let execution = session_execution("session", "session");
    let mut root = tree_owner("session");
    root.background_failure = true;
    let mut active = child_node(
        "active",
        "session",
        "session",
        WorkspaceNodeKind::WorkflowSession,
        "Active",
    );
    active.activity = Some(crate::domain::workflow::AgentSessionActivity::Working);
    active.session_id = Some("active-session".into());
    let mut idle = child_node(
        "idle",
        "session",
        "session",
        WorkspaceNodeKind::WorkflowSession,
        "Idle",
    );
    idle.sibling_order = 1;
    idle.session_id = Some("idle-session".into());
    idle.activity = Some(crate::domain::workflow::AgentSessionActivity::AwaitingInstruction);
    idle.status = WorkspaceNodeStatus::Completed;
    let mut tree = WorkspaceTree::restore("/repo", vec![root, active, idle]).unwrap();
    tree.record_executions(vec![execution]);
    let row = worktree_row("/repo", "main", Fetched::ready(tree));
    // When
    let wire = worktree(&row).unwrap();
    let summary = &wire.executions[0];
    // Then
    assert_eq!(summary.provider.as_deref(), Some("codex"));
    assert_eq!(summary.status, "attention");
    assert_eq!(summary.session_states, ["active", "idle"]);
    assert_eq!(summary.node_count, 2);
    assert!(!summary.is_workflow);
    assert_eq!(wire.aggregate_status.as_deref(), Some("attention"));
}

#[test]
fn test_pr状態の転送_完了済みprは既存のopen用フィールドに含めない() {
    use crate::domain::git_host::PrState;
    // Given
    for (state, expected) in [(PrState::Merged, "merged"), (PrState::Closed, "closed")] {
        let mut row = worktree_row("/repo", "feature", Fetched::default());
        row.state_pull_request = Some(PrInfo {
            number: 42,
            url: "https://example.test/42".into(),
            state,
            draft: false,
        });
        // When
        let branch = branch(&row);
        // Then
        assert_eq!(branch.has_pr, Some(false));
        assert_eq!(branch.pr_number, None);
        assert_eq!(branch.pr_url, None);
        assert_eq!(branch.pr_state.as_deref(), Some(expected));
        assert_eq!(branch.pr_state_number, Some(42));
    }
}
