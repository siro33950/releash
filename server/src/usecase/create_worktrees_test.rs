use super::*;
use crate::usecase::test_helpers::{usecase, FakeRepo};

#[tokio::test]
async fn test_複数作成_無いブランチはbaseから作り既存のブランチはbaseを使わず再利用する() {
    // Given
    let fake = Arc::new(FakeRepo {
        branches: vec![crate::domain::git::Branch::local("existing")],
        ..Default::default()
    });
    // When
    let paths = create_worktrees(
        Arc::new(usecase(fake.clone())),
        "/r".into(),
        vec!["new".into(), "existing".into()],
        Some("main".into()),
    )
    .await
    .unwrap();
    // Then
    assert_eq!(paths, vec!["/r-worktrees/new", "/r-worktrees/existing"]);
    assert_eq!(
        *fake.worktree_creations.lock(),
        vec![
            (
                "/r-worktrees/new".into(),
                "new".into(),
                true,
                Some("main".into())
            ),
            (
                "/r-worktrees/existing".into(),
                "existing".into(),
                false,
                None
            )
        ]
    );
    assert_eq!(
        *fake.set_branch_base_override_calls.lock(),
        vec![("new".to_string(), Some("main".to_string()))]
    );
}

#[tokio::test]
async fn test_複数作成_作成の失敗を返して後続を作らない() {
    // Given
    let fake = Arc::new(FakeRepo {
        fail_create_worktree: true,
        ..Default::default()
    });
    // When
    let result = create_worktrees(
        Arc::new(usecase(fake.clone())),
        "/r".into(),
        vec!["a".into(), "b".into()],
        None,
    )
    .await;
    // Then
    assert!(result.is_err());
    assert!(!fake
        .worktree_creations
        .lock()
        .iter()
        .any(|entry| entry.1 == "b"));
}

#[tokio::test]
async fn test_複数作成_空と重複の入力では作成しない() {
    // Given / When / Then
    for branches in [vec![], vec!["".into()], vec!["a".into(), "a".into()]] {
        let fake = Arc::new(FakeRepo::default());
        let repo = Arc::new(usecase(fake.clone()));
        assert!(create_worktrees(repo, "/r".into(), branches, None)
            .await
            .is_err());
        assert!(fake.worktree_creations.lock().is_empty());
    }
}
