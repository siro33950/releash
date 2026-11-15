use super::*;
use crate::domain::agent_session::aggregates::AgentSession;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::{test_helpers::*, WorkspaceNodeStatus};

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

#[test]
fn test_画面に出す木_実行を代表する節は実行の識別子と題で公開し子を並び順で返す() {
    // Given
    let mut nodes = workflow_nodes("exec-a");
    nodes.reverse();
    let tree = tree(
        nodes,
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
fn test_artifactの数_この節と過去の試行のうちartifactを持つものを数える() {
    // Given
    let cases = [
        (vec!["exec-a-fix-1", "exec-a-fix-2"], 2),
        (vec!["exec-a-fix-2"], 1),
        (vec!["exec-a-fix-1"], 1),
        (vec![], 0),
    ];
    for (with_artifact, expected) in cases {
        let mut nodes = workflow_nodes("exec-a");
        for node in &mut nodes {
            node.has_artifact = with_artifact.contains(&node.id.as_str());
        }
        let tree = tree(
            nodes,
            vec![execution("exec-a", ExecutionTreeLaunch::Workflow)],
        );
        // When
        let visible = tree.visible();
        let roots = visible.roots();
        let counts: Vec<_> = roots[0]
            .children()
            .iter()
            .map(|child| (child.id().to_string(), child.artifact_count()))
            .collect();
        // Then
        assert_eq!(
            counts,
            [
                ("exec-a-review".to_string(), 0),
                ("exec-a-fix-2".to_string(), expected)
            ],
            "{with_artifact:?}"
        );
    }
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
    assert!(tree.archived_history(None).is_empty());
}

#[test]
fn test_provider_単独sessionの根はsessionを採りworkflowの節は定義の値を採る() {
    // Given
    let (mut nodes, session) = session_execution("session", false);
    nodes[1].provider = Some(ProviderKind::Claude);
    let mut workflow = workflow_nodes("workflow");
    workflow[2].provider = Some(ProviderKind::Claude);
    nodes.extend(workflow);
    let tree = tree(
        nodes,
        vec![
            session,
            execution("workflow", ExecutionTreeLaunch::Workflow),
        ],
    );

    // When
    let visible = tree.visible();
    let roots = visible.roots();

    // Then
    let session = roots.iter().find(|root| root.id() == "session").unwrap();
    assert_eq!(session.provider(), Some(ProviderKind::Codex));
    let workflow = roots.iter().find(|root| root.id() == "workflow").unwrap();
    assert_eq!(
        workflow.children()[0].provider(),
        Some(ProviderKind::Claude)
    );
    assert_eq!(workflow.children()[1].provider(), None);
}
