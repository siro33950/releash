use super::*;

#[test]
fn test_worktree削除_受理済み変更の終了を待ち新しい変更を拒否する() {
    // Given
    let mut state = WorktreeOperationState::default();
    assert!(!state.is_deleting());
    state.begin_mutation().unwrap();
    // When
    state.begin_deletion().unwrap();
    assert!(state.is_deleting());
    // Then
    assert!(!state.ready_to_delete());
    assert!(state.begin_mutation().is_err());
    assert!(state.begin_deletion().is_err());
    state.finish_mutation();
    assert!(state.ready_to_delete());
    state.finish_deletion();
    assert!(!state.is_deleting());
    state.begin_mutation().unwrap();
}

#[test]
fn test_worktree削除_受理した対象は排他終了と同時に失われる() {
    // Given
    let mut state = WorktreeOperationState::default();
    state.begin_deletion().unwrap();
    assert!(state.deletion_target().is_none());
    // When
    state
        .accept_deletion(WorktreeDeletionTarget {
            repository_root: "/repo".into(),
            path: "/worktree".into(),
            branch: Some("feature".into()),
        })
        .unwrap();
    // Then
    assert_eq!(state.deletion_target().unwrap().path, "/worktree");
    assert!(state.begin_mutation().is_err());
    state.finish_deletion();
    assert!(state.deletion_target().is_none());
    assert!(state.begin_mutation().is_ok());
}

#[test]
fn test_worktree削除_先行変更が残る間は受理できない() {
    // Given
    let mut state = WorktreeOperationState::default();
    state.begin_mutation().unwrap();
    state.begin_deletion().unwrap();
    // When
    let result = state.accept_deletion(WorktreeDeletionTarget {
        repository_root: "/repo".into(),
        path: "/worktree".into(),
        branch: None,
    });
    // Then
    assert!(result.is_err());
    assert!(state.deletion_target().is_none());
    assert!(state.is_deleting());
}
