pub const TERMINAL_SURFACE_SCROLLBACK_ROWS: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceCheckpoint {
    pub replay: String,
    pub sequence: u64,
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSurfaceCheckpoint {
    pub fn empty(cols: u16, rows: u16) -> Self {
        Self {
            replay: String::new(),
            sequence: 0,
            cols,
            rows,
        }
    }
}
