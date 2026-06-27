use std::path::Path;
use std::sync::Arc;

mod dto;

use crate::domain::comment::{
    apply_filter, ensure_can_delete, ensure_thread_open, project_thread, project_threads,
    validate_content, validate_filter, validate_target, ReviewActor, ReviewError, ReviewEvent,
    ReviewHistoryEntry, ReviewTarget, ReviewThread, ReviewThreadFilter,
};

pub(crate) use dto::{
    review_error_to_json_string, ReviewHistoryEntryDto, ReviewThreadDto, ReviewThreadFilterDto,
};

pub(crate) type ReviewEventMutation<'a> =
    Box<dyn FnOnce(&[ReviewEvent]) -> Result<Vec<ReviewEvent>, ReviewError> + Send + 'a>;

pub(crate) trait ReviewEventStore: Send + Sync {
    fn load(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
    ) -> Result<Vec<ReviewEvent>, ReviewError>;

    fn mutate(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        mutation: ReviewEventMutation<'_>,
    ) -> Result<Vec<ReviewEvent>, ReviewError>;
}

pub(crate) trait ReviewClock: Send + Sync {
    fn now(&self) -> f64;
}

pub(crate) trait ReviewIdGenerator: Send + Sync {
    fn event_id(&self) -> String;
}

pub(crate) struct ReviewCommentUsecase {
    store: Arc<dyn ReviewEventStore>,
    clock: Arc<dyn ReviewClock>,
    id_generator: Arc<dyn ReviewIdGenerator>,
}

impl ReviewCommentUsecase {
    pub(crate) fn new(
        store: Arc<dyn ReviewEventStore>,
        clock: Arc<dyn ReviewClock>,
        id_generator: Arc<dyn ReviewIdGenerator>,
    ) -> Self {
        Self {
            store,
            clock,
            id_generator,
        }
    }

    pub(crate) fn list_threads(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        filter: Option<ReviewThreadFilter>,
        viewer: ReviewActor,
    ) -> Result<Vec<ReviewThread>, ReviewError> {
        validate_filter(&filter)?;
        let events = self.store.load(app_data_dir, worktree_name)?;
        Ok(apply_filter(
            project_threads(worktree_name, &events),
            filter,
            &viewer,
        ))
    }

    pub(crate) fn get_thread(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        thread_id: &str,
    ) -> Result<ReviewThread, ReviewError> {
        let events = self.store.load(app_data_dir, worktree_name)?;
        find_thread(worktree_name, thread_id, &events)
    }

