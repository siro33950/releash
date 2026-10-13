use super::*;

#[test]
fn test_前面処理_shell以外のプロセスグループだけを処理中とする() {
    // Given / When / Then
    assert!(!TerminalShellState::observed(None, 10, 10, "zsh", "zsh", &[]).has_foreground_process);
    assert!(
        TerminalShellState::observed(Some("/tmp".into()), 10, 11, "zsh", "zsh", &[])
            .has_foreground_process
    );
    assert!(!TerminalShellState::observed(None, 10, 0, "zsh", "zsh", &[]).has_foreground_process);
    assert!(TerminalShellState::observed(None, 10, 10, "zsh", "sleep", &[]).has_foreground_process);
}

#[test]
fn test_前面処理_shellと同じgroupの子も確認対象とする() {
    assert!(
        TerminalShellState::observed(None, 10, 10, "bash", "bash", &[10]).has_foreground_process
    );
    assert!(
        !TerminalShellState::observed(None, 10, 10, "bash", "bash", &[11]).has_foreground_process
    );
}
