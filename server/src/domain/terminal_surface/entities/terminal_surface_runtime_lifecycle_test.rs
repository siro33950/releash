use super::TerminalSurfaceRuntimeLifecycle;

#[test]
fn test_ターミナル実行環境ライフサイクル_実行環境idで識別する() {
    let lifecycle = TerminalSurfaceRuntimeLifecycle::new("application-process".to_string());

    assert_eq!(lifecycle.runtime_id(), "application-process");
}

#[test]
fn test_ターミナル実行環境ライフサイクル_通常稼働中は変更を受理する() {
    let lifecycle = TerminalSurfaceRuntimeLifecycle::new("application-process".to_string());

    assert!(lifecycle.admit_mutation().is_ok());
}

#[test]
fn test_ターミナル実行環境ライフサイクル_終了開始後は変更を再受理しない() {
    let mut lifecycle = TerminalSurfaceRuntimeLifecycle::new("application-process".to_string());

    lifecycle.begin_shutdown();
    lifecycle.begin_shutdown();

    assert!(lifecycle.admit_mutation().is_err());
}