    pub(crate) fn history(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewHistoryEntry>, ReviewError> {
        Ok(self
            .history_events(app_data_dir, worktree_name, thread_id)?
            .iter()
            .map(ReviewHistoryEntry::from)
            .collect())
    }

    pub(crate) fn create_thread(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        actor: ReviewActor,
        target: ReviewTarget,
        content: String,
    ) -> Result<ReviewThread, ReviewError> {
        validate_content(&content, "content")?;
        validate_target(&target)?;
        let clock = Arc::clone(&self.clock);
        let id_generator = Arc::clone(&self.id_generator);
        let thread_id = id_generator.event_id();
        let thread_id_for_event = thread_id.clone();
        let events = self.store.mutate(
            app_data_dir,
            worktree_name,
            Box::new(move |_| {
                let at = clock.now();
                Ok(vec![ReviewEvent::ThreadCreated {
                    event_id: id_generator.event_id(),
                    thread_id: thread_id_for_event,
                    comment_id: id_generator.event_id(),
                    actor,
                    target,
                    content,
                    at,
                }])
            }),
        )?;
        find_thread(worktree_name, &thread_id, &events)
    }

    pub(crate) fn append_comment(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        actor: ReviewActor,
        thread_id: &str,
        content: String,
    ) -> Result<ReviewThread, ReviewError> {
        validate_content(&content, "content")?;
        let clock = Arc::clone(&self.clock);
        let id_generator = Arc::clone(&self.id_generator);
        let thread_id_for_event = thread_id.to_string();
        let events = self.store.mutate(
            app_data_dir,
            worktree_name,
            Box::new(move |events| {
                let thread = find_thread(worktree_name, &thread_id_for_event, events)?;
                ensure_thread_open(&thread, &thread_id_for_event)?;
                Ok(vec![ReviewEvent::CommentAppended {
                    event_id: id_generator.event_id(),
                    thread_id: thread_id_for_event,
                    comment_id: id_generator.event_id(),
                    actor,
                    content,
                    at: clock.now(),
                }])
            }),
        )?;
        find_thread(worktree_name, thread_id, &events)
    }

    pub(crate) fn resolve_thread(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        actor: ReviewActor,
        thread_id: &str,
        outcome: String,
        summary: String,
    ) -> Result<ReviewThread, ReviewError> {
        validate_content(&outcome, "outcome")?;
        validate_content(&summary, "summary")?;
        let clock = Arc::clone(&self.clock);
        let id_generator = Arc::clone(&self.id_generator);
        let thread_id_for_event = thread_id.to_string();
        let events = self.store.mutate(
            app_data_dir,
            worktree_name,
            Box::new(move |events| {
                let thread = find_thread(worktree_name, &thread_id_for_event, events)?;
                ensure_thread_open(&thread, &thread_id_for_event)?;
                Ok(vec![ReviewEvent::ThreadResolved {
                    event_id: id_generator.event_id(),
                    thread_id: thread_id_for_event,
                    actor,
                    outcome,
                    summary,
                    at: clock.now(),
                }])
            }),
        )?;
        find_thread(worktree_name, thread_id, &events)
    }

    pub(crate) fn delete_thread(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        actor: ReviewActor,
        thread_id: &str,
    ) -> Result<(), ReviewError> {
        ensure_can_delete(&actor)?;
        let clock = Arc::clone(&self.clock);
        let id_generator = Arc::clone(&self.id_generator);
        let thread_id_for_event = thread_id.to_string();
        self.store.mutate(
            app_data_dir,
            worktree_name,
            Box::new(move |events| {
                find_thread(worktree_name, &thread_id_for_event, events)?;
                Ok(vec![ReviewEvent::ThreadDeleted {
                    event_id: id_generator.event_id(),
                    thread_id: thread_id_for_event,
                    actor,
                    at: clock.now(),
                }])
            }),
        )?;
        Ok(())
    }

    pub(crate) fn build_handoff(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        thread_id: &str,
        releash_alias: &str,
    ) -> Result<String, ReviewError> {
        let thread = self.get_thread(app_data_dir, worktree_name, thread_id)?;
        Ok(build_review_thread_handoff_message(releash_alias, &thread))
    }

    fn history_events(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        thread_id: &str,
    ) -> Result<Vec<ReviewEvent>, ReviewError> {
        let events: Vec<_> = self
            .store
            .load(app_data_dir, worktree_name)?
            .into_iter()
            .filter(|event| event.thread_id() == thread_id)
            .collect();
        if !events
            .iter()
            .any(|event| matches!(event, ReviewEvent::ThreadCreated { .. }))
        {
            return Err(ReviewError::NotFound(format!(
                "Review thread not found: {thread_id}"
            )));
        }
        Ok(events)
    }
}

pub(crate) fn build_review_thread_handoff_message(
    releash_alias: &str,
    thread: &ReviewThread,
) -> String {
    format!(
        "以下のスレッドの内容を確認してください。\n\n{releash_alias} review get --session-id \"$RELEASH_SESSION_ID\" {}",
        thread.id
    )
}

fn find_thread(
    worktree_name: &str,
    thread_id: &str,
    events: &[ReviewEvent],
) -> Result<ReviewThread, ReviewError> {
    project_thread(worktree_name, thread_id, events)
        .ok_or_else(|| ReviewError::NotFound(format!("Review thread not found: {thread_id}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::comment::{AuthorScope, ReviewActorKind, ReviewThreadState};
    use parking_lot::Mutex;
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

    fn agent(backend_id: &str, model: &str) -> ReviewActor {
        ReviewActor::agent(backend_id.to_string(), model.to_string(), None)
    }

    #[test]
    fn review_thread_filter_dto_deserializes_and_maps_to_domain_filter() {
        let dto: ReviewThreadFilterDto = serde_json::from_value(serde_json::json!({
            "file": "src/main.rs",
            "state": "resolved",
            "author": "mine",
            "unread": true,
            "threadId": ["thread-a", "thread-b"]
        }))
        .unwrap();

        let filter: ReviewThreadFilter = dto.into();

        assert_eq!(filter.file.as_deref(), Some("src/main.rs"));
        assert_eq!(filter.state, Some(ReviewThreadState::Resolved));
        assert_eq!(filter.author, Some(AuthorScope::Mine));
        assert_eq!(filter.unread, Some(true));
        assert_eq!(
            filter.thread_id,
            vec!["thread-a".to_string(), "thread-b".to_string()]
        );
    }

    #[test]
    fn create_append_resolve_history_and_handoff_use_storage_port() {
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
    fn invalid_create_does_not_persist_event() {
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
    fn nul_content_inputs_are_rejected_before_mutation() {
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
    fn get_returns_live_thread_and_rejects_deleted_thread() {
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
    fn append_rejects_missing_and_resolved_threads() {
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
    fn resolve_rejects_missing_and_already_resolved_threads() {
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
    fn history_and_handoff_reject_missing_thread() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();

        let history = usecase.history(dir.path(), "wt", "missing-thread");
        assert!(matches!(history, Err(ReviewError::NotFound(_))));

        let handoff = usecase.build_handoff(dir.path(), "wt", "missing-thread", "releash");
        assert!(matches!(handoff, Err(ReviewError::NotFound(_))));
    }

    #[test]
    fn list_filters_cover_state_file_author_unread_thread_id_and_combined_axes() {
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
    fn delete_is_human_only_and_hides_thread() {
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
        let agent = ReviewActor::agent(
            "codex".to_string(),
            "gpt-5".to_string(),
            Some("s1".to_string()),
        );

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
    fn delete_rejects_unknown_and_already_deleted_threads_but_allows_resolved_threads() {
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
    fn public_actor_projection_does_not_expose_session_id() {
        let dir = TempDir::new().unwrap();
        let usecase = usecase();
        let actor = ReviewActor::agent(
            "codex".to_string(),
            "gpt-5".to_string(),
            Some("secret-session".to_string()),
        );

        let thread = usecase
            .create_thread(dir.path(), "wt", actor, target(), "Claim".to_string())
            .unwrap();

        assert_eq!(thread.author.kind, ReviewActorKind::Agent);
        let json = serde_json::to_string(&ReviewThreadDto::from(&thread)).unwrap();
        assert!(!json.contains("sessionId"));
        assert!(!json.contains("secret-session"));
    }
}
