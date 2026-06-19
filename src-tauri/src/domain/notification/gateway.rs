use crate::domain::notification::{NotificationError, NotificationEvent};

#[async_trait::async_trait]
pub trait WebhookSenderGateway: Send + Sync {
    async fn send(&self, url: &str, event: NotificationEvent) -> Result<(), NotificationError>;
}
