use super::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;

#[test]
fn test_ターミナル画面所有者_ワークスペースパス正規化で同一識別子になる() {
    let first = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo/worktree/"));
    let second = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo//worktree"));

    assert_eq!(first, second);
    assert_eq!(first.workspace_identity().as_str(), "/repo/worktree");
    assert_eq!(first.stable_key(), second.stable_key());
}

#[test]
fn test_ターミナル画面所有者_所有者種別ごとに安定キーを隔離する() {
    let workspace = WorkspaceIdentity::new("/repo");
    let owners = [
        TerminalSurfaceOwner::workspace(workspace.clone()),
        TerminalSurfaceOwner::session(workspace, "shared-id"),
    ];

    assert_ne!(owners[0].stable_key(), owners[1].stable_key());
}

#[test]
fn test_ターミナル画面所有者_長さ接頭辞で構成要素衝突を防ぐ() {
    let workspace = WorkspaceIdentity::new("/repo");
    let first = TerminalSurfaceOwner::session(workspace.clone(), "a:b");
    let second = TerminalSurfaceOwner::session(workspace, "a");

    assert_ne!(first.stable_key(), second.stable_key());
}
