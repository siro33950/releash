use super::TerminalSurfaceStartupCommand;

#[test]
fn test_ターミナル画面起動コマンド_新規生成時だけ改行付き入力にする() {
    let fresh = TerminalSurfaceStartupCommand::new(Some("  cargo test  "))
        .and_then(|command| command.into_input(false));
    let restored = TerminalSurfaceStartupCommand::new(Some("cargo test"))
        .and_then(|command| command.into_input(true));

    assert_eq!(fresh.as_deref(), Some("cargo test\n"));
    assert!(restored.is_none());
}

#[test]
fn test_ターミナル画面起動コマンド_空入力を実行しない() {
    assert!(TerminalSurfaceStartupCommand::new(None).is_none());
    assert!(TerminalSurfaceStartupCommand::new(Some("  ")).is_none());
}
