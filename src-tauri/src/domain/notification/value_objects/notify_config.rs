use crate::domain::notification::DesktopNotifyMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyConfig {
    pub webhook_url: String,
    pub on_running: bool,
    pub on_done: bool,
    pub on_error: bool,
    pub on_waiting: bool,
    pub desktop_mode: DesktopNotifyMode,
    pub inactive_timeout_minutes: u32,
}
