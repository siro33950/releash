use releash_lib::test_support::integration::fixtures::fixtures_domain_comment_mod_agent as agent;
pub(crate) mod tests {
    use super::*;

    use parking_lot::Mutex;
    use releash_lib::test_support::integration::platform::AuthorScope;
    use releash_lib::test_support::integration::platform::ReviewActor;
    use releash_lib::test_support::integration::platform::ReviewActorKind;
    use releash_lib::test_support::integration::platform::ReviewClock;
    use releash_lib::test_support::integration::platform::ReviewCommentUsecase;
    use releash_lib::test_support::integration::platform::ReviewError;
    use releash_lib::test_support::integration::platform::ReviewEvent;
    use releash_lib::test_support::integration::platform::ReviewEventMutation;
    use releash_lib::test_support::integration::platform::ReviewEventStore;
    use releash_lib::test_support::integration::platform::ReviewHistoryEntry;
    use releash_lib::test_support::integration::platform::ReviewIdGenerator;
    use releash_lib::test_support::integration::platform::ReviewTarget;
    use releash_lib::test_support::integration::platform::ReviewThreadFilter;
    use releash_lib::test_support::integration::platform::ReviewThreadState;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[derive(Default)]
    struct FakeStore {
        events: Mutex<Vec<ReviewEvent>>,
    }

    impl ReviewEventStore for FakeStore {
        fn load(
            &self,
            _app_data_dir: &Path,
            _worktree_name: &str,
        ) -> Result<Vec<ReviewEvent>, ReviewError> {
            Ok(self.events.lock().clone())
        }

        fn mutate(
            &self,
            _app_data_dir: &Path,
            _worktree_name: &str,
            mutation: ReviewEventMutation<'_>,
        ) -> Result<Vec<ReviewEvent>, ReviewError> {
            let mut events = self.events.lock();
            let appended = mutation(&events)?;
            events.extend(appended);
            Ok(events.clone())
        }
    }

    #[derive(Default)]
    struct SequentialClock {
        next: Mutex<u32>,
    }

    impl ReviewClock for SequentialClock {
        fn now(&self) -> f64 {
            let mut next = self.next.lock();
            *next += 1;
            f64::from(*next)
        }
    }

    #[derive(Default)]
    struct SequentialIds {
        next: Mutex<u32>,
    }

    impl ReviewIdGenerator for SequentialIds {
        fn event_id(&self) -> String {
            let mut next = self.next.lock();
            *next += 1;
            format!("id-{next}")
        }
    }

    fn usecase() -> ReviewCommentUsecase {
        ReviewCommentUsecase::new(
            Arc::new(FakeStore::default()),
            Arc::new(SequentialClock::default()),
            Arc::new(SequentialIds::default()),
        )
    }

    #[test]
    pub fn test_comment変更_成功した操作だけ購読口へ通知する() {
        use releash_lib::test_support::integration::subscriptions::StateChangeSource;
        // Given
        let subscriptions =
            releash_lib::test_support::integration::subscriptions::test_subscriptions();
        let mut changes = subscriptions.changes();
        let usecase = usecase().with_subscriptions(subscriptions);
        let dir = TempDir::new().unwrap();
        // When
        let thread = usecase
            .create_thread(
                dir.path(),
                "repository",
                ReviewActor::human(),
                target(),
                "first".into(),
            )
            .unwrap();
        usecase
            .append_comment(
                dir.path(),
                "repository",
                ReviewActor::human(),
                &thread.id,
                "second".into(),
            )
            .unwrap();
        usecase
            .resolve_thread(
                dir.path(),
                "repository",
                ReviewActor::human(),
                &thread.id,
                "done".into(),
                "summary".into(),
            )
            .unwrap();
        assert!(usecase
            .append_comment(
                dir.path(),
                "repository",
                ReviewActor::human(),
                &thread.id,
                "late".into()
            )
            .is_err());
        usecase
            .delete_thread(dir.path(), "repository", ReviewActor::human(), &thread.id)
            .unwrap();
        // Then
        for _ in 0..4 {
            assert_eq!(
                changes.try_recv().unwrap(),
                StateChangeSource::ReviewComments(Some("repository".into()))
            );
        }
        assert!(changes.try_recv().is_err());
    }

