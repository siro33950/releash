use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use parking_lot::RwLock;

use crate::usecase::agent_session::session::{
    ChatMessage, ChatSession, MessagePart, PageCursor, SessionAttachment, SessionMeta, SessionPage,
};

mod attachment_blob;
mod fork_copier;
mod layout;
mod legacy;
mod message_store;
mod meta_repository;
mod titles;

#[cfg(test)]
mod tests;

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

    fn write_session_meta(&self, app_data_dir: &Path, meta: &Self::Meta) -> Result<(), String> {
        FileSessionStorage::write_session_meta(self, app_data_dir, meta)
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
    ) -> Result<(), String> {
        FileSessionStorage::append_message(self, app_data_dir, session_id, message)
    }

    fn persist_message_parts(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[Self::MessagePart],
        completed_at: Option<f64>,
    ) -> Result<(), String> {
        FileSessionStorage::persist_message_parts(
            self,
            app_data_dir,
            session_id,
            message_id,
            parts,
            completed_at,
        )
    }
}
