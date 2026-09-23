use super::*;

type Lists = WorkspaceListRefresh<Vec<String>, String>;

fn populated() -> Lists {
    let mut lists = Lists::default();
    let generation = lists.begin();
    lists.complete_repositories(generation, Ok(vec!["/repo".into()]));
    lists.complete_branches(
        "/repo",
        generation,
        Ok((vec!["main".into()], vec!["/tree".into()])),
    );
    lists.complete_worktree("/tree", generation, Ok("session".into()));
    lists
}

#[test]
fn test_一覧更新_初回失敗と取得済み失敗を区別して復旧する() {
    // Given
    let mut lists = Lists::default();
    // When
    let generation = lists.begin();
    lists.complete_repositories(generation, Err("offline".into()));
    // Then
    assert!(!lists.repositories().loaded());
    assert_eq!(lists.repositories().error(), Some("offline"));
    // When
    let generation = lists.begin();
    lists.complete_repositories(generation, Ok(vec!["/repo".into()]));
    let generation = lists.begin();
    lists.complete_repositories(generation, Err("failed".into()));
    // Then
    assert!(lists.repositories().loaded());
    assert_eq!(lists.repositories().value().unwrap(), &["/repo"]);
    assert_eq!(lists.repositories().error(), Some("failed"));
    // When
    let generation = lists.begin();
    lists.complete_repositories(generation, Ok(vec![]));
    // Then
    assert!(lists.repositories().value().unwrap().is_empty());
    assert_eq!(lists.repositories().error(), None);
}

#[test]
fn test_一覧更新_古い成功と失敗を全階層で反映しない() {
    // Given
    let mut lists = populated();
    let old = lists.begin();
    let current = lists.begin();
    lists.complete_repositories(current, Ok(vec!["/repo".into()]));
    lists.complete_branches(
        "/repo",
        current,
        Ok((vec!["new".into()], vec!["/tree".into()])),
    );
    lists.complete_worktree("/tree", current, Ok("new".into()));
    // When / Then
    for result in [Ok(vec![]), Err("old".into())] {
        assert!(lists.complete_repositories(old, result).is_empty());
    }
    for result in [Ok((vec![], vec![])), Err("old".into())] {
        assert!(lists.complete_branches("/repo", old, result).is_empty());
    }
    for result in [Ok("old".into()), Err("old".into())] {
        assert!(!lists.complete_worktree("/tree", old, result));
    }
    assert_eq!(lists.repositories().value().unwrap(), &["/repo"]);
    assert_eq!(lists.branches("/repo").unwrap().value().unwrap().0, ["new"]);
    assert_eq!(lists.nodes("/tree").unwrap().value().unwrap(), "new");
    assert!(lists.nodes("/tree").unwrap().error().is_none());
}

#[test]
fn test_局所更新_全体の他階層を失効させず対象の古い結果だけを破棄する() {
    // Given
    let mut lists = populated();
    let full = lists.begin();
    let local = lists.begin_worktree("/tree").unwrap();
    // When
    lists.complete_worktree("/tree", local, Ok("local".into()));
    lists.complete_branches(
        "/repo",
        full,
        Ok((vec!["updated".into()], vec!["/tree".into()])),
    );
    // Then
    assert!(!lists.complete_worktree("/tree", full, Ok("old".into())));
    assert!(lists.is_current(full));
    assert_eq!(lists.nodes("/tree").unwrap().value().unwrap(), "local");
    assert_eq!(
        lists.branches("/repo").unwrap().value().unwrap().0,
        ["updated"]
    );
    // When
    let new = lists.begin();
    lists.complete_worktree("/tree", new, Ok("new".into()));
    // Then
    assert!(!lists.complete_worktree("/tree", local, Err("late".into())));
    assert_eq!(lists.nodes("/tree").unwrap().value().unwrap(), "new");
}

#[test]
fn test_削除確認_保持値を解放し遅れて返る子一覧を復活させない() {
    // Given
    let mut lists = populated();
    let local = lists.begin_worktree("/tree").unwrap();
    let full = lists.begin();
    // When
    lists.complete_branches("/repo", full, Ok((vec![], vec![])));
    // Then
    assert!(lists.nodes("/tree").is_none());
    assert!(!lists.complete_worktree("/tree", local, Ok("late".into())));
    assert!(lists.begin_worktree("/missing").is_none());
    // When
    lists.complete_repositories(full, Ok(vec![]));
    // Then
    assert!(lists.branches("/repo").is_none());
}

