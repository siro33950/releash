use std::time::Duration;

use crate::domain::notification::services::{extract_branch, is_discord_webhook};
use crate::domain::notification::{
    AgentNotificationState, NotificationError, NotificationEvent, WebhookSenderGateway,
};

pub struct ReqwestWebhookSenderGateway;

fn build_slack_payload(event: &NotificationEvent) -> serde_json::Value {
    let branch = extract_branch(&event.worktree_path);

    let text = match event.state {
        AgentNotificationState::Running => format!(":hourglass: Agent started on `{branch}`"),
        AgentNotificationState::Done => {
            let code = event.exit_code.unwrap_or(0);
            format!(":white_check_mark: Agent completed on `{branch}` (exit code: {code})")
        }
        AgentNotificationState::Error => {
            let code = event.exit_code.unwrap_or(1);
            format!(":x: Agent failed on `{branch}` (exit code: {code})")
        }
        AgentNotificationState::Waiting => format!(":bell: Agent waiting for input on `{branch}`"),
    };

    serde_json::json!({
        "text": &text,
        "content": &text,
    })
}

fn build_discord_payload(event: &NotificationEvent) -> serde_json::Value {
    let branch = extract_branch(&event.worktree_path);

    let (description, color) = match event.state {
        AgentNotificationState::Running => {
            (format!("\u{23f3} Agent started on `{branch}`"), 3447003)
        }
        AgentNotificationState::Done => {
            let code = event.exit_code.unwrap_or(0);
            (
                format!("\u{2705} Agent completed on `{branch}` (exit code: {code})"),
                3066993,
            )
        }
        AgentNotificationState::Error => {
            let code = event.exit_code.unwrap_or(1);
            (
                format!("\u{274c} Agent failed on `{branch}` (exit code: {code})"),
                15158332,
            )
        }
        AgentNotificationState::Waiting => (
            format!("\u{1f514} Agent waiting for input on `{branch}`"),
            15965202,
        ),
    };

    serde_json::json!({
        "embeds": [{
            "description": description,
            "color": color,
        }]
    })
}

fn build_payload(url: &str, event: &NotificationEvent) -> serde_json::Value {
    if is_discord_webhook(url) {
        build_discord_payload(event)
    } else {
        build_slack_payload(event)
    }
}

#[async_trait::async_trait]
impl WebhookSenderGateway for ReqwestWebhookSenderGateway {
    async fn send(&self, url: &str, event: NotificationEvent) -> Result<(), NotificationError> {
        let payload = build_payload(url, &event);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| NotificationError::Message(format!("Webhook client build error: {e}")))?;

        client
            .post(url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| NotificationError::Message(format!("Webhook send error: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(state: AgentNotificationState, exit_code: Option<i32>) -> NotificationEvent {
        NotificationEvent {
            worktree_path: "/repos/worktrees/feature-x".to_string(),
            state,
            exit_code,
            timestamp: 0.0,
            session_id: None,
            pty_id: None,
        }
    }

    #[test]
    fn slack_done_payload_contains_exit_code() {
        let payload = build_slack_payload(&make_event(AgentNotificationState::Done, Some(0)));
        assert!(payload["text"].as_str().unwrap().contains("exit code: 0"));
    }

    #[test]
    fn discord_error_payload_uses_error_color() {
        let payload = build_discord_payload(&make_event(AgentNotificationState::Error, Some(1)));
        assert_eq!(payload["embeds"][0]["color"], 15158332);
    }
}
