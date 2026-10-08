use super::{TerminalProcessLaunch, TerminalProcessLaunchError};

#[test]
fn test_terminal_process_launch_構造化された実行情報を保持する() {
    let launch = TerminalProcessLaunch::new(
        "/usr/bin/provider",
        vec!["resume".to_string(), "provider-session-1".to_string()],
        vec![("RELEASH_BINDING_ID".to_string(), "binding-1".to_string())],
    )
    .unwrap();

    assert_eq!(launch.executable(), "/usr/bin/provider");
    assert_eq!(
        launch.arguments(),
        &["resume".to_string(), "provider-session-1".to_string()]
    );
    assert_eq!(
        launch.environment(),
        &[("RELEASH_BINDING_ID".to_string(), "binding-1".to_string())]
    );
}

#[test]
fn test_terminal_process_launch_実行ファイルと環境変数keyの不正を拒否する() {
    assert_eq!(
        TerminalProcessLaunch::new(" ", Vec::new(), Vec::new()).unwrap_err(),
        TerminalProcessLaunchError::ExecutableMissing
    );
    assert_eq!(
        TerminalProcessLaunch::new(
            "provider",
            Vec::new(),
            vec![(" ".to_string(), "value".to_string())],
        )
        .unwrap_err(),
        TerminalProcessLaunchError::EnvironmentKeyMissing
    );
    assert_eq!(
        TerminalProcessLaunch::new(
            "provider",
            Vec::new(),
            vec![
                ("RELEASH_SLOT_ID".to_string(), "slot-1".to_string()),
                ("RELEASH_SLOT_ID".to_string(), "slot-2".to_string()),
            ],
        )
        .unwrap_err(),
        TerminalProcessLaunchError::EnvironmentKeyDuplicate
    );
}