#[test]
fn test_子一覧失敗_前回値を保持し再取得成功でエラーを解消する() {
    // Given
    let mut lists = populated();
    let generation = lists.begin();
    // When
    lists.complete_branches("/repo", generation, Err("branches".into()));
    lists.complete_worktree("/tree", generation, Err("nodes".into()));
    // Then
    assert_eq!(
        lists.branches("/repo").unwrap().value().unwrap().0,
        ["main"]
    );
    assert_eq!(lists.nodes("/tree").unwrap().value().unwrap(), "session");
    assert_eq!(lists.nodes("/tree").unwrap().error(), Some("nodes"));
    // When
    let local = lists.begin_worktree("/tree").unwrap();
    lists.complete_worktree("/tree", local, Ok(String::new()));
    // Then
    assert_eq!(lists.nodes("/tree").unwrap().value().unwrap(), "");
    assert!(lists.nodes("/tree").unwrap().error().is_none());
    assert_eq!(lists.branches("/repo").unwrap().error(), Some("branches"));
}

#[test]
fn test_repository更新_他repositoryと登録一覧の進行を妨げず対象の古い結果を破棄する() {
    // Given
    let mut lists = populated();
    let full = lists.begin();
    let local = lists.begin_repository("/repo").unwrap();
    // When
    lists.complete_repositories(full, Ok(vec!["/repo".into(), "/other".into()]));
    lists.complete_branches(
        "/repo",
        local,
        Ok((vec!["local".into()], vec!["/tree".into()])),
    );
    lists.complete_worktree("/tree", local, Ok("local".into()));
    lists.complete_branches("/other", full, Ok((vec!["other".into()], vec![])));
    // Then
    for result in [Ok((vec![], vec![])), Err("late".into())] {
        assert!(lists.complete_branches("/repo", full, result).is_empty());
    }
    assert!(!lists.complete_worktree("/tree", full, Ok("late".into())));
    assert_eq!(
        lists.branches("/repo").unwrap().value().unwrap().0,
        ["local"]
    );
    assert_eq!(
        lists.branches("/other").unwrap().value().unwrap().0,
        ["other"]
    );
    assert_eq!(lists.nodes("/tree").unwrap().value().unwrap(), "local");
    assert!(lists.begin_repository("/missing").is_none());
    // When
    let next = lists.begin();
    lists.complete_repositories(next, Ok(vec!["/other".into()]));
    // Then
    assert!(lists
        .complete_branches("/repo", local, Ok((vec![], vec![])))
        .is_empty());
    assert!(lists.branches("/repo").is_none());
    assert!(lists.nodes("/tree").is_none());
}

#[test]
fn test_一覧状態_初回と空と失敗と復旧を区別する() {
    // Given
    let mut list = WorkspaceListEntry::<Vec<String>>::default();
    assert_eq!(list.state(true), WorkspaceListState::Loading);
    // When
    list.complete(0, Err("offline".into()));
    // Then
    assert_eq!(list.state(true), WorkspaceListState::InitialFailed);
    list.complete(0, Ok(vec![]));
    assert_eq!(list.state(true), WorkspaceListState::Empty);
    list.complete(0, Err("offline".into()));
    assert_eq!(list.state(true), WorkspaceListState::RefreshFailed);
    list.complete(0, Ok(vec!["repo".into()]));
    assert_eq!(list.state(false), WorkspaceListState::Ready);
}

#[test]
fn test_pr反映_古い世代と削除されたrepositoryを反映しない() {
    // Given
    let mut lists = populated();
    let generation = lists.begin_repository("/repo").unwrap();
    // When
    assert!(!lists.update_branches("/repo", generation - 1, |branches| branches.clear()));
    assert!(lists.update_branches("/repo", generation, |branches| branches.push("pr".into())));
    // Then
    assert_eq!(
        lists.branches("/repo").unwrap().value().unwrap().0,
        ["main", "pr"]
    );
    let next = lists.begin();
    lists.complete_repositories(next, Ok(vec![]));
    assert!(!lists.update_branches("/repo", generation, |branches| branches.clear()));
}

#[test]
fn test_全体更新要求_未開始を統合し実行中は次の一回を予約する() {
    // Given
    let mut lists = WorkspaceListRefresh::<(), ()>::default();
    assert_eq!(lists.start_full(), None);
    // When / Then
    assert_eq!(lists.request_full(), 1);
    assert_eq!(lists.request_full(), 1);
    let (first, _) = lists.start_full().unwrap();
    assert_eq!(first, 1);
    for _ in 0..3 {
        assert_eq!(lists.request_full(), 2);
        assert_eq!(lists.start_full(), None);
    }
    lists.complete_full(first);
    let (second, _) = lists.start_full().unwrap();
    assert_eq!(second, 2);
    assert_eq!(lists.request_full(), 3);
    lists.complete_full(second);
    let (third, _) = lists.start_full().unwrap();
    assert_eq!(third, 3);
    lists.complete_full(third);
    assert_eq!(lists.start_full(), None);
}
