use std::time::Duration;

/// 出力recencyに基づく実行状態分類の閾値。最終出力からの経過が
/// この時間未満なら running、それ以外は idle とする。
pub const TERMINAL_ACTIVITY_RUNNING_WINDOW: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalActivity {
    Running,
    Idle,
}

impl TerminalActivity {
    /// 最終出力からの経過時間で running / idle を分類する。
    /// `None`（出力未観測）は idle。
    pub fn classify(elapsed_since_output: Option<Duration>) -> Self {
        match elapsed_since_output {
            Some(elapsed) if elapsed < TERMINAL_ACTIVITY_RUNNING_WINDOW => Self::Running,
            _ => Self::Idle,
        }
    }
}

#[cfg(test)]
#[path = "terminal_activity_test.rs"]
mod terminal_activity_tests;
