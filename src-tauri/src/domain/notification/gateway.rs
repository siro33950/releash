#[async_trait::async_trait]
pub trait WebhookSenderGateway: Send + Sync {
    async fn send(&self, url: &str, payload: serde_json::Value);
}
