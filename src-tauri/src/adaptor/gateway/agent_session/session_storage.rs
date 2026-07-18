use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use parking_lot::RwLock;

use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::session::{
    ChatMessage, ChatSession, MessagePart, PageCursor, SessionAttachment,
    SessionEventLogRecoverySignal, SessionMeta, SessionPage, SessionQueuePauseReader,
    SessionReviewContext, SessionReviewContextReader, SessionToolOutput,
};

mod attachment_blob;
mod event_store;
mod fork_copier;
mod gc;
mod layout;
mod message_store;
mod meta_repository;
mod private_context;
mod projection_commit;
mod titles;
mod tool_output_blob;
mod transaction;

#[cfg(test)]
mod tests;

pub(crate) use gc::SessionGcMetaRead;

#[cfg(test)]
pub(crate) use projection_commit::ProjectionCommitStage;

#[cfg(test)]
pub(crate) type ProjectionCommitHook =
    std::sync::Arc<dyn Fn(ProjectionCommitStage) -> Result<(), String> + Send + Sync>;

pub struct FileSessionStorage {
    pub(super) cache: RwLock<HashMap<String, SessionMeta>>,
    /// 壊れた / 旧形式の session JSON を session_id 単位で隔離する。
    /// Spec issues-947: 1つの不正セッションで全体ロードを Err にせず、無関係な正常セッションの
    /// 一覧取得・取得は素通しさせる。値は API に返す汎化済みエラー文言（フルパス・serde 生メッセージは含まない）。
    pub(super) invalid_sessions: RwLock<HashMap<String, String>>,
    /// Durable commit 済みだが meta/events への反映が完了していない session。
    /// clean session の read path で transaction marker を毎回確認しないため、
    /// process 内の reconciliation 対象を session id 単位で限定する。
    pub(super) materialization_pending_sessions: RwLock<HashSet<String>>,
    pub(super) file_lock: parking_lot::Mutex<()>,
    pub(super) loaded: AtomicBool,
    pub(super) recovered_event_logs: RwLock<HashSet<String>>,
    #[cfg(test)]
    pub(super) message_read_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(super) meta_read_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    transaction_apply_hook: RwLock<Option<transaction::TransactionApplyHook>>,
    #[cfg(test)]
    pub(super) projection_commit_hook: RwLock<Option<ProjectionCommitHook>>,
    #[cfg(test)]
    pub(super) event_read_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(super) event_batch_directory_scan_count: std::sync::atomic::AtomicUsize,
}

