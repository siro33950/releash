use crate::domain::pty_session::entities::PtySessionSnapshot;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PtySessionInfo {
    pub pty_id: u64,
    pub session_key: String,
    pub worktree_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl From<PtySessionSnapshot> for PtySessionInfo {
    fn from(snapshot: PtySessionSnapshot) -> Self {
        Self {
            pty_id: snapshot.pty_id,
            session_key: snapshot.session_key,
            worktree_path: snapshot.worktree_path,
            label: snapshot.label,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PtySessionAvailability {
    pub unavailable_session_keys: Vec<String>,
}

pub struct FoundPtySession {
    pub snapshot: PtySessionSnapshot,
    pub buffered_output: String,
    pub buffered_output_sequence: u64,
}

#[derive(Clone, serde::Serialize)]
pub struct GetPtyBufferedOutputResult {
    pub pty_id: u64,
    pub session_key: String,
    pub buffered_output: String,
    pub buffered_output_sequence: u64,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
}

#[derive(Clone, serde::Serialize)]
pub struct GetOrSpawnPtyResult {
    pub pty_id: u64,
    pub session_key: String,
    pub buffered_output: String,
    pub buffered_output_sequence: u64,
    pub is_new: bool,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
