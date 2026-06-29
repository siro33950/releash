#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtySession {
    pub pty_id: u64,
    pub session_key: String,
    pub worktree_path: Option<String>,
    pub label: Option<String>,
    pub exited: bool,
    pub exit_code: Option<i32>,
}

impl PtySession {
    pub fn new(
        pty_id: u64,
        session_key: String,
        worktree_path: Option<String>,
        label: Option<String>,
    ) -> Self {
        Self {
            pty_id,
            session_key,
            worktree_path,
            label,
            exited: false,
            exit_code: None,
        }
    }

    pub fn mark_exited(&mut self, exit_code: Option<i32>) {
        self.exited = true;
        self.exit_code = exit_code;
    }

    pub fn snapshot(&self) -> PtySessionSnapshot {
        PtySessionSnapshot {
            pty_id: self.pty_id,
            session_key: self.session_key.clone(),
            worktree_path: self.worktree_path.clone(),
            label: self.label.clone(),
            exited: self.exited,
            exit_code: self.exit_code,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtySessionSnapshot {
    pub pty_id: u64,
    pub session_key: String,
    pub worktree_path: Option<String>,
    pub label: Option<String>,
    pub exited: bool,
    pub exit_code: Option<i32>,
}
