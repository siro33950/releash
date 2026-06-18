use crate::domain::notification::services::{build_payload, should_notify};
use crate::domain::notification::{NotificationEvent, NotifyConfig, WebhookSenderGateway};

pub async fn on_agent_status_changed(
    notify: NotifyConfig,
    inactive: bool,
    event: NotificationEvent,
    sender: &dyn WebhookSenderGateway,
) {
    let url = notify.webhook_url.clone();
    if url.is_empty() || !should_notify(&notify, &event.state, inactive) {
        return;
    }
    let payload = build_payload(&url, &event);
    sender.send(&url, payload).await;
}