impl Default for FileSessionStorage {
    fn default() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            invalid_sessions: RwLock::new(HashMap::new()),
            materialization_pending_sessions: RwLock::new(HashSet::new()),
            file_lock: parking_lot::Mutex::new(()),
            loaded: AtomicBool::new(false),
            recovered_event_logs: RwLock::new(HashSet::new()),
            #[cfg(test)]
            message_read_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            meta_read_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            transaction_apply_hook: RwLock::new(None),
            #[cfg(test)]
            projection_commit_hook: RwLock::new(None),
            #[cfg(test)]
            event_read_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            event_batch_directory_scan_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl crate::domain::agent_session::AgentSessionStorageTypes for FileSessionStorage {
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

impl crate::domain::agent_session::AgentSessionReader for FileSessionStorage {
    fn list_metas(&self, app_data_dir: &Path) -> Result<Vec<Self::Meta>, String> {
        FileSessionStorage::list_metas(self, app_data_dir)
    }

    fn session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        FileSessionStorage::session_title(self, app_data_dir, session_id)
    }

    fn session_titles(&self, app_data_dir: &Path) -> Result<HashMap<String, String>, String> {
        FileSessionStorage::session_titles(self, app_data_dir)
    }

    fn get_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Meta>, String> {
        FileSessionStorage::get_session_meta(self, app_data_dir, session_id)
    }

    fn load_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Session>, String> {
        FileSessionStorage::load_full_session_for_restore(self, app_data_dir, session_id)
    }

    fn load_previous_human_message_before_agent(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_message_id: &str,
    ) -> Result<Option<Self::Message>, String> {
        FileSessionStorage::load_previous_human_message_before_agent(
            self,
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
        FileSessionStorage::get_session_page(self, app_data_dir, session_id, cursor, limit)
    }

    fn get_session_attachment(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<Self::Attachment>, String> {
        FileSessionStorage::get_session_attachment(self, app_data_dir, session_id, attachment_id)
    }

    fn get_session_tool_output(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        tool_output_id: &str,
    ) -> Result<Option<Self::ToolOutput>, String> {
        FileSessionStorage::get_session_tool_output(self, app_data_dir, session_id, tool_output_id)
    }

    fn load_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<Self::Event>, String> {
        FileSessionStorage::load_session_events(self, app_data_dir, session_id)
    }
}

impl SessionReviewContextReader for FileSessionStorage {
    fn get_session_review_context(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionReviewContext>, String> {
        FileSessionStorage::get_session_review_context(self, app_data_dir, session_id)
    }
}

impl SessionEventLogRecoverySignal for FileSessionStorage {
    fn take_event_log_recovered(&self, session_id: &str) -> bool {
        self.recovered_event_logs.write().remove(session_id)
    }
}

impl SessionQueuePauseReader for FileSessionStorage {
    fn load_queue_paused_at(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<f64>, String> {
        FileSessionStorage::load_queue_paused_at(self, app_data_dir, session_id)
    }
}

impl crate::domain::agent_session::AgentSessionWriter for FileSessionStorage {
    fn write_session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<(), String> {
        FileSessionStorage::write_session_title(self, app_data_dir, session_id, title)
    }

    fn fork_session_layout(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        forked_meta: &Self::Meta,
    ) -> Result<(), String> {
        FileSessionStorage::fork_session_layout(self, app_data_dir, session_id, forked_meta)
    }

    fn remove_session(&self, app_data_dir: &Path, session_id: &str) {
        FileSessionStorage::remove_session(self, app_data_dir, session_id);
    }

    fn update_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: &mut dyn FnMut(&mut Self::Meta) -> Result<(), String>,
    ) -> Result<Self::Meta, String> {
        FileSessionStorage::update_session_meta(self, app_data_dir, session_id, update)
    }

    fn update_session_meta_and_append_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: &mut dyn FnMut(&mut Self::Meta) -> Result<(), String>,
        events: &[Self::Event],
    ) -> Result<Self::Meta, String> {
        FileSessionStorage::update_session_meta_and_append_session_events(
            self,
            app_data_dir,
            session_id,
            update,
            events,
        )
    }

    fn save_full_session_for_migration_or_restore(
        &self,
        app_data_dir: &Path,
        session: &Self::Session,
    ) -> Result<(), String> {
        FileSessionStorage::save_full_session_for_migration_or_restore(self, app_data_dir, session)
    }

    fn append_message(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message: &Self::Message,
    ) -> Result<Self::Meta, String> {
        FileSessionStorage::append_message(self, app_data_dir, session_id, message)
    }

    fn persist_message_parts(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[Self::MessagePart],
        streaming_final_seq: u64,
        completed_at: Option<f64>,
    ) -> Result<Vec<Self::MessagePart>, String> {
        FileSessionStorage::persist_message_parts(
            self,
            app_data_dir,
            session_id,
            message_id,
            parts,
            streaming_final_seq,
            completed_at,
        )
    }

    fn append_session_event(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: &Self::Event,
    ) -> Result<Vec<Self::Event>, String> {
        FileSessionStorage::append_session_event(self, app_data_dir, session_id, event)
    }

    fn append_session_event_without_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: &Self::Event,
    ) -> Result<(), String> {
        FileSessionStorage::append_session_event_without_projection(
            self,
            app_data_dir,
            session_id,
            event,
        )
    }

    fn commit_session_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[Self::Event],
        prepare: &mut dyn crate::domain::agent_session::AgentSessionProjectionPreparer<
            Self::Event,
            Self::Meta,
            Self::Message,
            Self::MessagePart,
        >,
    ) -> Result<Vec<Self::MessagePart>, String> {
        FileSessionStorage::commit_session_projection(
            self,
            app_data_dir,
            session_id,
            events,
            prepare,
        )
    }

    fn append_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[Self::Event],
    ) -> Result<(), String> {
        FileSessionStorage::append_session_events(self, app_data_dir, session_id, events)
    }
}
