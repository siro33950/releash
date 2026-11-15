use super::*;
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
    assert!(matches!(&history[0], ArchivedHistoryEntry::Session(s)
        if s.agent_session_id == "session-a" && s.archived_at == 10.0 && s.name == "node-session-a" && s.can_delete));
    assert_eq!(filtered, history[1..]);
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
    assert!(
        matches!(&history[0], ArchivedHistoryEntry::Session(value) if value.agent_session_id == "a")
    );
    assert_eq!(filtered, history[1..]);
}
