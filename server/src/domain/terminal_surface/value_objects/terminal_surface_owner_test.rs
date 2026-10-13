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
        Err(TerminalSurfaceOwnerError::WorkspacePath)
    );
    assert_eq!(
        TerminalSurfaceOwner::session(WorkspaceIdentity::new(" "), "session-1"),
        Err(TerminalSurfaceOwnerError::WorkspacePath)
    );
}

#[test]
fn test_ターミナル画面所有者_空白のみのセッションidを拒否する() {
    assert_eq!(
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "  "),
        Err(TerminalSurfaceOwnerError::SessionId)
    );
    assert_eq!(
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), ""),
        Err(TerminalSurfaceOwnerError::SessionId)
    );
}

#[test]
fn test_terminal所有者_同じworktreeのタブと既存所有者を区別する() {
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let first = TerminalSurfaceOwner::workspace_terminal(workspace.clone(), "first").unwrap();
    let second = TerminalSurfaceOwner::workspace_terminal(workspace.clone(), "second").unwrap();
    // When / Then
    assert_ne!(first.stable_key(), second.stable_key());
    assert_ne!(
        first.stable_key(),
        TerminalSurfaceOwner::workspace(workspace.clone())
            .unwrap()
            .stable_key()
    );
    assert_ne!(
        first.stable_key(),
        TerminalSurfaceOwner::session(workspace.clone(), "first")
            .unwrap()
            .stable_key()
    );
    for id in [" ", "bad\0id"] {
        assert_eq!(
            TerminalSurfaceOwner::workspace_terminal(workspace.clone(), id),
            Err(TerminalSurfaceOwnerError::TerminalId)
        );
    }
}

#[test]
fn test_ターミナル画面所有者_nullを含むパスとidを拒否する() {
    // Given
    let workspace = WorkspaceIdentity::new("/repo\0/path");
    // When / Then
    assert_eq!(
        TerminalSurfaceOwner::workspace(workspace.clone()),
        Err(TerminalSurfaceOwnerError::WorkspacePath)
    );
    assert_eq!(
        TerminalSurfaceOwner::workspace_terminal(workspace.clone(), "terminal"),
        Err(TerminalSurfaceOwnerError::WorkspacePath)
    );
    assert_eq!(
        TerminalSurfaceOwner::session(workspace, "session"),
        Err(TerminalSurfaceOwnerError::WorkspacePath)
    );
    let workspace = WorkspaceIdentity::new("/repo");
    assert_eq!(
        TerminalSurfaceOwner::workspace_terminal(workspace.clone(), "terminal\0id"),
        Err(TerminalSurfaceOwnerError::TerminalId)
    );
    assert_eq!(
        TerminalSurfaceOwner::session(workspace, "session\0id"),
        Err(TerminalSurfaceOwnerError::SessionId)
    );
}
