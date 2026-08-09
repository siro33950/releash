use std::time::Duration;

use super::{TerminalActivity, TERMINAL_ACTIVITY_RUNNING_WINDOW};

#[test]
fn test_terminal_activity_閾値未満の出力recencyはrunningに分類する() {
    assert_eq!(
        TerminalActivity::classify(Some(Duration::ZERO)),
        TerminalActivity::Running
    );
    assert_eq!(
        TerminalActivity::classify(Some(
            TERMINAL_ACTIVITY_RUNNING_WINDOW - Duration::from_millis(1)
        )),
        TerminalActivity::Running
    );
}

#[test]
fn test_terminal_activity_閾値以上の経過はidleに分類する() {
    assert_eq!(
        TerminalActivity::classify(Some(TERMINAL_ACTIVITY_RUNNING_WINDOW)),
        TerminalActivity::Idle
    );
    assert_eq!(
        TerminalActivity::classify(Some(Duration::from_secs(60))),
        TerminalActivity::Idle
    );
}

#[test]
fn test_terminal_activity_出力未観測はidleに分類する() {
    assert_eq!(TerminalActivity::classify(None), TerminalActivity::Idle);
}
