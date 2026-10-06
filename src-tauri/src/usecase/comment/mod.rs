use std::path::Path;
use std::sync::Arc;

pub(crate) mod dto;

use crate::domain::comment::{
    apply_filter, ensure_can_delete, ensure_thread_open, project_thread, project_threads,
    validate_content, validate_filter, validate_target, ReviewActor, ReviewError, ReviewEvent,
    ReviewHistoryEntry, ReviewTarget, ReviewThread, ReviewThreadFilter,
};

pub(crate) use dto::{ReviewHistoryEntryDto, ReviewThreadDto};

pub type ReviewEventMutation<'a> =
    Box<dyn FnOnce(&[ReviewEvent]) -> Result<Vec<ReviewEvent>, ReviewError> + Send + 'a>;

pub trait ReviewEventStore: Send + Sync {
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

pub trait ReviewClock: Send + Sync {
    fn now(&self) -> f64;
}

pub trait ReviewIdGenerator: Send + Sync {
    fn event_id(&self) -> String;
}

pub struct ReviewCommentUsecase {
    store: Arc<dyn ReviewEventStore>,
    clock: Arc<dyn ReviewClock>,
    id_generator: Arc<dyn ReviewIdGenerator>,
    subscriptions: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
}

impl ReviewCommentUsecase {
    pub fn new(
        store: Arc<dyn ReviewEventStore>,
        clock: Arc<dyn ReviewClock>,
        id_generator: Arc<dyn ReviewIdGenerator>,
    ) -> Self {
        Self {
            store,
            clock,
            id_generator,
            subscriptions: None,
        }
    }

    pub fn with_subscriptions(
        mut self,
        subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.subscriptions = Some(subscriptions);
        self
    }

    fn notify_changed(&self, worktree_name: &str) {
        if let Some(subscriptions) = &self.subscriptions {
            subscriptions.notify(
                crate::usecase::state_subscription::StateChangeSource::ReviewComments(Some(
                    worktree_name.into(),
                )),
            );
        }
    }

    pub fn list_threads(
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

    pub fn get_thread(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        thread_id: &str,
    ) -> Result<ReviewThread, ReviewError> {
        let events = self.store.load(app_data_dir, worktree_name)?;
        find_thread(worktree_name, thread_id, &events)
    }

    pub fn history(
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

    pub fn create_thread(
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
        let thread = find_thread(worktree_name, &thread_id, &events)?;
        self.notify_changed(worktree_name);
        Ok(thread)
    }

    pub fn append_comment(
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
        let thread = find_thread(worktree_name, thread_id, &events)?;
        self.notify_changed(worktree_name);
        Ok(thread)
    }

    pub fn resolve_thread(
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
        let thread = find_thread(worktree_name, thread_id, &events)?;
        self.notify_changed(worktree_name);
        Ok(thread)
    }

    pub fn delete_thread(
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
        self.notify_changed(worktree_name);
        Ok(())
    }

    pub fn build_handoff(
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
