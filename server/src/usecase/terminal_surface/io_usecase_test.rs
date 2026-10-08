use super::*;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::test_helpers::FakePtyGateway;

#[test]
fn test_ターミナル画面_パス入力_引用符処理して結合後に書き込む() {
    let gateway = FakePtyGateway::new();
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();

    write_paths(
        &gateway,
        &owner,
        &[
            "/tmp/a.txt".to_string(),
            "/tmp/my file.txt".to_string(),
            "/tmp/it's.txt".to_string(),
        ],
    )
    .unwrap();

    assert_eq!(
        *gateway.writes.lock(),
        vec![(
            owner.stable_key(),
            "/tmp/a.txt '/tmp/my file.txt' '/tmp/it'\\''s.txt'".to_string()
        )]
    );
}

#[test]
fn test_ターミナル画面_パス入力_空配列では何もしない() {
    let gateway = FakePtyGateway::new();
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();

    write_paths(&gateway, &owner, &[]).unwrap();

    assert!(gateway.writes.lock().is_empty());
}
