use std::time::Duration;

use crate::protocol::{AgentState, AgentStateSync};

pub async fn send_webhook(url: &str, event: &AgentStateSync) {
    if url.is_empty() {
        return;
    }

    let branch = event
        .worktree_path
        .rsplit('/')
        .next()
        .unwrap_or(&event.worktree_path);

    let text = match event.state {
        AgentState::Running => format!(":hourglass: Agent started on `{branch}`"),
        AgentState::Done => {
            let code = event.exit_code.unwrap_or(0);
            format!(":white_check_mark: Agent completed on `{branch}` (exit code: {code})")
        }
        AgentState::Error => {
            let code = event.exit_code.unwrap_or(1);
            format!(":x: Agent failed on `{branch}` (exit code: {code})")
        }
        AgentState::Waiting => format!(":bell: Agent waiting for input on `{branch}`"),
    };

    let payload = serde_json::json!({
        "text": &text,
        "content": &text,
    });

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_text_running() {
        let event = AgentStateSync {
            worktree_path: "/repos/my-project-worktrees/feature-auth".to_string(),
            state: AgentState::Running,
            exit_code: None,
            timestamp: 0.0,
            session_id: None,
        };
        let branch = event
            .worktree_path
            .rsplit('/')
            .next()
            .unwrap_or(&event.worktree_path);
        assert_eq!(branch, "feature-auth");
    }

    #[test]
    fn webhook_text_done() {
        let event = AgentStateSync {
            worktree_path: "/repo".to_string(),
            state: AgentState::Done,
            exit_code: Some(0),
            timestamp: 0.0,
            session_id: None,
        };
        let code = event.exit_code.unwrap_or(0);
        assert_eq!(code, 0);
    }

    #[test]
    fn webhook_text_error() {
        let event = AgentStateSync {
            worktree_path: "/repo".to_string(),
            state: AgentState::Error,
            exit_code: Some(1),
            timestamp: 0.0,
            session_id: None,
        };
        let code = event.exit_code.unwrap_or(1);
        assert_eq!(code, 1);
    }

    #[tokio::test]
    async fn empty_url_returns_immediately() {
        let event = AgentStateSync {
            worktree_path: "/repo".to_string(),
            state: AgentState::Running,
            exit_code: None,
            timestamp: 0.0,
            session_id: None,
        };
        send_webhook("", &event).await;
    }
}
