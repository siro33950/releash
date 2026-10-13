use super::*;

#[test]
fn test_前面処理_shell以外のプロセスグループだけを処理中とする() {
    // Given / When / Then
    assert!(!TerminalShellState::has_foreground_process(
        10,
        10,
        "zsh",
        "zsh",
        &[]
    ));
    assert!(TerminalShellState::has_foreground_process(
        10,
        11,
        "zsh",
        "zsh",
        &[]
    ));
    assert!(!TerminalShellState::has_foreground_process(
        10,
        0,
        "zsh",
        "zsh",
        &[]
    ));
    assert!(TerminalShellState::has_foreground_process(
        10,
        10,
        "zsh",
        "sleep",
        &[]
    ));
}

#[test]
fn test_前面処理_shellと同じgroupの子も確認対象とする() {
    assert!(TerminalShellState::has_foreground_process(
        10,
        10,
        "bash",
        "bash",
        &[10]
    ));
    assert!(!TerminalShellState::has_foreground_process(
        10,
        10,
        "bash",
        "bash",
        &[11]
    ));
}
