use releashd::test_support::integration::installation::{
    cli_link, create_cli_link, read_only, run_admin_command, CliLink,
};
#[test]
fn test_cli設置_os上でリンクを作り古いリンクを置き換えファイルを保護する() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let link = tmp.path().join("bin/releash");
    let target = tmp.path().join("new/releash");
    // When / Then
    assert_eq!(cli_link(&link).unwrap(), CliLink::Missing);
    create_cli_link(&target, &link).unwrap();
    assert_eq!(cli_link(&link).unwrap(), CliLink::Symlink(target.clone()));
    let replacement = tmp.path().join("replacement/releash");
    create_cli_link(&replacement, &link).unwrap();
    assert_eq!(cli_link(&link).unwrap(), CliLink::Symlink(replacement));
    std::fs::remove_file(&link).unwrap();
    std::fs::write(&link, "user command").unwrap();
    assert_eq!(cli_link(&link).unwrap(), CliLink::Other);
    assert!(create_cli_link(&target, &link).is_err());
    assert_eq!(std::fs::read_to_string(link).unwrap(), "user command");
}
#[test]
fn test_配置観測_現在の実行ファイルと取得失敗を返す() {
    // Given / When / Then
    assert!(!read_only(&std::env::current_exe().unwrap()).unwrap());
    assert!(read_only(std::path::Path::new("/no-such-releash-bundle/releashd")).is_err());
}
#[cfg(target_os = "macos")]
#[test]
fn test_配置観測_読み取り専用ボリュームを検出する() {
    // Given / When / Then
    assert!(read_only(std::path::Path::new(
        "/System/Library/CoreServices/SystemVersion.plist"
    ))
    .unwrap());
}

#[test]
fn test_管理者設置_期限切れと取り消しで子を回収しリンクを作らない() {
    use releashd::test_support::integration::platform::{sync_scope, Deadline, OperationContext};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    // Given
    for duration in [Duration::ZERO, Duration::from_millis(30)] {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("releash");
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args(["-c", "sleep 0.3; ln -s /target \"$1\"", "installation"])
            .arg(&link);
        let context =
            OperationContext::default().with_deadline(Deadline::new(Instant::now() + duration));
        // When
        let result = sync_scope(context, || run_admin_command(&mut command));
        // Then
        assert_eq!(result.unwrap_err(), "Operation deadline exceeded");
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(cli_link(&link).unwrap(), CliLink::Missing);
    }
    // Given
    let cancellation = tokio_util::sync::CancellationToken::new();
    cancellation.cancel();
    let context = OperationContext::new(None, Arc::new(cancellation));
    let mut command = std::process::Command::new("/bin/sh");
    command.args(["-c", "sleep 1"]);
    // When / Then
    assert_eq!(
        sync_scope(context, || run_admin_command(&mut command)).unwrap_err(),
        "Operation cancelled"
    );
}

#[test]
fn test_管理者設置_子の成功と失敗と起動失敗を返す() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let link = tmp.path().join("releash");
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", "ln -s /target \"$1\"", "installation"])
        .arg(&link);
    // When / Then
    run_admin_command(&mut command).unwrap();
    assert_eq!(cli_link(&link).unwrap(), CliLink::Symlink("/target".into()));
    for (script, succeeds) in [("exit 0", true), ("exit 7", false)] {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", script]);
        assert_eq!(run_admin_command(&mut command).is_ok(), succeeds);
    }
    assert!(run_admin_command(&mut std::process::Command::new(
        "/no-such-administrator-command"
    ))
    .is_err());
}
