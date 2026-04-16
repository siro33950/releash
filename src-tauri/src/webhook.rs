use std::time::Duration;

use crate::config::{DesktopNotifyMode, NotifySection};
use crate::focus_tracker::FocusTracker;
use crate::protocol::{AgentState, AgentStateSync};

/// 通知を送出すべきかを判定する。
/// - state ごとの on_* フラグで有効/無効
/// - desktop_mode が WhenInactive の場合、focus_tracker.is_inactive() を併用
pub fn should_notify(
    notify: &NotifySection,
    state: &AgentState,
    focus_tracker: &parking_lot::Mutex<FocusTracker>,
) -> bool {
    let enabled = match state {
        AgentState::Running => notify.on_running,
        AgentState::Done => notify.on_done,
        AgentState::Error => notify.on_error,
        AgentState::Waiting => notify.on_waiting,
    };
    if !enabled {
        return false;
    }

    match notify.desktop_mode {
        DesktopNotifyMode::Always => true,
        DesktopNotifyMode::WhenInactive => focus_tracker
            .lock()
            .is_inactive(notify.inactive_timeout_minutes),
    }
}

pub fn is_discord_webhook(url: &str) -> bool {
    url.contains("discord.com/api/webhooks/") || url.contains("discordapp.com/api/webhooks/")
}

pub fn extract_branch(worktree_path: &str) -> &str {
    worktree_path.rsplit('/').next().unwrap_or(worktree_path)
}

pub fn build_slack_payload(event: &AgentStateSync) -> serde_json::Value {
    let branch = extract_branch(&event.worktree_path);

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

    serde_json::json!({
        "text": &text,
        "content": &text,
    })
}

pub fn build_discord_payload(event: &AgentStateSync) -> serde_json::Value {
    let branch = extract_branch(&event.worktree_path);

    let (description, color) = match event.state {
        AgentState::Running => (
            format!("\u{23f3} Agent started on `{branch}`"),
            3447003, // 0x3498DB
        ),
        AgentState::Done => {
            let code = event.exit_code.unwrap_or(0);
            (
                format!("\u{2705} Agent completed on `{branch}` (exit code: {code})"),
                3066993, // 0x2ECC71
            )
        }
        AgentState::Error => {
            let code = event.exit_code.unwrap_or(1);
            (
                format!("\u{274c} Agent failed on `{branch}` (exit code: {code})"),
                15158332, // 0xE74C3C
            )
        }
        AgentState::Waiting => (
            format!("\u{1f514} Agent waiting for input on `{branch}`"),
            15965202, // 0xF39C12
        ),
    };

    serde_json::json!({
        "embeds": [{
            "description": description,
            "color": color,
        }]
    })
}

pub fn build_payload(url: &str, event: &AgentStateSync) -> serde_json::Value {
    if is_discord_webhook(url) {
        build_discord_payload(event)
    } else {
        build_slack_payload(event)
    }
}

