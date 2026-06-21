use std::path::Path;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::domain::agent_session::{
    AgentSessionReader, AgentSessionStorage, AgentSessionStorageTypes,
};

use super::{
    now_timestamp, ChatMessage, ChatSession, ContextCarryState, MessagePart, PageCursor,
    SessionAttachment, SessionMeta, SessionPage, SessionState, SessionSummary,
};

/// `SessionState` の遷移を観測する購読者向けコールバック。
/// 引数は `(session_id, worktree_path, new_state)`。
pub type SessionStateChangeListener =
    Arc<dyn Fn(&str, &str, &SessionState) + Send + Sync + 'static>;

pub type SessionStoragePort = dyn AgentSessionStorage<
        Session = ChatSession,
        Meta = SessionMeta,
        PageCursor = PageCursor,
        Page = SessionPage,
        Message = ChatMessage,
        MessagePart = MessagePart,
        Attachment = SessionAttachment,
    > + Send
    + Sync;

pub type SessionReaderPort = dyn AgentSessionReader<
        Session = ChatSession,
        Meta = SessionMeta,
        PageCursor = PageCursor,
        Page = SessionPage,
        Message = ChatMessage,
        MessagePart = MessagePart,
        Attachment = SessionAttachment,
    > + Send
    + Sync;

pub struct SessionStore {
    storage: Arc<SessionStoragePort>,
    state_change_listeners: RwLock<Vec<SessionStateChangeListener>>,
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
}

impl SessionStore {
    pub fn new(storage: Arc<SessionStoragePort>) -> Self {
        Self {
            storage,
            state_change_listeners: RwLock::new(Vec::new()),
        }
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
            .filter(|s| s.worktree_path == worktree_path && predicate(s))
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
        if meta.workflow_step_session {
            return Err("Workflow step sessions cannot be archived".to_string());
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

    pub fn set_session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<SessionSummary, String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if meta.workflow_step_session {
            return Err("Workflow step sessions cannot be renamed".to_string());
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
        if parent_meta.workflow_step_session {
            return Err("Workflow step sessions cannot be forked".to_string());
        }

        let now = now_timestamp();
        let mut forked_meta = parent_meta.clone();
        forked_meta.id = uuid::Uuid::new_v4().to_string();
        forked_meta.state = SessionState::Idle;
        forked_meta.created_at = now;
        forked_meta.updated_at = now;
        forked_meta.agent_session_id = None;
        forked_meta.context_carry = None;
        forked_meta.workflow_step_session = false;

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

    pub fn get_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionMeta>, String> {
        self.storage.get_session_meta(app_data_dir, session_id)
    }

    pub fn load_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        self.storage
            .load_full_session_for_restore(app_data_dir, session_id)
    }

    pub fn list_worktree_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<ChatSession>, String> {
        Ok(self
            .storage
            .list_metas(app_data_dir)?
            .into_iter()
            .filter(|session| session.worktree_path == worktree_path)
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
            .filter(|session| session.worktree_path == worktree_path)
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
        let permission_mode = crate::permission::PermissionMode::parse(&session.permission_mode)
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

    fn notify_state_change(&self, session_id: &str, worktree_path: &str, new_state: &SessionState) {
        let listeners = self.state_change_listeners.read().clone();
        for listener in listeners {
            listener(session_id, worktree_path, new_state);
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
        let mut meta = self.require_meta(app_data_dir, session_id)?;
        let previous_state = meta.state.clone();
        update(&mut meta)?;
        self.storage.write_session_meta(app_data_dir, &meta)?;
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
        let mut meta = self.require_meta(app_data_dir, session_id)?;
        if !update(&mut meta)? {
            return Ok(None);
        }
        self.storage.write_session_meta(app_data_dir, &meta)?;
        Ok(Some(meta))
    }

    pub fn set_session_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
    ) -> Result<(), String> {
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
        let permission_mode =
            crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;
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
            meta.backend_id = Some(backend_id);
            meta.selected_model = selected_model;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        Ok(())
    }

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
    ) -> Result<(), String> {
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

    pub fn persist_message_parts(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[MessagePart],
        completed_at: Option<f64>,
    ) -> Result<(), String> {
        self.storage.persist_message_parts(
            app_data_dir,
            session_id,
            message_id,
            parts,
            completed_at,
        )
    }
}
