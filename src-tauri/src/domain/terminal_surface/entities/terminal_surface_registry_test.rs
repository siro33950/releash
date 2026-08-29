use super::*;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;

fn session(
    runtime_generation: u64,
    session_key: &str,
    worktree_path: Option<&str>,
    label: Option<&str>,
) -> TerminalSurface {
    let workspace = WorkspaceIdentity::new(worktree_path.unwrap_or("/"));
    let mut session = TerminalSurface::new_with_session_key(
        runtime_generation,
        session_key.to_string(),
        TerminalSurfaceOwner::session(workspace, session_key).unwrap(),
        label.map(str::to_string),
    );
    session.worktree_path = worktree_path.map(str::to_string);
    session
}

#[test]
fn test_ターミナル画面_実行世代_単調増加で採番する() {
    let mut registry = TerminalSurfaceRegistry::default();

    assert_eq!(registry.next_runtime_generation(), 1);
    assert_eq!(registry.next_runtime_generation(), 2);
    assert_eq!(registry.next_runtime_generation(), 3);
}

#[test]
fn test_ターミナル画面_登録簿_追加参照削除が一致する() {
    let mut registry = TerminalSurfaceRegistry::default();
    registry.insert(session(10, "key-1", Some("/repo"), Some("dev")));

    let found = registry.find_by_session_key("key-1").unwrap();
    assert_eq!(found.runtime_generation.value(), 10);
    assert_eq!(found.label.as_deref(), Some("dev"));

    assert!(registry.find_by_session_key("missing").is_none());
    assert_eq!(registry.remove(10).unwrap().session_key, "key-1");
    assert_eq!(registry.len(), 0);
}

#[test]
fn test_ターミナル画面登録簿_概要を可変セッションを公開せず列挙する() {
    let mut registry = TerminalSurfaceRegistry::default();
    registry.insert(session(1, "key-1", Some("/repo"), Some("dev")));
    registry.insert(session(2, "key-2", Some("/repo2"), None));

    let snapshots = registry.list_summaries();

    assert_eq!(snapshots.len(), 2);
    assert!(snapshots
        .iter()
        .any(|snapshot| snapshot.runtime_generation.value() == 1));
    assert!(snapshots
        .iter()
        .any(|snapshot| snapshot.runtime_generation.value() == 2));
}

#[test]
fn test_ターミナル画面終了対象_作業木が一致する画面だけを選ぶ() {
    let mut registry = TerminalSurfaceRegistry::default();
    registry.insert(session(1, "key-1", Some("/repo"), Some("dev")));
    registry.insert(session(2, "key-2", Some("/repo"), Some("test")));
    registry.insert(session(3, "key-3", Some("/other"), None));
    registry.insert(session(4, "key-4", None, None));

    let mut targets = registry.select_kill_targets_by_worktree("/repo");
    targets.sort_unstable();

    assert_eq!(targets, vec![1, 2]);
}

#[test]
fn test_ターミナル画面終了_登録簿内のプロセス状態を更新する() {
    let mut registry = TerminalSurfaceRegistry::default();
    registry.insert(session(1, "key-1", Some("/repo"), None));

    assert_eq!(registry.mark_exited(1, Some(42)), Some(1));
    let snapshot = registry.get(1).unwrap().clone();
    assert!(snapshot.process_state.is_exited());
    assert_eq!(snapshot.process_state.exit_code(), Some(42));
    assert!(registry.mark_exited(999, None).is_none());
}

#[test]
fn test_ターミナル画面生成予約_同一作業木で旧上限を超えて成功する() {
    let mut registry = TerminalSurfaceRegistry::default();

    for index in 0..33 {
        let session_key = format!("key-{index}");
        let reservation = registry.reserve_spawn_slot(&session_key).unwrap();
        registry.complete_spawn_slot(&reservation);
        registry.insert(session(index + 1, &session_key, Some("/repo"), None));
    }

    assert_eq!(registry.len(), 33);
}

#[test]
fn test_ターミナル画面生成予約_生存中または予約済み所有者を拒否する() {
    let mut registry = TerminalSurfaceRegistry::default();
    registry.insert(session(1, "live-key", Some("/repo"), None));

    assert_eq!(
        registry.reserve_spawn_slot("live-key"),
        Err(TerminalSurfaceSpawnReservationError::OwnerOccupied(
            "live-key".to_string()
        ))
    );

    let reservation = registry.reserve_spawn_slot("reserved-key").unwrap();
    assert_eq!(
        registry.reserve_spawn_slot("reserved-key"),
        Err(TerminalSurfaceSpawnReservationError::OwnerOccupied(
            "reserved-key".to_string()
        ))
    );

    registry.rollback_spawn_slot(&reservation);
    assert!(registry.reserve_spawn_slot("reserved-key").is_ok());
}
