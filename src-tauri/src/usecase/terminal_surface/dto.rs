#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalSurfaceCheckpointDto {
    pub replay: String,
    pub sequence: u64,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalSurfaceDto {
    pub session_key: String,
    pub checkpoint: TerminalSurfaceCheckpointDto,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalSurfaceSummaryDto {
    pub session_key: String,
    pub worktree_path: Option<String>,
    pub label: Option<String>,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GetOrSpawnTerminalDto {
    pub session_key: String,
    pub restored_from_checkpoint: bool,
    pub is_new: bool,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
}