    fn target() -> ReviewTarget {
        ReviewTarget {
            file_path: None,
            line_number: None,
            end_line: None,
        }
    }

    fn file_target(file_path: &str, line_number: u32) -> ReviewTarget {
        ReviewTarget {
            file_path: Some(file_path.to_string()),
            line_number: Some(line_number),
            end_line: None,
        }
    }

    #[test]
    pub fn create_append_resolve_history_and_handoff_use_storage_port() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();

        let thread = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Claim".to_string(),
            )
            .unwrap();
        assert_eq!(thread.state, ReviewThreadState::Open);
        assert_eq!(thread.comments.len(), 1);

        let appended = usecase
            .append_comment(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &thread.id,
                "Follow-up".to_string(),
            )
            .unwrap();
        assert_eq!(appended.comments.len(), 2);

        let resolved = usecase
            .resolve_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &thread.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();
        assert_eq!(resolved.state, ReviewThreadState::Resolved);

        let history = usecase.history(dir.path(), "wt", &thread.id).unwrap();
        assert_eq!(history.len(), 3);
        let handoff = usecase
            .build_handoff(dir.path(), "wt", &thread.id, "releash-dev")
            .unwrap();
        assert!(handoff.contains("releash-dev review get"));
    }

    #[test]
    pub fn invalid_create_does_not_persist_event() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();

        let result = usecase.create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            target(),
            " ".to_string(),
        );

        assert!(matches!(result, Err(ReviewError::InvalidInput(_))));
        assert!(usecase
            .list_threads(dir.path(), "wt", None, ReviewActor::human())
            .unwrap()
            .is_empty());
    }

    #[test]
    pub fn nul_content_inputs_are_rejected_before_mutation() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();

        let create = usecase.create_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            target(),
            "bad\0content".to_string(),
        );
        assert!(matches!(create, Err(ReviewError::InvalidInput(_))));
        assert!(usecase
            .list_threads(dir.path(), "wt", None, ReviewActor::human())
            .unwrap()
            .is_empty());

        let thread = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Claim".to_string(),
            )
            .unwrap();
        let append = usecase.append_comment(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "bad\0comment".to_string(),
        );
        assert!(matches!(append, Err(ReviewError::InvalidInput(_))));

        let outcome = usecase.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "bad\0outcome".to_string(),
            "done".to_string(),
        );
        assert!(matches!(outcome, Err(ReviewError::InvalidInput(_))));

        let summary = usecase.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "accepted".to_string(),
            "bad\0summary".to_string(),
        );
        assert!(matches!(summary, Err(ReviewError::InvalidInput(_))));

        let history = usecase.history(dir.path(), "wt", &thread.id).unwrap();
        assert_eq!(history.len(), 1);
    }

    #[test]
    pub fn get_returns_live_thread_and_rejects_deleted_thread() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();
        let thread = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                file_target("src/main.rs", 3),
                "Claim".to_string(),
            )
            .unwrap();

        let got = usecase.get_thread(dir.path(), "wt", &thread.id).unwrap();
        assert_eq!(got.id, thread.id);
        assert_eq!(got.target.file_path.as_deref(), Some("src/main.rs"));

        usecase
            .delete_thread(dir.path(), "wt", ReviewActor::human(), &thread.id)
            .unwrap();
        let deleted = usecase.get_thread(dir.path(), "wt", &thread.id);
        assert!(matches!(deleted, Err(ReviewError::NotFound(_))));
    }

    #[test]
    pub fn append_rejects_missing_and_resolved_threads() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();

        let missing = usecase.append_comment(
            dir.path(),
            "wt",
            ReviewActor::human(),
            "missing-thread",
            "Comment".to_string(),
        );
        assert!(matches!(missing, Err(ReviewError::NotFound(_))));

        let thread = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Claim".to_string(),
            )
            .unwrap();
        usecase
            .resolve_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &thread.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();

        let late = usecase.append_comment(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "Late".to_string(),
        );
        assert!(matches!(late, Err(ReviewError::AlreadyResolved(_))));
    }

    #[test]
    pub fn resolve_rejects_missing_and_already_resolved_threads() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();

        let missing = usecase.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            "missing-thread",
            "accepted".to_string(),
            "done".to_string(),
        );
        assert!(matches!(missing, Err(ReviewError::NotFound(_))));

        let thread = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Claim".to_string(),
            )
            .unwrap();
        usecase
            .resolve_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &thread.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();

        let second = usecase.resolve_thread(
            dir.path(),
            "wt",
            ReviewActor::human(),
            &thread.id,
            "accepted".to_string(),
            "again".to_string(),
        );
        assert!(matches!(second, Err(ReviewError::AlreadyResolved(_))));
    }

    #[test]
    pub fn history_and_handoff_reject_missing_thread() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();

        let history = usecase.history(dir.path(), "wt", "missing-thread");
        assert!(matches!(history, Err(ReviewError::NotFound(_))));

        let handoff = usecase.build_handoff(dir.path(), "wt", "missing-thread", "releash");
        assert!(matches!(handoff, Err(ReviewError::NotFound(_))));
    }

    #[test]
    pub fn list_filters_cover_state_file_author_unread_thread_id_and_combined_axes() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();
        let viewer = agent("codex", "gpt-5");
        let other = agent("claude", "opus");

        let mine = usecase
            .create_thread(
                dir.path(),
                "wt",
                viewer.clone(),
                file_target("src/a.rs", 1),
                "Mine".to_string(),
            )
            .unwrap();
        let resolved_other = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                file_target("src/b.rs", 2),
                "Other".to_string(),
            )
            .unwrap();
        usecase
            .append_comment(
                dir.path(),
                "wt",
                viewer.clone(),
                &resolved_other.id,
                "viewer follow-up".to_string(),
            )
            .unwrap();
        usecase
            .resolve_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &resolved_other.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();
        let unread = usecase
            .create_thread(
                dir.path(),
                "wt",
                viewer.clone(),
                file_target("src/c.rs", 3),
                "Unread".to_string(),
            )
            .unwrap();
        usecase
            .append_comment(
                dir.path(),
                "wt",
                other,
                &unread.id,
                "other follow-up".to_string(),
            )
            .unwrap();

        let by_file = usecase
            .list_threads(
                dir.path(),
                "wt",
                Some(ReviewThreadFilter {
                    file: Some("src/b.rs".to_string()),
                    ..Default::default()
                }),
                viewer.clone(),
            )
            .unwrap();
        assert_eq!(by_file.len(), 1);
        assert_eq!(by_file[0].id, resolved_other.id);

        let by_state = usecase
            .list_threads(
                dir.path(),
                "wt",
                Some(ReviewThreadFilter {
                    state: Some(ReviewThreadState::Resolved),
                    ..Default::default()
                }),
                viewer.clone(),
            )
            .unwrap();
        assert_eq!(by_state.len(), 1);
        assert_eq!(by_state[0].id, resolved_other.id);

        let mine_ids: Vec<_> = usecase
            .list_threads(
                dir.path(),
                "wt",
                Some(ReviewThreadFilter {
                    author: Some(AuthorScope::Mine),
                    ..Default::default()
                }),
                viewer.clone(),
            )
            .unwrap()
            .into_iter()
            .map(|thread| thread.id)
            .collect();
        assert!(mine_ids.contains(&mine.id));
        assert!(mine_ids.contains(&unread.id));
        assert!(!mine_ids.contains(&resolved_other.id));

        let unread_threads = usecase
            .list_threads(
                dir.path(),
                "wt",
                Some(ReviewThreadFilter {
                    unread: Some(true),
                    ..Default::default()
                }),
                viewer.clone(),
            )
            .unwrap();
        assert_eq!(unread_threads.len(), 1);
        assert_eq!(unread_threads[0].id, unread.id);

        let mut by_ids: Vec<_> = usecase
            .list_threads(
                dir.path(),
                "wt",
                Some(ReviewThreadFilter {
                    thread_id: vec![mine.id.clone(), resolved_other.id.clone()],
                    ..Default::default()
                }),
                viewer.clone(),
            )
            .unwrap()
            .into_iter()
            .map(|thread| thread.id)
            .collect();
        by_ids.sort();
        let mut expected = vec![mine.id.clone(), resolved_other.id.clone()];
        expected.sort();
        assert_eq!(by_ids, expected);

        let combined = usecase
            .list_threads(
                dir.path(),
                "wt",
                Some(ReviewThreadFilter {
                    file: Some("src/b.rs".to_string()),
                    state: Some(ReviewThreadState::Resolved),
                    author: Some(AuthorScope::Other),
                    unread: Some(false),
                    thread_id: vec![resolved_other.id.clone()],
                }),
                viewer,
            )
            .unwrap();
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].id, resolved_other.id);
    }

    #[test]
    pub fn delete_is_human_only_and_hides_thread() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();
        let thread = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Claim".to_string(),
            )
            .unwrap();
        let agent = ReviewActor::provider_agent("codex".to_string(), Some("s1".to_string()));

        let denied = usecase.delete_thread(dir.path(), "wt", agent, &thread.id);
        assert!(matches!(denied, Err(ReviewError::PermissionDenied(_))));

        usecase
            .delete_thread(dir.path(), "wt", ReviewActor::human(), &thread.id)
            .unwrap();
        let got = usecase.get_thread(dir.path(), "wt", &thread.id);
        assert!(matches!(got, Err(ReviewError::NotFound(_))));
        let history = usecase.history(dir.path(), "wt", &thread.id).unwrap();
        assert!(matches!(
            history.last(),
            Some(ReviewHistoryEntry::ThreadDeleted { .. })
        ));
    }

    #[test]
    pub fn delete_rejects_unknown_and_already_deleted_threads_but_allows_resolved_threads() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();

        let unknown =
            usecase.delete_thread(dir.path(), "wt", ReviewActor::human(), "missing-thread");
        assert!(matches!(unknown, Err(ReviewError::NotFound(_))));

        let deleted = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Delete me".to_string(),
            )
            .unwrap();
        usecase
            .delete_thread(dir.path(), "wt", ReviewActor::human(), &deleted.id)
            .unwrap();
        let second = usecase.delete_thread(dir.path(), "wt", ReviewActor::human(), &deleted.id);
        assert!(matches!(second, Err(ReviewError::NotFound(_))));

        let resolved = usecase
            .create_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                target(),
                "Resolve then delete".to_string(),
            )
            .unwrap();
        usecase
            .resolve_thread(
                dir.path(),
                "wt",
                ReviewActor::human(),
                &resolved.id,
                "accepted".to_string(),
                "done".to_string(),
            )
            .unwrap();
        usecase
            .delete_thread(dir.path(), "wt", ReviewActor::human(), &resolved.id)
            .unwrap();
        let got = usecase.get_thread(dir.path(), "wt", &resolved.id);
        assert!(matches!(got, Err(ReviewError::NotFound(_))));
    }

    #[test]
    pub fn public_actor_projection_does_not_expose_session_id() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();
        let actor =
            ReviewActor::provider_agent("codex".to_string(), Some("secret-session".to_string()));

        let thread = usecase
            .create_thread(dir.path(), "wt", actor, target(), "Claim".to_string())
            .unwrap();

        assert_eq!(thread.author.kind, ReviewActorKind::Agent);
        let wire = releash_lib::test_support::integration::wire::ReviewThreadDto::try_from(thread)
            .unwrap();
        let json = releash_lib::test_support::integration::wire::from_message(
            "releash.client.v1.ReviewThreadDto",
            &wire,
        )
        .unwrap()
        .to_string();
        assert!(!json.contains("sessionId"));
        assert!(!json.contains("secret-session"));
    }
}
