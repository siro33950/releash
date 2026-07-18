use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::domain::agent_session::{
    AgentSessionReader, AgentSessionStorage, AgentSessionStorageTypes,
};
use crate::domain::path::same_worktree_path;
use crate::usecase::agent_session::context_meta::ContextEpochMeta;
use crate::usecase::agent_session::event_log::{
    AgentSessionEvent, BackendSessionRecoveryProjection, BackendSessionRecoveryReason,
    GoalReactivationOutcome, TurnEventLog,
};

use super::{
    now_timestamp, ChatMessage, ChatSession, ContextCarryState, MessagePart, PageCursor,
    PendingRecoveryMessage, SessionAttachment, SessionMeta, SessionPage, SessionReviewContext,
    SessionState, SessionSummary, SessionToolOutput,
};

/// `SessionState` の遷移を観測する購読者向けコールバック。
/// 引数は `(session_id, worktree_path, new_state)`。
pub type SessionStateChangeListener =
    Arc<dyn Fn(&str, &str, &SessionState) + Send + Sync + 'static>;
pub type SessionEventLogRecoveryListener = Arc<dyn Fn(&str) + Send + Sync + 'static>;

pub trait SessionReviewContextReader: Send + Sync {
    fn get_session_review_context(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionReviewContext>, String>;
}

/// Gateway が物理 event log を修復した事実を usecase へ一度だけ伝える signal。
/// 修復方式や storage format は domain の writer API へ露出させない。
pub(crate) trait SessionEventLogRecoverySignal: Send + Sync {
    fn take_event_log_recovered(&self, session_id: &str) -> bool;
}

pub trait SessionStoragePort:
    AgentSessionStorage<
        Session = ChatSession,
        Meta = SessionMeta,
        PageCursor = PageCursor,
        Page = SessionPage,
        Message = ChatMessage,
        MessagePart = MessagePart,
        Attachment = SessionAttachment,
        ToolOutput = SessionToolOutput,
        Event = AgentSessionEvent,
    > + SessionReviewContextReader
    + SessionEventLogRecoverySignal
    + Send
    + Sync
{
}

impl<T> SessionStoragePort for T where
    T: AgentSessionStorage<
            Session = ChatSession,
            Meta = SessionMeta,
            PageCursor = PageCursor,
            Page = SessionPage,
            Message = ChatMessage,
            MessagePart = MessagePart,
            Attachment = SessionAttachment,
            ToolOutput = SessionToolOutput,
            Event = AgentSessionEvent,
        > + SessionReviewContextReader
        + SessionEventLogRecoverySignal
        + Send
        + Sync
{
}

pub type SessionReaderPort = dyn AgentSessionReader<
        Session = ChatSession,
        Meta = SessionMeta,
        PageCursor = PageCursor,
        Page = SessionPage,
        Message = ChatMessage,
        MessagePart = MessagePart,
        Attachment = SessionAttachment,
        ToolOutput = SessionToolOutput,
        Event = AgentSessionEvent,
    > + Send
    + Sync;

/// テストで session 保存パスへ失敗を注入するためのフック。
/// workflow node session の作成ロールバック経路（fanout child node の save 失敗等）を
/// 検証するために用いる。
#[cfg(test)]
pub(crate) type SessionSaveHook = Arc<dyn Fn(&ChatSession) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionAppendMessageHook =
    Arc<dyn Fn(&str, &ChatMessage) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionPersistPartsHook =
    Arc<dyn Fn(&str, &str, &[MessagePart]) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionAppendEventHook =
    Arc<dyn Fn(&str, &AgentSessionEvent) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
pub(crate) type SessionSetStateHook =
    Arc<dyn Fn(&str, &SessionState) -> Result<(), String> + Send + Sync>;

pub struct SessionStore {
    storage: Arc<dyn SessionStoragePort>,
    state_change_listeners: RwLock<Vec<SessionStateChangeListener>>,
    event_log_recovery_listeners: RwLock<Vec<SessionEventLogRecoveryListener>>,
    recovery_publication_snapshots: RwLock<HashMap<String, SessionSummary>>,
    #[cfg(test)]
    save_hook: RwLock<Option<SessionSaveHook>>,
    #[cfg(test)]
    append_message_hook: RwLock<Option<SessionAppendMessageHook>>,
    #[cfg(test)]
    persist_parts_hook: RwLock<Option<SessionPersistPartsHook>>,
    #[cfg(test)]
    append_event_hook: RwLock<Option<SessionAppendEventHook>>,
    #[cfg(test)]
    set_state_hook: RwLock<Option<SessionSetStateHook>>,
}

fn compact_session_title(title: &str) -> String {
    let compact = title.split_whitespace().collect::<Vec<_>>().join(" ");
    match compact.char_indices().nth(100) {
        Some((byte_pos, _)) => format!("{}…", &compact[..byte_pos]),
        None => compact,
    }
}

impl AgentSessionStorageTypes for SessionStore {
    type Session = ChatSession;
    type Meta = SessionMeta;
    type PageCursor = PageCursor;
    type Page = SessionPage;
    type Message = ChatMessage;
    type MessagePart = MessagePart;
    type Attachment = SessionAttachment;
    type ToolOutput = SessionToolOutput;
    type Event = AgentSessionEvent;
}

impl AgentSessionReader for SessionStore {
    fn list_metas(&self, app_data_dir: &Path) -> Result<Vec<Self::Meta>, String> {
        self.storage.list_metas(app_data_dir)
    }

