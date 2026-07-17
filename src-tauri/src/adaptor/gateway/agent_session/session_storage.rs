use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use parking_lot::RwLock;

use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::session::{
    ChatMessage, ChatSession, MessagePart, PageCursor, SessionAttachment, SessionMeta, SessionPage,
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
mod titles;
mod tool_output_blob;

#[cfg(test)]
mod tests;

pub(crate) use gc::SessionGcMetaRead;

pub struct FileSessionStorage {
    pub(super) cache: RwLock<HashMap<String, SessionMeta>>,
    /// 壊れた / 旧形式の session JSON を session_id 単位で隔離する。
    /// Spec issues-947: 1つの不正セッションで全体ロードを Err にせず、無関係な正常セッションの
    /// 一覧取得・取得は素通しさせる。値は API に返す汎化済みエラー文言（フルパス・serde 生メッセージは含まない）。
    pub(super) invalid_sessions: RwLock<HashMap<String, String>>,
    pub(super) file_lock: parking_lot::Mutex<()>,
    pub(super) loaded: AtomicBool,
    #[cfg(test)]
    pub(super) message_read_count: std::sync::atomic::AtomicUsize,
}

impl Default for FileSessionStorage {
    fn default() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            invalid_sessions: RwLock::new(HashMap::new()),
            file_lock: parking_lot::Mutex::new(()),
            loaded: AtomicBool::new(false),
            #[cfg(test)]
            message_read_count: std::sync::atomic::AtomicUsize::new(0),
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
}
