use crate::domain::workflow::{ExecutionTreeArchiveRecord, ExecutionTreeLaunch};
use crate::domain::workspace_tree::{test_helpers::*, WorkspaceExecution};

fn archived(mut execution: WorkspaceExecution, archived_at: f64) -> WorkspaceExecution {
    execution.archive = Some(ExecutionTreeArchiveRecord {
        execution_id: execution.execution_id.clone(),
        archived_at,
    });
    execution
}

#[test]
fn test_archive履歴_混在を更新時刻順と識別子順に並べ名前で検索する() {
    // Given
    let (nodes, session) = session_execution("session-a", true);
    let mut workflow = archived(execution("exec-a", ExecutionTreeLaunch::Workflow), 99.0);
    workflow.updated_at = 5.0;
    workflow.workflow_name = "Thread workflow".into();
    let tree = tree(
        nodes,
        vec![
            archived(session, 10.0),
            workflow,
            execution("not-archived", ExecutionTreeLaunch::Workflow),
        ],
    );
    // When
    let history = tree.archived_history(None);
    let filtered =
        tree.archived_history(crate::domain::search_term::SearchTerm::new(" THREAD ").as_ref());
    // Then
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id(), "session-a");
    assert_eq!(history[0].ordered_at(), 10.0);
    assert_eq!(history[0].name(), "node-session-a");
    assert_eq!(history[1].ordered_at(), 5.0);
    assert_eq!(history[1].id(), "exec-a");
    assert_eq!(history[1].name(), "Thread workflow");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0], history[1]);
}

#[test]
fn test_archive履歴_同時刻は識別子順で他worktreeと未archiveを除く() {
    // Given
    let (mut nodes, second) = session_execution("b", true);
    let (first_nodes, first) = session_execution("a", true);
    nodes.extend(first_nodes);
    let mut foreign = archived(execution("foreign", ExecutionTreeLaunch::Workflow), 1.0);
    foreign.worktree_path = "/other".into();
    let tree = tree(
        nodes,
        vec![archived(second, 1.0), foreign, archived(first, 1.0)],
    );
    // When
    let history = tree.archived_history(None);
    let filtered =
        tree.archived_history(crate::domain::search_term::SearchTerm::new("node-b").as_ref());
    // Then
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id(), "a");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0], history[1]);
}

#[test]
fn test_archive履歴_sessionの欠損と未archiveを除く() {
    // Given
    let (nodes, active) = session_execution("active", false);
    let (_, missing_node) = session_execution("missing-node", true);
    let tree = tree(
        nodes,
        vec![
            archived(active, 1.0),
            archived(missing_node, 1.0),
            archived(
                execution("missing-session", ExecutionTreeLaunch::Session),
                1.0,
            ),
        ],
    );
    // When / Then
    assert!(tree.archived_history(None).is_empty());
}

#[test]
fn test_archive履歴の値_sessionとworkflowの識別子と名前と時刻を返す() {
    // Given
    let (_, execution) = session_execution("session-id", true);
    let session = super::ArchivedExecution::Session {
        session: execution.session.clone().unwrap(),
        name: "Session title".into(),
        archived_at: 42.5,
    };
    let workflow = super::ArchivedExecution::Workflow(super::WorkspaceExecution {
        execution_id: "workflow-id".into(),
        workflow_name: "Workflow title".into(),
        updated_at: 12.5,
        ..execution
    });
    // When / Then
    assert_eq!(
        (session.id(), session.name(), session.ordered_at()),
        ("session-id", "Session title", 42.5)
    );
    assert_eq!(
        (workflow.id(), workflow.name(), workflow.ordered_at()),
        ("workflow-id", "Workflow title", 12.5)
    );
}
