use super::{TerminalSurfaceOwner, TerminalSurfaceOwnerError};
use crate::domain::workspace_tree::WorkspaceIdentity;

#[test]
fn test_ターミナル画面所有者_ワークスペースパス正規化で同一識別子になる() {
    let first = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo/worktree/")).unwrap();
    let second =
        TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo//worktree")).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.workspace_identity().as_str(), "/repo/worktree");
    assert_eq!(first.stable_key(), second.stable_key());
}

#[test]
fn test_ターミナル画面所有者_所有者種別ごとに安定キーを隔離する() {
    let workspace = WorkspaceIdentity::new("/repo");
    let owners = [
        TerminalSurfaceOwner::workspace(workspace.clone()).unwrap(),
        TerminalSurfaceOwner::session(workspace, "shared-id").unwrap(),
    ];

    assert_ne!(owners[0].stable_key(), owners[1].stable_key());
}

#[test]
fn test_ターミナル画面所有者_長さ接頭辞で構成要素衝突を防ぐ() {
    let workspace = WorkspaceIdentity::new("/repo");
    let first = TerminalSurfaceOwner::session(workspace.clone(), "a:b").unwrap();
    let second = TerminalSurfaceOwner::session(workspace, "a").unwrap();

    assert_ne!(first.stable_key(), second.stable_key());
}

#[test]
fn test_ターミナル画面所有者_空白のみのワークスペースパスを拒否する() {
    assert_eq!(
        TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(" ")),
        Err(TerminalSurfaceOwnerError::WorkspacePathMissing)
    );
    assert_eq!(
        TerminalSurfaceOwner::session(WorkspaceIdentity::new(" "), "session-1"),
        Err(TerminalSurfaceOwnerError::WorkspacePathMissing)
    );
}

#[test]
fn test_ターミナル画面所有者_空白のみのセッションidを拒否する() {
    assert_eq!(
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "  "),
        Err(TerminalSurfaceOwnerError::SessionIdMissing)
    );
    assert_eq!(
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), ""),
        Err(TerminalSurfaceOwnerError::SessionIdMissing)
    );
}
