use std::path::Path;

use crate::domain::notification::{AgentNotificationState, DesktopNotifyMode, NotifyConfig};

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
    url.starts_with("https://discord.com/api/webhooks/")
        || url.starts_with("https://discordapp.com/api/webhooks/")
}

pub fn extract_branch(worktree_path: &str) -> &str {
    let file_name = Path::new(worktree_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(worktree_path);
    file_name.rsplit('\\').next().unwrap_or(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!is_discord_webhook(
            "https://example.com/discord.com/api/webhooks/123456/abcdef"
        ));
        assert!(!is_discord_webhook(
            "https://example.com/https://discord.com/api/webhooks/123456/abcdef"
        ));
        assert!(!is_discord_webhook(
            "https://discord.com.example/api/webhooks/123456/abcdef"
        ));
    }

    #[test]
    fn extract_branch_supports_unix_and_windows_paths() {
        assert_eq!(extract_branch("/repos/worktrees/feature-x"), "feature-x");
        assert_eq!(extract_branch(r"C:\repos\worktrees\feature-x"), "feature-x");
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
