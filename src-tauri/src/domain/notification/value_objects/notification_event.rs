#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentNotificationState {
    Running,
    Done,
    Error,
    Waiting,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NotificationEvent {
    pub worktree_path: String,
    pub state: AgentNotificationState,
    pub exit_code: Option<i32>,
    pub timestamp: f64,
    pub session_id: Option<String>,
    pub pty_id: Option<String>,
}
