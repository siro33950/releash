use crate::adaptor::gateway::workflow::event::WorkflowEvent;
#[cfg(test)]
use crate::domain::comment::{
    ReviewActor, ReviewComment, ReviewHistoryEntry, ReviewResolveInfo, ReviewTarget, ReviewThread,
    ReviewThreadState,
};
use crate::domain::workflow::WorkflowExecutionSummary as WorkflowExecutionMetadata;
use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage};

pub fn make_execution(
    execution_id: &str,
    worktree: &str,
    status: ExecutionStatus,
    started_at: f64,
) -> WorkflowExecutionMetadata {
    WorkflowExecutionMetadata {
        execution_id: execution_id.to_string(),
        workflow_name: "wf".to_string(),
        status,
        worktree_path: worktree.to_string(),
        current_node: None,
        created_from: ExecutionOrigin::Cli,
        started_at,
        updated_at: started_at,
        completed_at: if status.is_finished() {
            Some(started_at + 1.0)
        } else {
            None
        },
        error_reason: None,
        total_token_usage: TokenUsage::default(),
    }
}

pub fn test_uuid(seed: u8) -> String {
    uuid::Uuid::from_bytes([seed; 16]).to_string()
}

#[cfg(test)]
pub(in crate::cli) fn review_cli_thread(state: ReviewThreadState) -> ReviewThread {
    let thread_id = test_uuid(42);
    let author = ReviewActor::provider_agent("codex".to_string(), None).redacted_for_public();
    let resolver = ReviewActor::human().redacted_for_public();
    ReviewThread {
        id: thread_id.clone(),
        worktree_name: "/repo".to_string(),
        author: author.clone(),
        target: ReviewTarget {
            file_path: Some("src/main.rs".to_string()),
            line_number: Some(3),
            end_line: Some(5),
        },
        state: state.clone(),
        comments: vec![ReviewComment {
            id: test_uuid(43),
            thread_id,
            author,
            content: "Claim".to_string(),
            created_at: 10.0,
        }],
        resolve: (state == ReviewThreadState::Resolved).then_some(ReviewResolveInfo {
            actor: resolver,
            outcome: "accepted".to_string(),
            summary: "done".to_string(),
            resolved_at: 20.0,
        }),
        created_at: 10.0,
        updated_at: 20.0,
        version: 2,
        can_resolve: state == ReviewThreadState::Open,
    }
}

#[cfg(test)]
pub(in crate::cli) fn review_history_entries() -> Vec<ReviewHistoryEntry> {
    let thread = review_cli_thread(ReviewThreadState::Open);
    vec![
        ReviewHistoryEntry::ThreadCreated {
            id: test_uuid(50),
            thread_id: thread.id.clone(),
            comment_id: test_uuid(51),
            actor: thread.author.clone(),
            target: thread.target.clone(),
            content: "Claim".to_string(),
            at: 10.0,
        },
        ReviewHistoryEntry::ThreadResolved {
            id: test_uuid(52),
            thread_id: thread.id,
            actor: ReviewActor::human().redacted_for_public(),
            outcome: "accepted".to_string(),
            summary: "done".to_string(),
            at: 20.0,
        },
    ]
}

pub fn execution_started_event(
    execution_id: &str,
    workflow_name: &str,
    worktree: &str,
) -> WorkflowEvent {
    WorkflowEvent::ExecutionStarted {
        repository_root: None,
        execution_id: execution_id.to_string(),
        workflow_name: workflow_name.to_string(),
        worktree_path: worktree.to_string(),
        created_from: ExecutionOrigin::Cli,
        request: String::new(),
        definition: crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml {
            name: workflow_name.to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![crate::adaptor::gateway::workflow::schema::NodeDefinition {
                name: "main".to_string(),
                kind: crate::adaptor::gateway::workflow::schema::NodeKind::Session(
                    crate::adaptor::gateway::workflow::schema::SessionSpec::default(),
                ),
                ..Default::default()
            }],
            entry: "main".to_string(),
        },
        timestamp: 100.0,
    }
}

pub fn root_node_started_event(
    execution_id: &str,
    node_execution_id: &str,
    node_name: &str,
    timestamp: f64,
) -> WorkflowEvent {
    WorkflowEvent::NodeStarted {
        worktree: None,
        execution_id: execution_id.to_string(),
        node_execution_id: node_execution_id.to_string(),
        node_name: node_name.to_string(),
        kind: crate::domain::workflow::NodeKindName::Session,
        attempt: 1,
        parent: None,
        timestamp,
    }
}
