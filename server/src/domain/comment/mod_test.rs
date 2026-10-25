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
        let error = Error::Technical(failure.clone());
        // Then
        assert_eq!(error.to_string(), "technical failure");
        assert!(matches!(error, Error::Technical(value) if value == failure));
    }
}
pub(crate) mod tests {
    use super::*;

    #[test]
    fn test_provider_tui_actor_does_not_fabricate_a_model_identity() {
        let actor = Author::provider_agent("claude".to_string(), Some("session-1".to_string()));

        assert_eq!(actor.backend_id.as_deref(), Some("claude"));
        assert_eq!(actor.model, None);
        assert_eq!(actor.session_id.as_deref(), Some("session-1"));
        assert_eq!(actor.display_name, "claude");
        assert_eq!(actor.participant_key(), "agent:claude:");
    }

    fn target(file_path: Option<&str>) -> Target {
        Target {
            file_path: file_path.map(str::to_string),
            line_number: Some(1),
            end_line: None,
            side: None,
            end_side: None,
        }
    }

    fn created(thread_id: &str, actor: Author, file_path: Option<&str>, at: f64) -> ReviewEvent {
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
    fn test_入力検証_不正な本文と行範囲と絞り込みを拒む() {
        // Given / When / Then
        assert!(matches!(
            validate_content(" ", "content"),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            validate_content("has\0nul", "content"),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            validate_content(&"a".repeat(MAX_REVIEW_TEXT_BYTES + 1), "content"),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            Target {
                line_number: None,
                end_line: Some(5),
                ..target(None)
            }
            .validate(),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            validate_filter(&Some(ReviewThreadFilter {
                thread_id: vec![" ".to_string()],
                ..Default::default()
            })),
            Err(Error::InvalidInput(_))
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
                    target(Some(&file_path)).validate(),
                    Err(Error::InvalidInput(_))
                ),
                "expected invalid file_path: {file_path:?}"
            );
        }
    }

