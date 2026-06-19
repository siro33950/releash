use crate::domain::pty_session::{entities::PtySessionSnapshot, PtyKind};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PtySessionInfo {
    pub pty_id: u64,
    pub session_key: String,
    pub worktree_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub kind: PtyKind,
}

impl From<PtySessionSnapshot> for PtySessionInfo {
    fn from(snapshot: PtySessionSnapshot) -> Self {
        Self {
            pty_id: snapshot.pty_id,
            session_key: snapshot.session_key,
            worktree_path: snapshot.worktree_path,
            label: snapshot.label,
            kind: snapshot.kind,
        }
    }
}

pub struct FoundPtySession {
    pub snapshot: PtySessionSnapshot,
    pub buffered_output: String,
}

#[derive(serde::Serialize)]
pub struct GetOrSpawnPtyResult {
    pub pty_id: u64,
    pub session_key: String,
    pub buffered_output: String,
    pub is_new: bool,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub kind: PtyKind,
}
