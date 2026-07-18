use std::collections::HashMap;
use std::path::Path;

pub trait AgentSessionStorageTypes: Send + Sync {
    type Session;
    type Meta;
    type PageCursor;
    type Page;
    type Message;
    type MessagePart;
    type Attachment;
    type ToolOutput;
    type Event;
}

pub trait AgentSessionReader: AgentSessionStorageTypes {
    fn list_metas(&self, app_data_dir: &Path) -> Result<Vec<Self::Meta>, String>;

    fn session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String>;

    fn session_titles(&self, app_data_dir: &Path) -> Result<HashMap<String, String>, String>;

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

    fn load_previous_human_message_before_agent(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_message_id: &str,
    ) -> Result<Option<Self::Message>, String>;

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

    fn get_session_tool_output(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        tool_output_id: &str,
    ) -> Result<Option<Self::ToolOutput>, String>;

    fn load_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<Self::Event>, String>;
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

    /// Session meta の RMW と複数 event の追記を、同じ local storage lock の
    /// commit boundary で確定する。
    fn update_session_meta_and_append_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: &mut dyn FnMut(&mut Self::Meta) -> Result<(), String>,
        events: &[Self::Event],
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
    ) -> Result<Self::Meta, String>;

    fn persist_message_parts(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[Self::MessagePart],
        streaming_final_seq: u64,
        completed_at: Option<f64>,
    ) -> Result<Vec<Self::MessagePart>, String>;

    fn append_session_event(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: &Self::Event,
    ) -> Result<Vec<Self::Event>, String>;

    fn append_session_event_without_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: &Self::Event,
    ) -> Result<(), String>;
}

pub trait AgentSessionStorage: AgentSessionReader + AgentSessionWriter {}

impl<T> AgentSessionStorage for T where T: AgentSessionReader + AgentSessionWriter + ?Sized {}
