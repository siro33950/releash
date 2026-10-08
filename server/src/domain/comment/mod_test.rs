use super::test_helpers::*;
use super::*;

#[test]
fn test_review停止_技術的な失敗の値とメッセージを保持する() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    // Given
    for nature in [
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "technical failure".into(),
        };
        // When
        let error = ReviewError::Technical(failure.clone());
        // Then
        assert_eq!(error.to_string(), "technical failure");
        assert!(matches!(error, ReviewError::Technical(value) if value == failure));
    }
}
pub(crate) mod tests {
    use super::*;

    #[test]
    fn test_provider_tui_actor_does_not_fabricate_a_model_identity() {
        let actor =
            ReviewActor::provider_agent("claude".to_string(), Some("session-1".to_string()));

        assert_eq!(actor.backend_id.as_deref(), Some("claude"));
        assert_eq!(actor.model, None);
        assert_eq!(actor.session_id.as_deref(), Some("session-1"));
        assert_eq!(actor.display_name, "claude");
        assert_eq!(actor.participant_key(), "agent:claude:");
    }

    fn target(file_path: Option<&str>) -> ReviewTarget {
        ReviewTarget {
            file_path: file_path.map(str::to_string),
            line_number: Some(1),
            end_line: None,
        }
    }

    fn created(
        thread_id: &str,
        actor: ReviewActor,
        file_path: Option<&str>,
        at: f64,
    ) -> ReviewEvent {
        ReviewEvent::ThreadCreated {
            event_id: format!("event-{thread_id}"),
            thread_id: thread_id.to_string(),
            comment_id: format!("comment-{thread_id}"),
            actor,
            target: target(file_path),
            content: format!("content-{thread_id}"),
            at,
        }
    }

    #[test]
    fn validates_content_target_and_filter_inputs() {
        assert!(matches!(
            validate_content(" ", "content"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_content("has\0nul", "content"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_content(&"a".repeat(MAX_REVIEW_TEXT_BYTES + 1), "content"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_line_range(None, Some(5)),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_filter(&Some(ReviewThreadFilter {
                thread_id: vec![" ".to_string()],
                ..Default::default()
            })),
            Err(ReviewError::InvalidInput(_))
        ));
    }

    #[test]
    fn validates_review_target_path_boundaries() {
        let absolute_path = if cfg!(windows) {
            "C:/repo/src/main.rs"
        } else {
            "/repo/src/main.rs"
        };
        let long_path = "a".repeat(MAX_REVIEW_TARGET_PATH_BYTES + 1);
        let invalid_paths = [
            absolute_path.to_string(),
            "src\\main.rs".to_string(),
            "src/main.rs\0".to_string(),
            long_path,
            "./src/main.rs".to_string(),
            "../outside".to_string(),
            "path/../traversal".to_string(),
        ];

        for file_path in invalid_paths {
            assert!(
                matches!(
                    validate_review_file_path(&file_path),
                    Err(ReviewError::InvalidInput(_))
                ),
                "expected invalid file_path: {file_path:?}"
            );
        }
    }

    #[test]
    fn validates_review_target_line_range_boundaries() {
        let invalid_ranges = [(Some(0), None), (Some(5), Some(4))];

        for (line_number, end_line) in invalid_ranges {
            assert!(
                matches!(
                    validate_line_range(line_number, end_line),
                    Err(ReviewError::InvalidInput(_))
                ),
                "expected invalid range: {line_number:?}..{end_line:?}"
            );
        }
    }

    #[test]
    fn validates_existing_target_and_filter_regressions() {
        assert!(matches!(
            validate_review_file_path("../outside"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_review_file_path("path/../traversal"),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_line_range(None, Some(5)),
            Err(ReviewError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_filter(&Some(ReviewThreadFilter {
                thread_id: vec![" ".to_string()],
                ..Default::default()
            })),
            Err(ReviewError::InvalidInput(_))
        ));
    }

    #[test]
    fn projects_threads_sorted_by_updated_at_and_excludes_deleted() {
        let viewer = ReviewActor::human();
        let other = agent("codex", "gpt-5");
        let events = vec![
            created("old", viewer.clone(), Some("src/a.rs"), 1.0),
            created("new", other.clone(), Some("src/b.rs"), 2.0),
            ReviewEvent::CommentAppended {
                event_id: "event-old-2".to_string(),
                thread_id: "old".to_string(),
                comment_id: "comment-old-2".to_string(),
                actor: other,
                content: "update".to_string(),
                at: 3.0,
            },
            created("deleted", viewer.clone(), None, 4.0),
            ReviewEvent::ThreadDeleted {
                event_id: "event-delete".to_string(),
                thread_id: "deleted".to_string(),
                actor: viewer,
                at: 5.0,
            },
        ];

        let threads = project_threads("wt", &events);

        assert_eq!(
            threads
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>(),
            vec!["old", "new"]
        );
    }

    #[test]
    fn filter_combines_axes_and_unread_ignores_resolve() {
        let viewer = agent("codex", "gpt-5");
        let other = ReviewActor::human();
        let events = vec![
            created("mine", viewer.clone(), Some("src/a.rs"), 1.0),
            created("other", other.clone(), Some("src/a.rs"), 2.0),
            ReviewEvent::CommentAppended {
                event_id: "event-other-comment".to_string(),
                thread_id: "other".to_string(),
                comment_id: "comment-other-2".to_string(),
                actor: viewer.clone(),
                content: "viewer last".to_string(),
                at: 3.0,
            },
            ReviewEvent::ThreadResolved {
                event_id: "event-resolve".to_string(),
                thread_id: "other".to_string(),
                actor: other,
                outcome: "accepted".to_string(),
                summary: "done".to_string(),
                at: 4.0,
            },
        ];
        let threads = project_threads("wt", &events);

        let filtered = apply_filter(
            threads,
            Some(ReviewThreadFilter {
                file: Some("src/a.rs".to_string()),
                state: Some(ReviewThreadState::Resolved),
                author: Some(AuthorScope::Other),
                unread: Some(false),
                thread_id: vec!["other".to_string()],
            }),
            &viewer,
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "other");
    }
}

#[test]
fn test_review絞り込み値_正規の値だけ解釈する() {
    // Given / When / Then
    assert_eq!(
        ReviewThreadState::parse("open").unwrap(),
        ReviewThreadState::Open
    );
    assert_eq!(
        ReviewThreadState::parse("resolved").unwrap(),
        ReviewThreadState::Resolved
    );
    assert_eq!(AuthorScope::parse("self").unwrap(), AuthorScope::Mine);
    assert_eq!(AuthorScope::parse("other").unwrap(), AuthorScope::Other);
    assert!(ReviewThreadState::parse("OPEN").is_err());
    assert!(AuthorScope::parse("human").is_err());
    assert!(ReviewThreadFilter::parse_unread("true").unwrap());
    assert!(!ReviewThreadFilter::parse_unread("false").unwrap());
    assert!(ReviewThreadFilter::parse_unread("1").is_err());
}

#[test]
fn test_review書き手_openなsessionだけ変更を許す() {
    use crate::domain::agent_session::aggregates::AgentSessionLifecycle as L;
    // Given / When / Then
    assert!(ensure_session_can_review(L::Open, "session").is_ok());
    for state in [L::Paused, L::Archived] {
        assert!(matches!(
            ensure_session_can_review(state, "session"),
            Err(ReviewError::SessionNotOpen(_))
        ));
    }
}
