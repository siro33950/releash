use crate::domain::pty_session::{entities::PtySessionSnapshot, PtyKind};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PtySessionInfo {
    pub pty_id: u64,
    pub session_key: String,
    pub worktree_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub kind: String,
}

impl From<PtySessionSnapshot> for PtySessionInfo {
    fn from(snapshot: PtySessionSnapshot) -> Self {
        Self {
            pty_id: snapshot.pty_id,
            session_key: snapshot.session_key,
            worktree_path: snapshot.worktree_path,
            label: snapshot.label,
            kind: pty_kind_to_wire(snapshot.kind).to_string(),
        }
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyReplayOutput {
    pub pty_id: u64,
    pub data: String,
    pub sequence: u64,
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
    pub kind: String,
}

pub fn pty_kind_to_wire(kind: PtyKind) -> &'static str {
    match kind {
        PtyKind::Terminal => "terminal",
        PtyKind::OneShot => "one_shot",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_kind_dto_serializes_as_snake_case_wire_value() {
        let terminal = GetOrSpawnPtyResult {
            pty_id: 1,
            session_key: "key".to_string(),
            buffered_output: String::new(),
            buffered_output_sequence: 0,
            is_new: true,
            is_exited: false,
            exit_code: None,
            label: None,
            kind: pty_kind_to_wire(PtyKind::Terminal).to_string(),
        };
        let one_shot = GetOrSpawnPtyResult {
            kind: pty_kind_to_wire(PtyKind::OneShot).to_string(),
            ..terminal.clone()
        };

        assert_eq!(serde_json::to_value(terminal).unwrap()["kind"], "terminal");
        assert_eq!(serde_json::to_value(one_shot).unwrap()["kind"], "one_shot");
    }
}
