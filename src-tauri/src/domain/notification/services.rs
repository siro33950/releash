use crate::domain::notification::{
    AgentNotificationState, DesktopNotifyMode, NotificationEvent, NotifyConfig,
};

pub fn should_notify(
    notify: &NotifyConfig,
    state: &AgentNotificationState,
    inactive: bool,
) -> bool {
    let enabled = match state {
        AgentNotificationState::Running => notify.on_running,
        AgentNotificationState::Done => notify.on_done,
        AgentNotificationState::Error => notify.on_error,
        AgentNotificationState::Waiting => notify.on_waiting,
    };
    if !enabled {
        return false;
    }

    match notify.desktop_mode {
        DesktopNotifyMode::Always => true,
        DesktopNotifyMode::WhenInactive => inactive,
    }
}

pub fn is_discord_webhook(url: &str) -> bool {
    url.contains("discord.com/api/webhooks/") || url.contains("discordapp.com/api/webhooks/")
}

pub fn extract_branch(worktree_path: &str) -> &str {
    worktree_path.rsplit('/').next().unwrap_or(worktree_path)
}

pub fn build_slack_payload(event: &NotificationEvent) -> serde_json::Value {
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

pub fn build_discord_payload(event: &NotificationEvent) -> serde_json::Value {
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

pub fn build_payload(url: &str, event: &NotificationEvent) -> serde_json::Value {
    if is_discord_webhook(url) {
        build_discord_payload(event)
    } else {
        build_slack_payload(event)
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

    fn make_notify(
        on_running: bool,
        on_done: bool,
        on_error: bool,
        on_waiting: bool,
    ) -> NotifyConfig {
        NotifyConfig {
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
    fn discord_urls_detected() {
        assert!(is_discord_webhook(
            "https://discord.com/api/webhooks/123456/abcdef"
        ));
        assert!(is_discord_webhook(
            "https://discordapp.com/api/webhooks/123456/abcdef"
        ));
        assert!(!is_discord_webhook(
            "https://hooks.slack.com/services/T00/B00/xxx"
        ));
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

    #[test]
    fn should_notify_filters_disabled_state() {
        let notify = make_notify(false, true, true, true);
        assert!(!should_notify(
            &notify,
            &AgentNotificationState::Running,
            true
        ));
        assert!(should_notify(&notify, &AgentNotificationState::Done, true));
    }

    #[test]
    fn should_notify_when_inactive_blocks_active_desktop() {
        let notify = NotifyConfig {
            desktop_mode: DesktopNotifyMode::WhenInactive,
            ..make_notify(true, true, true, true)
        };
        assert!(!should_notify(
            &notify,
            &AgentNotificationState::Done,
            false
        ));
        assert!(should_notify(&notify, &AgentNotificationState::Done, true));
    }
}
