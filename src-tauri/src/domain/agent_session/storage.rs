use std::path::Path;

pub trait AgentSessionStorageTypes: Send + Sync {
    type Session;
    type Meta;
    type PageCursor;
    type Page;
    type Message;
    type MessagePart;
    type Attachment;
}

pub trait AgentSessionReader: AgentSessionStorageTypes {
    fn list_metas(&self, app_data_dir: &Path) -> Result<Vec<Self::Meta>, String>;

    fn session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String>;

    fn get_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Meta>, String>;

    fn load_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<Self::Session>, String>;

    fn get_session_page(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        cursor: Option<Self::PageCursor>,
        limit: usize,
    ) -> Result<Option<Self::Page>, String>;

    fn get_session_attachment(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<Self::Attachment>, String>;
}

pub trait AgentSessionWriter: AgentSessionStorageTypes {
    fn write_session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<(), String>;

    fn fork_session_layout(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        forked_meta: &Self::Meta,
    ) -> Result<(), String>;

    fn remove_session(&self, app_data_dir: &Path, session_id: &str);

    /// 原子的に session meta を read-modify-write する。
    /// ストレージ実装内で file lock を取得した状態で disk から meta を読み、
    /// クロージャを適用した結果を同じ lock 内で書き戻す。
    /// SessionStore 層からの並行 RMW で lost update が発生しないことを保証する。
    fn update_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: &mut dyn FnMut(&mut Self::Meta) -> Result<(), String>,
    ) -> Result<Self::Meta, String>;

    fn save_full_session_for_migration_or_restore(
        &self,
        app_data_dir: &Path,
        session: &Self::Session,
    ) -> Result<(), String>;

    fn append_message(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message: &Self::Message,
    ) -> Result<(), String>;

    fn persist_message_parts(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[Self::MessagePart],
        completed_at: Option<f64>,
    ) -> Result<(), String>;
}

pub trait AgentSessionStorage: AgentSessionReader + AgentSessionWriter {}

impl<T> AgentSessionStorage for T where T: AgentSessionReader + AgentSessionWriter + ?Sized {}
