use super::message::Message;

#[allow(dead_code)]
// issues-1301 G-1: domain entity retained for backend-boundary migration while legacy session DTO projection is still being collapsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub id: String,
    pub worktree_path: String,
    pub messages: Vec<Message>,
    pub state: SessionState,
    pub created_at: f64,
    pub updated_at: f64,
    pub agent_session_id: Option<String>,
    pub context_carry: Option<ContextCarryState>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub selected_model: Option<String>,
    pub permission_profile_id: Option<String>,
    pub backend_id: String,
}

#[allow(dead_code)]
// issues-1301 G-1: domain entity retained for backend-boundary migration while legacy session DTO projection is still being collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Active,
    Idle,
    Done,
    Error,
    Closed,
    Archived,
}

#[allow(dead_code)]
// issues-1301 G-1: domain entity retained for backend-boundary migration while legacy session DTO projection is still being collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextCarryState {
    Resumed,
    Reinjected,
    Failed,
}
