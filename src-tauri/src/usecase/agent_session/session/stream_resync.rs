use async_trait::async_trait;

use super::MessagePart;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StreamResyncSnapshot {
    pub(crate) session_id: String,
    pub(crate) message_id: String,
    pub(crate) seq: u64,
    pub(crate) parts: Vec<MessagePart>,
}

#[async_trait]
pub(crate) trait AgentStreamResyncReadModel: Send + Sync {
    async fn resync_streaming_message(
        &self,
        session_id: &str,
        message_id: &str,
        since_seq: u64,
    ) -> Result<Option<StreamResyncSnapshot>, String>;
}

pub(crate) async fn resync_streaming_message(
    read_model: &dyn AgentStreamResyncReadModel,
    session_id: &str,
    message_id: &str,
    since_seq: u64,
) -> Result<Option<StreamResyncSnapshot>, String> {
    read_model
        .resync_streaming_message(session_id, message_id, since_seq)
        .await
}
