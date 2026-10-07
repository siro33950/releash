use super::*;

#[test]
fn test_workflow設定入力_転送値の真偽と欠落を入力データへ写す() {
    // Given
    let values = [(Some(true), true), (Some(false), false), (None, false)];

    // When
    for (wire_value, expected) in values {
        let input =
            crate::usecase::app_config::WorkflowConfigInput::try_from(wire::WorkflowSection {
                approval_auto_approve: wire_value.map(|value| value.try_into().unwrap()),
            })
            .unwrap();
        // Then
        assert_eq!(input.approval_auto_approve, expected);
    }
}

#[test]
fn test_review転送_解決済みthreadの全項目を既存のjsonの形で保持する() {
    use crate::domain::comment::{
        ReviewActor, ReviewComment, ReviewResolveInfo, ReviewTarget, ReviewThread,
        ReviewThreadState,
    };
    // Given
    let actor =
        ReviewActor::provider_agent("codex".into(), Some("session".into())).redacted_for_public();
    let thread = ReviewThread {
        id: "thread".into(),
        worktree_name: "/repo".into(),
        author: actor.clone(),
        target: ReviewTarget {
            file_path: Some("src/main.rs".into()),
            line_number: Some(3),
            end_line: Some(5),
        },
        state: ReviewThreadState::Resolved,
        comments: vec![ReviewComment {
            id: "comment".into(),
            thread_id: "thread".into(),
            author: actor.clone(),
            content: "content".into(),
            created_at: 1.0,
        }],
        resolve: Some(ReviewResolveInfo {
            actor,
            outcome: "fixed".into(),
            summary: "summary".into(),
            resolved_at: 2.0,
        }),
        created_at: 1.0,
        updated_at: 2.0,
        version: 3,
        can_resolve: false,
    };
    // When
    let message = wire::ReviewThreadDto::try_from(thread).unwrap();
    let json = wire::from_message("releash.client.v1.ReviewThreadDto", &message).unwrap();
    // Then
    let actor = serde_json::json!({"kind":"agent", "backendId":"codex", "model":null, "displayName":"codex"});
    assert_eq!(
        json,
        serde_json::json!({
            "id":"thread", "worktreeName":"/repo", "author":actor,
            "target":{"filePath":"src/main.rs", "lineNumber":3, "endLine":5}, "state":"resolved",
            "comments":[{"id":"comment", "threadId":"thread", "author":actor, "content":"content", "createdAt":1.0}],
            "resolve":{"actor":actor, "outcome":"fixed", "summary":"summary", "resolvedAt":2.0},
            "createdAt":1.0, "updatedAt":2.0, "version":3, "canResolve":false,
        })
    );
}
