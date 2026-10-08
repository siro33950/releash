use super::*;
use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage};

fn candidate(path: &str, status: ExecutionStatus) -> WorkflowExecutionSummary {
    WorkflowExecutionSummary {
        execution_id: "blocking-execution".into(),
        workflow_name: "running-workflow".into(),
        worktree_path: path.into(),
        status,
        current_node: None,
        created_from: ExecutionOrigin::Cli,
        started_at: 1.0,
        updated_at: 1.0,
        completed_at: None,
        error_reason: None,
        total_token_usage: TokenUsage::default(),
    }
}

#[test]
fn test_worktree排他_同じworktreeの実行中だけを拒否し占有実行を返す() {
    // Given
    for (path, status, blocked) in [
        ("/repo", ExecutionStatus::Running, true),
        ("/other", ExecutionStatus::Running, false),
        ("/repo", ExecutionStatus::Completed, false),
        ("/repo", ExecutionStatus::Aborted, false),
    ] {
        let candidates = [candidate(path, status)];
        // When
        let result = validate_worktree_start("/repo", &candidates);
        // Then
        if blocked {
            assert_eq!(
                result.unwrap_err(),
                WorktreeActiveExecution {
                    worktree_path: "/repo".into(),
                    execution_id: "blocking-execution".into(),
                    workflow_name: "running-workflow".into(),
                }
            );
        } else {
            assert!(result.is_ok());
        }
    }
    assert!(validate_worktree_start("/repo", &[]).is_ok());
}
