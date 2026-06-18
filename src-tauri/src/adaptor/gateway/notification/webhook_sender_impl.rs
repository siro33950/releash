use std::time::Duration;

use crate::domain::notification::WebhookSenderGateway;

pub struct ReqwestWebhookSenderGateway;

#[async_trait::async_trait]
impl WebhookSenderGateway for ReqwestWebhookSenderGateway {
    async fn send(&self, url: &str, payload: serde_json::Value) {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build();

        let client = match client {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Webhook client build error: {e}");
                return;
            }
        };

        if let Err(e) = client.post(url).json(&payload).send().await {
            log::warn!("Webhook send error: {e}");
        }
    }
}