pub async fn send_webhook(url: &str, event: &AgentStateSync) {
    if url.is_empty() {
        return;
    }

    let payload = build_payload(url, event);

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

    // --- is_discord_webhook ---

    #[test]
    fn discord_com_url_detected() {
        assert!(is_discord_webhook(
            "https://discord.com/api/webhooks/123456/abcdef"
        ));
    }

    #[test]
    fn discordapp_com_url_detected() {
        assert!(is_discord_webhook(
            "https://discordapp.com/api/webhooks/123456/abcdef"
        ));
    }

    #[test]
    fn slack_url_not_discord() {
        assert!(!is_discord_webhook(
            "https://hooks.slack.com/services/T00/B00/xxx"
        ));
    }

    #[test]
    fn empty_url_not_discord() {
        assert!(!is_discord_webhook(""));
    }

    #[test]
    fn generic_url_not_discord() {
        assert!(!is_discord_webhook("https://example.com/webhook"));
    }

    // --- extract_branch ---

    #[test]
    fn extracts_last_segment() {
        assert_eq!(
            extract_branch("/repos/my-project-worktrees/feature-auth"),
            "feature-auth"
        );
    }

    #[test]
    fn single_segment_returns_itself() {
        assert_eq!(extract_branch("main"), "main");
    }

    #[test]
    fn root_slash_returns_empty() {
        assert_eq!(extract_branch("/"), "");
    }

    // --- build_slack_payload ---

    fn make_event(state: AgentState, exit_code: Option<i32>) -> AgentStateSync {
        AgentStateSync {
            worktree_path: "/repos/worktrees/feature-x".to_string(),
            state,
            exit_code,
            timestamp: 0.0,
            session_id: None,
            pty_id: None,
        }
    }

    #[test]
    fn slack_running_payload() {
        let event = make_event(AgentState::Running, None);
        let payload = build_slack_payload(&event);
        let text = payload["text"].as_str().unwrap();
        assert!(text.contains(":hourglass:"));
        assert!(text.contains("feature-x"));
        assert!(payload.get("content").is_some());
    }

    #[test]
    fn slack_done_payload() {
        let event = make_event(AgentState::Done, Some(0));
        let payload = build_slack_payload(&event);
        let text = payload["text"].as_str().unwrap();
        assert!(text.contains(":white_check_mark:"));
        assert!(text.contains("exit code: 0"));
    }

    #[test]
    fn slack_error_payload() {
        let event = make_event(AgentState::Error, Some(1));
        let payload = build_slack_payload(&event);
        let text = payload["text"].as_str().unwrap();
        assert!(text.contains(":x:"));
        assert!(text.contains("exit code: 1"));
    }

    #[test]
    fn slack_waiting_payload() {
        let event = make_event(AgentState::Waiting, None);
        let payload = build_slack_payload(&event);
        let text = payload["text"].as_str().unwrap();
        assert!(text.contains(":bell:"));
        assert!(text.contains("waiting for input"));
    }

    // --- build_discord_payload ---

    #[test]
    fn discord_running_payload() {
        let event = make_event(AgentState::Running, None);
        let payload = build_discord_payload(&event);
        let embed = &payload["embeds"][0];
        assert_eq!(embed["color"], 3447003);
        let desc = embed["description"].as_str().unwrap();
        assert!(desc.contains("\u{23f3}"));
        assert!(desc.contains("feature-x"));
    }

    #[test]
    fn discord_done_payload() {
        let event = make_event(AgentState::Done, Some(0));
        let payload = build_discord_payload(&event);
        let embed = &payload["embeds"][0];
        assert_eq!(embed["color"], 3066993);
        let desc = embed["description"].as_str().unwrap();
        assert!(desc.contains("\u{2705}"));
        assert!(desc.contains("exit code: 0"));
    }

    #[test]
    fn discord_error_payload() {
        let event = make_event(AgentState::Error, Some(1));
        let payload = build_discord_payload(&event);
        let embed = &payload["embeds"][0];
        assert_eq!(embed["color"], 15158332);
        let desc = embed["description"].as_str().unwrap();
        assert!(desc.contains("\u{274c}"));
        assert!(desc.contains("exit code: 1"));
    }

    #[test]
    fn discord_waiting_payload() {
        let event = make_event(AgentState::Waiting, None);
        let payload = build_discord_payload(&event);
        let embed = &payload["embeds"][0];
        assert_eq!(embed["color"], 15965202);
        let desc = embed["description"].as_str().unwrap();
        assert!(desc.contains("\u{1f514}"));
        assert!(desc.contains("waiting for input"));
    }

    // --- build_payload dispatch ---

    #[test]
    fn dispatches_to_discord_for_discord_url() {
        let event = make_event(AgentState::Running, None);
        let payload = build_payload("https://discord.com/api/webhooks/123/abc", &event);
        assert!(payload.get("embeds").is_some());
        assert!(payload.get("text").is_none());
    }

    #[test]
    fn dispatches_to_slack_for_slack_url() {
        let event = make_event(AgentState::Running, None);
        let payload = build_payload("https://hooks.slack.com/services/T00/B00/xxx", &event);
        assert!(payload.get("text").is_some());
        assert!(payload.get("embeds").is_none());
    }

    #[test]
    fn dispatches_to_slack_for_generic_url() {
        let event = make_event(AgentState::Running, None);
        let payload = build_payload("https://example.com/webhook", &event);
        assert!(payload.get("text").is_some());
        assert!(payload.get("embeds").is_none());
    }

    // --- send_webhook ---

    #[tokio::test]
    async fn empty_url_returns_immediately() {
        let event = make_event(AgentState::Running, None);
        send_webhook("", &event).await;
    }

    // --- should_notify ---

    fn make_notify(
        on_running: bool,
        on_done: bool,
        on_error: bool,
        on_waiting: bool,
    ) -> NotifySection {
        NotifySection {
            webhook_url: "https://example.com/hook".to_string(),
            on_running,
            on_done,
            on_error,
            on_waiting,
            desktop_mode: DesktopNotifyMode::Always,
            inactive_timeout_minutes: 2,
        }
    }

    #[test]
    fn should_notify_filters_disabled_state() {
        let ft = parking_lot::Mutex::new(FocusTracker::new());
        let notify = make_notify(false, true, true, true);
        assert!(!should_notify(&notify, &AgentState::Running, &ft));
        assert!(should_notify(&notify, &AgentState::Done, &ft));
        assert!(should_notify(&notify, &AgentState::Error, &ft));
        assert!(should_notify(&notify, &AgentState::Waiting, &ft));
    }

    #[test]
    fn should_notify_always_sends_when_focused() {
        let ft = parking_lot::Mutex::new(FocusTracker::new());
        let notify = make_notify(true, true, true, true);
        assert!(should_notify(&notify, &AgentState::Done, &ft));
    }

    #[test]
    fn should_notify_when_inactive_blocks_focused() {
        let ft = parking_lot::Mutex::new(FocusTracker::new());
        let notify = NotifySection {
            desktop_mode: DesktopNotifyMode::WhenInactive,
            ..make_notify(true, true, true, true)
        };
        // フォーカス中なのでblockされる
        assert!(!should_notify(&notify, &AgentState::Done, &ft));
    }

    #[test]
    fn should_notify_when_inactive_allows_after_timeout() {
        let ft = parking_lot::Mutex::new(FocusTracker::new());
        ft.lock().on_blur();
        let notify = NotifySection {
            desktop_mode: DesktopNotifyMode::WhenInactive,
            inactive_timeout_minutes: 0,
            ..make_notify(true, true, true, true)
        };
        assert!(should_notify(&notify, &AgentState::Done, &ft));
    }
}
