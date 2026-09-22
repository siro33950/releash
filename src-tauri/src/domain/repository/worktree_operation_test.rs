use super::*;

#[test]
fn test_worktree削除_受理済み変更の終了を待ち新しい変更を拒否する() {
    // Given
    let mut state = WorktreeOperationState::default();
    state.begin_mutation().unwrap();
    // When
    state.begin_deletion().unwrap();
    // Then
    assert!(!state.ready_to_delete());
    assert!(state.begin_mutation().is_err());
    assert!(state.begin_deletion().is_err());
    state.finish_mutation();
    assert!(state.ready_to_delete());
    state.finish_deletion();
    state.begin_mutation().unwrap();
}