    fn session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        self.storage.session_title(app_data_dir, session_id)
    }

    fn session_titles(&self, app_data_dir: &Path) -> Result<HashMap<String, String>, String> {
        self.storage.session_titles(app_data_dir)
    }

    fn get_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Meta>, String> {
        self.storage.get_session_meta(app_data_dir, session_id)
    }

    fn load_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Session>, String> {
        self.storage
            .load_full_session_for_restore(app_data_dir, session_id)
    }

    fn load_previous_human_message_before_agent(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_message_id: &str,
    ) -> Result<Option<Self::Message>, String> {
        self.storage.load_previous_human_message_before_agent(
            app_data_dir,
            session_id,
            agent_message_id,
        )
    }

    fn get_session_page(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        cursor: Option<Self::PageCursor>,
        limit: usize,
    ) -> Result<Option<Self::Page>, String> {
        self.storage
            .get_session_page(app_data_dir, session_id, cursor, limit)
    }

    fn get_session_attachment(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<Self::Attachment>, String> {
        self.storage
            .get_session_attachment(app_data_dir, session_id, attachment_id)
    }

    fn get_session_tool_output(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        tool_output_id: &str,
    ) -> Result<Option<Self::ToolOutput>, String> {
        self.storage
            .get_session_tool_output(app_data_dir, session_id, tool_output_id)
    }

    fn load_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<Self::Event>, String> {
        self.storage.load_session_events(app_data_dir, session_id)
    }
}

impl SessionStore {
    pub fn new(storage: Arc<dyn SessionStoragePort>) -> Self {
        Self {
            storage,
            state_change_listeners: RwLock::new(Vec::new()),
            event_log_recovery_listeners: RwLock::new(Vec::new()),
            recovery_publication_snapshots: RwLock::new(HashMap::new()),
            #[cfg(test)]
            save_hook: RwLock::new(None),
            #[cfg(test)]
            append_message_hook: RwLock::new(None),
            #[cfg(test)]
            persist_parts_hook: RwLock::new(None),
            #[cfg(test)]
            append_event_hook: RwLock::new(None),
            #[cfg(test)]
            set_state_hook: RwLock::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_save_hook_for_test(&self, hook: SessionSaveHook) {
        *self.save_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_append_message_hook_for_test(&self, hook: SessionAppendMessageHook) {
        *self.append_message_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_persist_parts_hook_for_test(&self, hook: SessionPersistPartsHook) {
        *self.persist_parts_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_append_event_hook_for_test(&self, hook: SessionAppendEventHook) {
        *self.append_event_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_state_hook_for_test(&self, hook: SessionSetStateHook) {
        *self.set_state_hook.write() = Some(hook);
    }

    pub fn list_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_filtered(app_data_dir, worktree_path, |s| {
            s.state != SessionState::Closed && s.state != SessionState::Archived
        })
    }

    pub fn list_closed_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_filtered(app_data_dir, worktree_path, |s| {
            s.state == SessionState::Closed
        })
    }

    pub(crate) fn hold_recovery_publication_snapshot(&self, summary: SessionSummary) {
        self.recovery_publication_snapshots
            .write()
            .insert(summary.id.clone(), summary);
    }

    pub(crate) fn release_recovery_publication_snapshot(&self, session_id: &str) {
        self.recovery_publication_snapshots
            .write()
            .remove(session_id);
    }

    pub fn list_published_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        let summaries = self.list_sessions(app_data_dir, worktree_path)?;
        self.overlay_recovery_publication_snapshots(app_data_dir, summaries)
    }

    pub fn list_published_closed_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        let summaries = self.list_closed_sessions(app_data_dir, worktree_path)?;
        self.overlay_recovery_publication_snapshots(app_data_dir, summaries)
    }

    fn overlay_recovery_publication_snapshots(
        &self,
        app_data_dir: &Path,
        summaries: Vec<SessionSummary>,
    ) -> Result<Vec<SessionSummary>, String> {
        let mut published = Vec::with_capacity(summaries.len());
        for mut summary in summaries {
            let events = self
                .storage
                .load_session_events(app_data_dir, &summary.id)?;
            let recovery = TurnEventLog::from_events(events).project().backend_recovery;
            if matches!(
                recovery,
                Some(BackendSessionRecoveryProjection::Recovering { .. })
            ) {
                let snapshot = self
                    .recovery_publication_snapshots
                    .read()
                    .get(&summary.id)
                    .cloned();
                let Some(snapshot) = snapshot else {
                    continue;
                };
                let published_title = summary.first_message.clone();
                summary = snapshot;
                summary.first_message = published_title;
            }
            published.push(summary);
        }
        published.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(published)
    }

    fn list_sessions_filtered(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
        predicate: impl Fn(&SessionMeta) -> bool,
    ) -> Result<Vec<SessionSummary>, String> {
        let mut summaries = self
            .storage
            .list_metas(app_data_dir)?
            .into_iter()
            .filter(|s| same_worktree_path(&s.worktree_path, worktree_path) && predicate(s))
            .map(|meta| meta.to_summary())
            .collect::<Vec<_>>();
        summaries.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for summary in &mut summaries {
            if let Some(title) = self.storage.session_title(app_data_dir, &summary.id)? {
                summary.first_message = title;
            }
        }
        Ok(summaries)
    }

    pub fn archive_session(&self, app_data_dir: &Path, session_id: &str) -> Result<(), String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if meta.state != SessionState::Closed {
            return Err("Only closed sessions can be archived".to_string());
        }
        self.set_session_state(app_data_dir, session_id, SessionState::Archived)
    }

    pub fn archive_open_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if meta.workflow_node_session {
            return Err("Workflow node sessions cannot be archived".to_string());
        }
        self.set_session_state(app_data_dir, session_id, SessionState::Archived)
    }

