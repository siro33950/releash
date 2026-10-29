use super::*;

fn config() -> AppConfig {
    AppConfig::new(
        false,
        false,
        vec![],
        String::new(),
        String::new(),
        TelemetryConfig {
            crash_reporting: false,
            performance_telemetry: false,
        },
        WorkflowConfig {
            approval_auto_approve: false,
        },
        HashMap::new(),
        HashMap::new(),
    )
}

#[test]
fn test_terminal起動設定_nulを含む入力で既存設定を変えない() {
    // Given
    let mut app = AppConfig {
        auto_launch: false,
        start_minimized: false,
        repo_paths: vec![],
        external_editor: String::new(),
        terminal_startup_command: "echo ready".into(),
        ..config()
    };
    // When
    let result = app.update_terminal_startup_command("bad\0command".into());
    // Then
    assert!(result.is_err());
    assert_eq!(app.terminal_startup_command, "echo ready");
}

#[test]
fn test_追加_新規パス() {
    let mut app = config();

    let added = app.add_repo_path("/repo/a");
    assert!(added);
    assert_eq!(app.repo_paths(), vec!["/repo/a"]);
}

#[test]
fn test_追加_重複はfalse() {
    let mut app = config();

    app.add_repo_path("/repo/a");
    let added = app.add_repo_path("/repo/a");
    assert!(!added);
    assert_eq!(app.repo_paths().len(), 1);
}

#[test]
fn test_追加_正規化してから重複判定() {
    let mut app = config();

    app.add_repo_path("/repo/a/");
    let added = app.add_repo_path("/repo/a");
    assert!(!added);
}

#[test]
fn test_追加_空パスはfalse() {
    let mut app = config();

    let added = app.add_repo_path("");
    assert!(!added);
    assert!(app.repo_paths().is_empty());
}

#[test]
fn test_削除_既存() {
    let mut app = config();

    app.add_repo_path("/repo/a");
    let removed = app.remove_repo_path("/repo/a");
    assert!(removed);
    assert!(app.repo_paths().is_empty());
}

#[test]
fn test_削除_未存在はfalse() {
    let mut app = config();

    let removed = app.remove_repo_path("/repo/a");
    assert!(!removed);
}