    #[test]
    fn test_対象行範囲_正の行番号と終端の順序を検証する() {
        // Given / When / Then
        let invalid_ranges = [(Some(0), None), (Some(1), Some(0)), (Some(5), Some(4))];

        for (line_number, end_line) in invalid_ranges {
            assert!(
                matches!(
                    Target {
                        line_number,
                        end_line,
                        ..target(Some("file.rs"))
                    }
                    .validate(),
                    Err(Error::InvalidInput(_))
                ),
                "expected invalid range: {line_number:?}..{end_line:?}"
            );
        }
        for (line_number, end_line) in [
            (None, None),
            (Some(1), None),
            (Some(1), Some(1)),
            (Some(1), Some(2)),
        ] {
            Target {
                line_number,
                end_line,
                ..target(Some("file.rs"))
            }
            .validate()
            .unwrap();
        }
        Target {
            line_number: None,
            ..target(None)
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn test_対象と絞り込み検証_相対パス逸脱と開始行欠落と空idを拒む() {
        // Given / When / Then
        assert!(matches!(
            validate_review_file_path("../outside"),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            validate_review_file_path("path/../traversal"),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            Target {
                line_number: None,
                end_line: Some(5),
                ..target(None)
            }
            .validate(),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            validate_filter(&Some(ReviewThreadFilter {
                thread_id: vec![" ".to_string()],
                ..Default::default()
            })),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn projects_threads_sorted_by_updated_at_and_excludes_deleted() {
        let viewer = Author::human();
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
    fn test_未解決件数_解決済みを除き解決後もthreadを保持する() {
        // Given
        let mut events = vec![
            created("one", Author::human(), Some("file"), 1.0),
            created("two", Author::human(), Some("file"), 2.0),
            created("done", Author::human(), Some("file"), 3.0),
        ];
        let resolve = |id: &str| ReviewEvent::ThreadResolved {
            event_id: format!("resolve-{id}"),
            thread_id: id.into(),
            actor: Author::human(),
            outcome: "resolved".into(),
            summary: "done".into(),
            at: 4.0,
        };
        events.push(resolve("done"));
        let before = project_threads("wt", &events);
        // When
        events.push(resolve("one"));
        let after = project_threads("wt", &events);
        // Then
        assert_eq!(unresolved_thread_count(&before), 2);
        assert_eq!(unresolved_thread_count(&after), 1);
        assert_eq!(before.len(), after.len());
    }

    #[test]
    fn filter_combines_axes_and_unread_ignores_resolve() {
        let viewer = agent("codex", "gpt-5");
        let other = Author::human();
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
                state: Some(State::Resolved),
                author: Some(AuthorScope::Other),
                unread: Some(false),
                thread_id: vec!["other".to_string()],
            }),
            &viewer,
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "other");
    }

    #[test]
    fn test_thread復元_最初のcommentと返信と解決結果を保持する() {
        // Given
        let author = Author::provider_agent("codex".into(), Some("session".into()));
        let events = vec![
            created("one", author.clone(), Some("src/a.rs"), 1.0),
            ReviewEvent::CommentAppended {
                event_id: "reply".into(),
                thread_id: "one".into(),
                comment_id: "reply-comment".into(),
                actor: Author::human(),
                content: "返信".into(),
                at: 2.0,
            },
            ReviewEvent::ThreadResolved {
                event_id: "resolve".into(),
                thread_id: "one".into(),
                actor: Author::human(),
                outcome: "accepted".into(),
                summary: "完了".into(),
                at: 3.0,
            },
        ];
        // When
        let thread = project_thread("wt", "one", &events).unwrap();
        // Then
        assert_eq!(thread.id, "one");
        assert_eq!(thread.worktree_name, "wt");
        assert_eq!(thread.target, target(Some("src/a.rs")));
        assert_eq!(thread.state, State::Resolved);
        assert!(!thread.can_resolve);
        assert_eq!(thread.created_at, 1.0);
        assert_eq!(thread.updated_at, 3.0);
        assert_eq!(thread.version, 3);
        assert_eq!(
            thread.comments,
            vec![
                Comment {
                    id: "comment-one".into(),
                    thread_id: "one".into(),
                    author,
                    content: "content-one".into(),
                    created_at: 1.0,
                },
                Comment {
                    id: "reply-comment".into(),
                    thread_id: "one".into(),
                    author: Author::human(),
                    content: "返信".into(),
                    created_at: 2.0,
                },
            ]
        );
        assert_eq!(
            thread.resolve,
            Some(Resolve {
                actor: Author::human(),
                outcome: "accepted".into(),
                summary: "完了".into(),
                resolved_at: 3.0,
            })
        );
    }

    #[test]
    fn test_thread復元_作成のない履歴からはthreadを復元しない() {
        // Given
        let reply = ReviewEvent::CommentAppended {
            event_id: "reply".into(),
            thread_id: "one".into(),
            comment_id: "comment".into(),
            actor: Author::human(),
            content: "返信".into(),
            at: 1.0,
        };
        // When / Then
        assert!(project_thread("wt", "one", &[]).is_none());
        assert!(project_thread("wt", "one", &[reply]).is_none());
    }

    #[test]
    fn test_thread絞り込み_返信者ではなく最初のcommentの作者で判定する() {
        // Given
        let viewer = Author::provider_agent("codex".into(), Some("viewer".into()));
        let events = vec![
            created(
                "mine",
                Author::provider_agent("codex".into(), Some("creator".into())),
                Some("a.rs"),
                1.0,
            ),
            created("other", Author::human(), Some("b.rs"), 2.0),
            ReviewEvent::CommentAppended {
                event_id: "reply".into(),
                thread_id: "other".into(),
                comment_id: "reply-comment".into(),
                actor: viewer.clone(),
                content: "返信".into(),
                at: 3.0,
            },
        ];
        let threads = project_threads("wt", &events);
        for (filter, expected) in [
            (None, vec!["other", "mine"]),
            (
                Some(ReviewThreadFilter {
                    author: Some(AuthorScope::Mine),
                    ..Default::default()
                }),
                vec!["mine"],
            ),
            (
                Some(ReviewThreadFilter {
                    author: Some(AuthorScope::Other),
                    ..Default::default()
                }),
                vec!["other"],
            ),
            (
                Some(ReviewThreadFilter {
                    file: Some("absent.rs".into()),
                    ..Default::default()
                }),
                vec![],
            ),
            (
                Some(ReviewThreadFilter {
                    state: Some(State::Resolved),
                    ..Default::default()
                }),
                vec![],
            ),
            (
                Some(ReviewThreadFilter {
                    unread: Some(true),
                    ..Default::default()
                }),
                vec![],
            ),
            (
                Some(ReviewThreadFilter {
                    thread_id: vec!["missing".into()],
                    ..Default::default()
                }),
                vec![],
            ),
        ] {
            // When
            let actual = apply_filter(threads.clone(), filter, &viewer);
            // Then
            assert_eq!(
                actual
                    .iter()
                    .map(|thread| thread.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn test_thread履歴_全種類の事実を公開用の作者とともに保持する() {
        // Given
        let actor = Author {
            kind: AuthorKind::Agent,
            backend_id: Some("codex".into()),
            model: Some("model".into()),
            session_id: Some("private-session".into()),
            display_name: "Codex".into(),
        };
        let public_actor = ReviewActorDto {
            kind: AuthorKind::Agent,
            backend_id: Some("codex".into()),
            model: Some("model".into()),
            display_name: "Codex".into(),
        };
        let events = [
            created("one", actor.clone(), Some("a.rs"), 1.0),
            ReviewEvent::CommentAppended {
                event_id: "reply".into(),
                thread_id: "one".into(),
                comment_id: "reply-comment".into(),
                actor: actor.clone(),
                content: "返信".into(),
                at: 2.0,
            },
            ReviewEvent::ThreadResolved {
                event_id: "resolve".into(),
                thread_id: "one".into(),
                actor: actor.clone(),
                outcome: "accepted".into(),
                summary: "完了".into(),
                at: 3.0,
            },
            ReviewEvent::ThreadDeleted {
                event_id: "delete".into(),
                thread_id: "one".into(),
                actor,
                at: 4.0,
            },
        ];
        // When
        let history = events
            .iter()
            .map(ReviewHistoryEntry::from)
            .collect::<Vec<_>>();
        // Then
        assert_eq!(
            history,
            vec![
                ReviewHistoryEntry::ThreadCreated {
                    id: "event-one".into(),
                    thread_id: "one".into(),
                    comment_id: "comment-one".into(),
                    actor: public_actor.clone(),
                    target: target(Some("a.rs")),
                    content: "content-one".into(),
                    at: 1.0,
                },
                ReviewHistoryEntry::CommentAppended {
                    id: "reply".into(),
                    thread_id: "one".into(),
                    comment_id: "reply-comment".into(),
                    actor: public_actor.clone(),
                    content: "返信".into(),
                    at: 2.0,
                },
                ReviewHistoryEntry::ThreadResolved {
                    id: "resolve".into(),
                    thread_id: "one".into(),
                    actor: public_actor.clone(),
                    outcome: "accepted".into(),
                    summary: "完了".into(),
                    at: 3.0,
                },
                ReviewHistoryEntry::ThreadDeleted {
                    id: "delete".into(),
                    thread_id: "one".into(),
                    actor: public_actor,
                    at: 4.0,
                },
            ]
        );
    }
}

#[test]
fn test_review絞り込み値_正規の値だけ解釈する() {
    // Given / When / Then
    assert_eq!(State::parse("open").unwrap(), State::Open);
    assert_eq!(State::parse("resolved").unwrap(), State::Resolved);
    assert_eq!(AuthorScope::parse("self").unwrap(), AuthorScope::Mine);
    assert_eq!(AuthorScope::parse("other").unwrap(), AuthorScope::Other);
    assert!(State::parse("OPEN").is_err());
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
            Err(Error::SessionNotOpen(_))
        ));
    }
}

#[test]
fn test_行の側_行とファイルを必須にして終端の側も検証する() {
    // Given
    let mut target = Target {
        file_path: Some("file.rs".into()),
        line_number: Some(2),
        end_line: Some(3),
        side: Some(LineSide::Deletions),
        end_side: Some(LineSide::Additions),
    };
    // When / Then
    target.validate().unwrap();
    target.side = None;
    assert!(matches!(target.validate(), Err(Error::InvalidInput(_))));
    target.side = Some(LineSide::Deletions);
    target.end_line = None;
    assert!(matches!(target.validate(), Err(Error::InvalidInput(_))));
    target.end_side = None;
    target.file_path = None;
    assert!(matches!(target.validate(), Err(Error::InvalidInput(_))));
    target.file_path = Some("file.rs".into());
    target.line_number = None;
    assert!(matches!(target.validate(), Err(Error::InvalidInput(_))));
}