    pub fn session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        self.storage.session_title(app_data_dir, session_id)
    }

    pub fn session_titles(&self, app_data_dir: &Path) -> Result<HashMap<String, String>, String> {
        self.storage.session_titles(app_data_dir)
    }

    pub fn set_session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<SessionSummary, String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if meta.workflow_node_session {
            return Err("Workflow node sessions cannot be renamed".to_string());
        }

        let title_for_summary = title
            .map(compact_session_title)
            .filter(|title| !title.is_empty());
        self.storage
            .write_session_title(app_data_dir, session_id, title_for_summary.as_deref())?;

        let mut summary = meta.to_summary();
        if let Some(title) = title_for_summary {
            summary.first_message = title;
        }
        Ok(summary)
    }

    pub fn fork_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<ChatSession, String> {
        let parent_meta = self.require_meta(app_data_dir, session_id)?;
        if parent_meta.workflow_node_session {
            return Err("Workflow node sessions cannot be forked".to_string());
        }

        let now = now_timestamp();
        let mut forked_meta = parent_meta.clone();
        forked_meta.id = uuid::Uuid::new_v4().to_string();
        forked_meta.state = SessionState::Idle;
        forked_meta.created_at = now;
        forked_meta.updated_at = now;
        forked_meta.agent_session_id = None;
        forked_meta.provider_session_generation = 0;
        forked_meta.context_reinjection_generation = None;
        forked_meta.context_carry = None;
        forked_meta.workflow_node_session = false;

        self.storage
            .fork_session_layout(app_data_dir, session_id, &forked_meta)?;

        let title_result = self
            .storage
            .session_title(app_data_dir, session_id)
            .and_then(|title| {
                if let Some(title) = title {
                    self.storage.write_session_title(
                        app_data_dir,
                        &forked_meta.id,
                        Some(&title),
                    )?;
                }
                Ok(())
            });
        if let Err(err) = title_result {
            self.storage.remove_session(app_data_dir, &forked_meta.id);
            return Err(err);
        }

        Ok(forked_meta.to_session(Vec::new()))
    }

    pub fn get_session_shell(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        Ok(self
            .storage
            .get_session_meta(app_data_dir, session_id)?
            .map(|meta| meta.to_session(Vec::new())))
    }

    pub fn get_session_with_latest_page(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        limit: usize,
    ) -> Result<Option<(ChatSession, SessionPage)>, String> {
        let Some(meta) = self.storage.get_session_meta(app_data_dir, session_id)? else {
            return Ok(None);
        };
        let page = self
            .storage
            .get_session_page(app_data_dir, session_id, None, limit)?
            .unwrap_or(SessionPage {
                messages: Vec::new(),
                message_metadata: Vec::new(),
                next_cursor: None,
                has_more: false,
                total_count: meta.message_count,
                latest_token_usage: None,
            });
        let session = meta.to_session(page.messages.clone());
        Ok(Some((session, page)))
    }

    pub fn get_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionMeta>, String> {
        self.storage.get_session_meta(app_data_dir, session_id)
    }

    pub fn get_session_review_context(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionReviewContext>, String> {
        self.storage
            .get_session_review_context(app_data_dir, session_id)
    }

    pub fn load_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        self.storage
            .load_full_session_for_restore(app_data_dir, session_id)
    }

    pub fn load_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        self.storage.load_session_events(app_data_dir, session_id)
    }

    pub fn append_session_event_and_project_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<SessionState, String> {
        let projected_state =
            self.append_session_event_and_project(app_data_dir, session_id, event)?;
        self.set_session_state(app_data_dir, session_id, projected_state.clone())?;
        Ok(projected_state)
    }

    pub fn append_session_event_and_project(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<SessionState, String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        let events = self
            .storage
            .append_session_event(app_data_dir, session_id, &event)?;
        if self.storage.take_event_log_recovered(session_id) {
            self.notify_event_log_recovered(session_id);
        }
        let projected_state = TurnEventLog::from_events(events)
            .project()
            .status
            .session_state;
        Ok(projected_state)
    }

    pub fn append_session_event_without_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        self.storage
            .append_session_event_without_projection(app_data_dir, session_id, &event)?;
        if self.storage.take_event_log_recovered(session_id) {
            self.notify_event_log_recovered(session_id);
        }
        Ok(())
    }

    pub fn begin_backend_session_recovery(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        recovery_id: &str,
        reason: BackendSessionRecoveryReason,
    ) -> Result<SessionMeta, String> {
        let old_provider_session_generation = self
            .require_meta(app_data_dir, session_id)?
            .provider_session_generation;
        let event = AgentSessionEvent::BackendSessionRecoveryStarted {
            recovery_id: recovery_id.to_string(),
            old_provider_session_generation,
            reason,
            at: now_timestamp(),
        };
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        self.storage.update_session_meta_and_append_session_events(
            app_data_dir,
            session_id,
            &mut |meta| {
                if meta.provider_session_generation != old_provider_session_generation {
                    return Err(format!(
                        "Backend session generation changed while starting recovery: expected {old_provider_session_generation}, actual {}",
                        meta.provider_session_generation
                    ));
                }
                meta.agent_session_id = None;
                meta.context_reinjection_generation = None;
                meta.context_carry = Some(ContextCarryState::Failed);
                meta.updated_at = now_timestamp();
                Ok(())
            },
            &[event],
        )
    }

    pub fn complete_backend_session_recovery(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        recovery_id: &str,
        old_provider_session_generation: u64,
        backend_session_id: String,
    ) -> Result<SessionMeta, String> {
        let provider_session_generation = old_provider_session_generation.saturating_add(1);
        let at = now_timestamp();
        let pending_recovery_message = PendingRecoveryMessage::Notice {
            recovery_id: recovery_id.to_string(),
            message_id: uuid::Uuid::new_v4().to_string(),
        };
        let events = vec![
            AgentSessionEvent::SessionConfigurationReactivated {
                recovery_id: recovery_id.to_string(),
                provider_session_generation,
                consumed_observation_id: None,
                at,
            },
            AgentSessionEvent::SessionGoalReactivated {
                recovery_id: recovery_id.to_string(),
                outcome: GoalReactivationOutcome::NoCurrentGoal,
                provider_session_generation,
                restoring_turn_id: None,
                consumed_observation_id: None,
                at,
            },
            AgentSessionEvent::BackendSessionRecoveryCompleted {
                recovery_id: recovery_id.to_string(),
                provider_session_generation,
                at,
            },
        ];
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in &events {
                hook(session_id, event)?;
            }
        }
        self.storage.update_session_meta_and_append_session_events(
            app_data_dir,
            session_id,
            &mut |meta| {
                if meta.provider_session_generation != old_provider_session_generation {
                    return Err(format!(
                        "Backend session generation changed while completing recovery: expected {old_provider_session_generation}, actual {}",
                        meta.provider_session_generation
                    ));
                }
                meta.agent_session_id = Some(backend_session_id.clone());
                meta.provider_session_generation = provider_session_generation;
                meta.context_reinjection_generation = Some(provider_session_generation);
                meta.pending_recovery_message = Some(pending_recovery_message.clone());
                meta.updated_at = at;
                Ok(())
            },
            &events,
        )
    }

    pub fn fail_backend_session_recovery(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        recovery_id: &str,
        error: &str,
    ) -> Result<SessionMeta, String> {
        let at = now_timestamp();
        let message_id = self
            .storage
            .load_full_session_for_restore(app_data_dir, session_id)?
            .and_then(|session| {
                session
                    .messages
                    .into_iter()
                    .rev()
                    .find(|message| message.role == super::MessageRole::Agent)
                    .map(|message| message.id)
            })
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let pending_recovery_message = PendingRecoveryMessage::Error {
            recovery_id: recovery_id.to_string(),
            message_id,
            error: error.to_string(),
        };
        let event = AgentSessionEvent::BackendSessionRecoveryFailed {
            recovery_id: recovery_id.to_string(),
            error: error.to_string(),
            at,
        };
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        self.storage.update_session_meta_and_append_session_events(
            app_data_dir,
            session_id,
            &mut |meta| {
                meta.state = SessionState::Error;
                meta.pending_recovery_message = Some(pending_recovery_message.clone());
                meta.updated_at = at;
                Ok(())
            },
            &[event],
        )
    }

    pub(crate) fn clear_pending_recovery_message(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        delivered: &PendingRecoveryMessage,
    ) -> Result<(), String> {
        self.update_meta_only(app_data_dir, session_id, |meta| {
            if meta.pending_recovery_message.as_ref() == Some(delivered) {
                meta.pending_recovery_message = None;
            }
            Ok(())
        })?;
        Ok(())
    }

    pub fn record_backend_session_established(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        backend_session_id: String,
        context_carry: Option<ContextCarryState>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            meta.agent_session_id = Some(backend_session_id.clone());
            meta.context_carry = context_carry.clone();
            meta.provider_session_generation = meta.provider_session_generation.saturating_add(1);
            meta.context_reinjection_generation = None;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    pub fn load_previous_human_message_before_agent(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_message_id: &str,
    ) -> Result<Option<ChatMessage>, String> {
        self.storage.load_previous_human_message_before_agent(
            app_data_dir,
            session_id,
            agent_message_id,
        )
    }

    /// workflow step session のセットアップ失敗時に、作成済みの子 session を
    /// 取り除くロールバック経路。storage 層へ削除を委譲する。
    pub(crate) fn remove_session_for_rollback(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        self.storage.remove_session(app_data_dir, session_id);
        Ok(())
    }

    #[cfg(test)]
    pub fn list_worktree_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<ChatSession>, String> {
        Ok(self
            .storage
            .list_metas(app_data_dir)?
            .into_iter()
            .filter(|session| same_worktree_path(&session.worktree_path, worktree_path))
            .map(|meta| meta.to_session(Vec::new()))
            .collect())
    }

    pub fn list_worktree_sessions_full(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<ChatSession>, String> {
        let ids = self
            .storage
            .list_metas(app_data_dir)?
            .into_iter()
            .filter(|session| same_worktree_path(&session.worktree_path, worktree_path))
            .map(|session| session.id)
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| {
                self.load_full_session_for_restore(app_data_dir, &id)
                    .transpose()
            })
            .collect()
    }

    /// Full-session replacement for cold paths that own a complete `ChatSession`.
    ///
    /// Do not pass shell/page sessions returned by `get_session_shell` or `get_session_page`.
    /// Normal runtime updates must use `append_message`, `persist_message_parts`, or meta-only
    /// update methods so page-external message chunks cannot be removed by partial input.
    pub fn save_full_session_for_migration_or_restore(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
    ) -> Result<(), String> {
        let permission_mode =
            crate::domain::agent_session::PermissionMode::parse(&session.permission_mode)
                .map_err(|e| e.to_string())?;
        let normalized_session;
        let session = if session.permission_mode == permission_mode.as_str() {
            session
        } else {
            normalized_session = {
                let mut session = session.clone();
                session.permission_mode = permission_mode.as_str().to_string();
                session
            };
            &normalized_session
        };

        #[cfg(test)]
        if let Some(hook) = self.save_hook.read().clone() {
            hook(session)?;
        }

        let previous_state = self
            .storage
            .get_session_meta(app_data_dir, &session.id)
            .ok()
            .flatten()
            .map(|meta| meta.state);
        self.storage
            .save_full_session_for_migration_or_restore(app_data_dir, session)?;
        if previous_state.as_ref() != Some(&session.state) {
            self.notify_state_change(&session.id, &session.worktree_path, &session.state);
        }
        Ok(())
    }

    /// `SessionState` の遷移を購読するリスナーを登録する。
    /// 登録順に保存後に発火される。AgentStatusCenter のような中央管理が
    /// SessionStore からの状態変更を一方向に受け取るための入口。
    pub fn register_state_change_listener(&self, listener: SessionStateChangeListener) {
        self.state_change_listeners.write().push(listener);
    }

    pub fn register_event_log_recovery_listener(&self, listener: SessionEventLogRecoveryListener) {
        self.event_log_recovery_listeners.write().push(listener);
    }

    fn notify_state_change(&self, session_id: &str, worktree_path: &str, new_state: &SessionState) {
        let listeners = self.state_change_listeners.read().clone();
        for listener in listeners {
            listener(session_id, worktree_path, new_state);
        }
    }

    fn notify_event_log_recovered(&self, session_id: &str) {
        let listeners = self.event_log_recovery_listeners.read().clone();
        for listener in listeners {
            listener(session_id);
        }
    }

    fn require_meta(&self, app_data_dir: &Path, session_id: &str) -> Result<SessionMeta, String> {
        self.storage
            .get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))
    }

    fn update_meta_only(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: impl FnOnce(&mut SessionMeta) -> Result<(), String>,
    ) -> Result<Option<(String, SessionState)>, String> {
        let mut update = Some(update);
        let mut previous_state: Option<SessionState> = None;
        let meta = self
            .storage
            .update_session_meta(app_data_dir, session_id, &mut |meta| {
                previous_state = Some(meta.state.clone());
                let f = update
                    .take()
                    .expect("update closure must be invoked exactly once");
                f(meta)
            })?;
        let previous_state =
            previous_state.expect("update_session_meta must invoke closure before returning Ok");
        if previous_state != meta.state {
            Ok(Some((meta.worktree_path.clone(), meta.state.clone())))
        } else {
            Ok(None)
        }
    }

    fn update_meta_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: impl FnOnce(&mut SessionMeta) -> Result<bool, String>,
    ) -> Result<Option<SessionMeta>, String> {
        let mut update = Some(update);
        let mut changed = false;
        let meta = self
            .storage
            .update_session_meta(app_data_dir, session_id, &mut |meta| {
                let f = update
                    .take()
                    .expect("update closure must be invoked exactly once");
                changed = f(meta)?;
                if !changed {
                    return Err("__update_meta_if_changed::no_change__".to_string());
                }
                Ok(())
            });
        match meta {
            Ok(meta) => Ok(Some(meta)),
            Err(err) if err == "__update_meta_if_changed::no_change__" && !changed => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub fn set_session_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.set_state_hook.read().clone() {
            hook(session_id, &state)?;
        }
        let state_for_notify = state.clone();
        let change = self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.state = state;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        if let Some((worktree_path, _)) = change {
            self.notify_state_change(session_id, &worktree_path, &state_for_notify);
        }
        Ok(())
    }

    pub fn update_permission_mode(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        let permission_mode = crate::domain::agent_session::PermissionMode::parse(permission_mode)
            .map_err(|e| e.to_string())?;
        self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.permission_mode = permission_mode.as_str().to_string();
            Ok(())
        })?;
        Ok(())
    }

    pub fn update_plan_mode(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        plan_mode: bool,
    ) -> Result<(), String> {
        self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.plan_mode = plan_mode;
            Ok(())
        })?;
        Ok(())
    }

    pub fn update_backend_selection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        backend_id: String,
        selected_model: Option<String>,
    ) -> Result<(), String> {
        self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.backend_id = backend_id;
            meta.selected_model = selected_model;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        Ok(())
    }

    #[allow(dead_code)] // issues-1301 G-1: retained for permission profile settings surface; current runtime only reads the stored profile id.
    pub fn update_permission_profile_id(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_profile_id: Option<&str>,
    ) -> Result<(), String> {
        let profile_id = permission_profile_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.chars().any(char::is_control) {
                    Err("Permission profile id cannot contain control characters".to_string())
                } else {
                    Ok(value.to_string())
                }
            })
            .transpose()?;
        self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.permission_profile_id = profile_id;
            Ok(())
        })?;
        Ok(())
    }

    pub fn update_agent_session_id(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_session_id: Option<String>,
    ) -> Result<(), String> {
        self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.agent_session_id = agent_session_id;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn update_agent_session_id_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_session_id: Option<String>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.agent_session_id == agent_session_id {
                return Ok(false);
            }
            meta.agent_session_id = agent_session_id;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    pub fn update_context_carry_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        context_carry: Option<ContextCarryState>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.context_carry == context_carry {
                return Ok(false);
            }
            meta.context_carry = context_carry;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    pub fn complete_context_reinjection_if_required(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        reinjected: bool,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.context_reinjection_generation != Some(meta.provider_session_generation) {
                return Ok(false);
            }
            meta.context_reinjection_generation = None;
            if reinjected {
                meta.context_carry = Some(ContextCarryState::Reinjected);
            }
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    pub fn update_system_context_private_meta_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        context_epoch: Option<ContextEpochMeta>,
        workflow_instructions: Vec<String>,
        agent_read_paths: Option<Vec<PathBuf>>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.context_epoch == context_epoch
                && meta.workflow_instructions == workflow_instructions
                && (agent_read_paths.is_none() || meta.agent_read_paths == agent_read_paths)
            {
                return Ok(false);
            }
            meta.context_epoch = context_epoch;
            meta.workflow_instructions = workflow_instructions;
            if agent_read_paths.is_some() {
                meta.agent_read_paths = agent_read_paths.clone();
            }
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    #[cfg(test)]
    pub fn update_resume_metadata_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_session_id: Option<String>,
        context_carry: Option<ContextCarryState>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.agent_session_id == agent_session_id && meta.context_carry == context_carry {
                return Ok(false);
            }
            meta.agent_session_id = agent_session_id;
            meta.context_carry = context_carry;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    pub fn get_session_page(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        cursor: Option<PageCursor>,
        limit: usize,
    ) -> Result<Option<SessionPage>, String> {
        self.storage
            .get_session_page(app_data_dir, session_id, cursor, limit)
    }

    pub fn append_message(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message: &ChatMessage,
    ) -> Result<SessionMeta, String> {
        #[cfg(test)]
        if let Some(hook) = self.append_message_hook.read().clone() {
            hook(session_id, message)?;
        }
        self.storage
            .append_message(app_data_dir, session_id, message)
    }

    pub fn get_session_attachment(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<SessionAttachment>, String> {
        self.storage
            .get_session_attachment(app_data_dir, session_id, attachment_id)
    }

    pub fn get_session_tool_output(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        tool_output_id: &str,
    ) -> Result<Option<SessionToolOutput>, String> {
        self.storage
            .get_session_tool_output(app_data_dir, session_id, tool_output_id)
    }

    pub fn persist_message_parts(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[MessagePart],
        streaming_final_seq: u64,
        completed_at: Option<f64>,
    ) -> Result<Vec<MessagePart>, String> {
        #[cfg(test)]
        if let Some(hook) = self.persist_parts_hook.read().clone() {
            hook(session_id, message_id, parts)?;
        }
        self.storage.persist_message_parts(
            app_data_dir,
            session_id,
            message_id,
            parts,
            streaming_final_seq,
            completed_at,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn ids(values: impl IntoIterator<Item = String>) -> HashSet<String> {
        values.into_iter().collect()
    }

    fn rewrite_persisted_worktree_path(app_data_dir: &Path, session_id: &str, worktree_path: &str) {
        let meta_path = app_data_dir
            .join("sessions")
            .join(session_id)
            .join("meta.json");
        let mut meta: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
        meta["worktreePath"] = serde_json::Value::String(worktree_path.to_string());
        std::fs::write(meta_path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();
    }

    #[test]
    fn worktree_session_queries_match_legacy_trailing_slash_without_prefix_collision() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let writer = crate::test_support::build_session_store();
        let legacy = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo/",
            Some("claude".to_string()),
        )
        .unwrap();
        let canonical = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        let other = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repository",
            Some("claude".to_string()),
        )
        .unwrap();

        // Simulate metadata written before worktree paths were normalized on save.
        rewrite_persisted_worktree_path(app_data_dir.path(), &legacy.id, "/repo/");
        drop(writer);

        let reader = crate::test_support::build_session_store();
        let expected = HashSet::from([legacy.id.clone(), canonical.id.clone()]);
        for query in ["/repo", "/repo/"] {
            let summaries = reader.list_sessions(app_data_dir.path(), query).unwrap();
            assert_eq!(
                ids(summaries.iter().map(|session| session.id.clone())),
                expected
            );
            assert!(
                summaries
                    .iter()
                    .all(|session| session.worktree_path == "/repo"),
                "read models must expose the normalized identity"
            );

            assert_eq!(
                ids(reader
                    .list_worktree_sessions(app_data_dir.path(), query)
                    .unwrap()
                    .into_iter()
                    .map(|session| session.id),),
                expected
            );
            assert_eq!(
                ids(reader
                    .list_worktree_sessions_full(app_data_dir.path(), query)
                    .unwrap()
                    .into_iter()
                    .map(|session| session.id),),
                expected
            );
        }

        assert_eq!(
            ids(reader
                .list_sessions(app_data_dir.path(), "/repository")
                .unwrap()
                .into_iter()
                .map(|session| session.id),),
            HashSet::from([other.id])
        );
    }

    #[test]
    fn published_lists_restore_recovery_suppression_from_durable_events_after_restart() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let writer = crate::test_support::build_session_store();
        let active = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let closed = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .set_session_state(app_data_dir.path(), &closed.id, SessionState::Closed)
            .unwrap();

        for session_id in [&active.id, &closed.id] {
            let published_snapshot = writer
                .get_session_meta(app_data_dir.path(), session_id)
                .unwrap()
                .unwrap()
                .to_summary();
            writer.hold_recovery_publication_snapshot(published_snapshot);
            writer
                .begin_backend_session_recovery(
                    app_data_dir.path(),
                    session_id,
                    &format!("recovery-{session_id}"),
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .unwrap();
        }

        assert_eq!(
            ids(writer
                .list_published_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([active.id.clone()])
        );
        assert_eq!(
            ids(writer
                .list_published_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([closed.id.clone()])
        );
        drop(writer);

        let reopened = crate::test_support::build_session_store();
        assert_eq!(
            ids(reopened
                .list_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([active.id.clone()])
        );
        assert_eq!(
            ids(reopened
                .list_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([closed.id.clone()])
        );
        for session_id in [&active.id, &closed.id] {
            let recovery = TurnEventLog::from_events(
                reopened
                    .load_session_events(app_data_dir.path(), session_id)
                    .unwrap(),
            )
            .project()
            .backend_recovery;
            assert!(matches!(
                recovery,
                Some(BackendSessionRecoveryProjection::Recovering { .. })
            ));
        }
        assert!(reopened
            .list_published_sessions(app_data_dir.path(), "/repo")
            .unwrap()
            .is_empty());
        assert!(reopened
            .list_published_closed_sessions(app_data_dir.path(), "/repo")
            .unwrap()
            .is_empty());
    }
}
