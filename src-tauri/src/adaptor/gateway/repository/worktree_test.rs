use super::*;

#[cfg(unix)]
#[test]
fn test_worktree識別パス_実体消失後も同じ表記を返す() {
    // Given
    let parent = tempfile::tempdir().unwrap();
    let alias = parent.path().join("alias");
    std::os::unix::fs::symlink(parent.path(), &alias).unwrap();
    let worktree = alias.join("worktree");
    std::fs::create_dir(&worktree).unwrap();
    let identity = path_to_worktree_identity(&worktree).unwrap();

    // When
    std::fs::remove_dir(&worktree).unwrap();

    // Then
    assert_eq!(path_to_worktree_identity(&worktree).unwrap(), identity);
    assert_eq!(
        identity,
        parent
            .path()
            .canonicalize()
            .unwrap()
            .join("worktree")
            .to_string_lossy()
    );
}

#[test]
fn test_worktree識別パス_解決エラーを返す() {
    // Given
    let path = Path::new("/invalid\0path");
    // When / Then
    assert!(path_to_worktree_identity(path).is_err());
}

#[test]
fn test_worktree列挙_途中の停止を欠損や成功に変えず返す() {
    use crate::common::operation_context::{Deadline, OperationContext, OperationStopped};
    use std::sync::Arc;
    use std::time::Instant;
    // Given
    let (directory, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    for name in ["first", "second", "third"] {
        repo.worktree(name, &directory.path().join(name), None)
            .unwrap();
    }
    let names = repo.worktrees().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    for (context, expected) in [
        (
            OperationContext::new(None, Arc::new(token)),
            OperationStopped::Cancelled,
        ),
        (
            OperationContext::default().with_deadline(Deadline::new(Instant::now())),
            OperationStopped::Expired,
        ),
    ] {
        let mut entries = each_worktree(&repo, &names);
        assert!(entries.next().unwrap().is_ok());
        let mut visited = 0;
        // When
        let result = crate::common::operation_context::sync_scope(context, || {
            entries.try_for_each(|entry| {
                entry?;
                visited += 1;
                Ok::<_, git_operation::GitOperationError>(())
            })
        });
        // Then
        assert!(
            matches!(result, Err(git_operation::GitOperationError::Stopped(error)) if error == expected)
        );
        assert_eq!(visited, 0);
        assert!(entries.next().unwrap().is_ok());
    }
}

#[test]
fn test_worktree列挙_通常の取得失敗は従来どおり読み飛ばす() {
    // Given
    let (directory, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    let path = directory.path().join("missing");
    repo.worktree("missing", &path, None).unwrap();
    let names = repo.worktrees().unwrap();
    std::fs::remove_dir_all(&path).unwrap();
    repo.find_worktree("missing").unwrap().prune(None).unwrap();
    // When / Then
    assert!(each_worktree(&repo, &names).next().is_none());
}

#[test]
fn test_worktree一覧と掃除_各git操作の停止を成功に変えず後続へ進まない() {
    use super::super::test_helpers::assert_stops_at_each_checkpoint;
    // Given
    let (directory, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    repo.worktree("linked", &directory.path().join("linked"), None)
        .unwrap();
    let path = directory.path().to_str().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| list_worktrees(path));
    assert_stops_at_each_checkpoint(|| {
        prune_invalid_worktrees(&repo).map_err(crate::domain::repository::RepositoryError::from)
    });
}

#[test]
fn test_worktree掃除_無効な登録のpruneでも停止を返す() {
    use super::super::test_helpers::assert_stops_at_each_checkpoint;
    // Given / When / Then
    assert_stops_at_each_checkpoint(|| {
        let (directory, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        let path = directory.path().join("missing");
        repo.worktree("missing", &path, None).unwrap();
        std::fs::remove_dir_all(path).unwrap();
        prune_invalid_worktrees(&repo).map_err(crate::domain::repository::RepositoryError::from)
    });
}

#[test]
fn test_worktree変更_既存branchの作成と削除は各操作で停止する() {
    use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
    // Given / When / Then
    for remove in [false, true] {
        assert_stops_at_each_checkpoint(|| {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("repo");
            let repo = Repository::init(&root).unwrap();
            crate::test_support::git::create_initial_commit(&repo);
            let wt = dir.path().join("worktree");
            if remove {
                repo.worktree("worktree", &wt, None).unwrap();
                remove_worktree(root.to_str().unwrap(), wt.to_str().unwrap(), true).map(|_| ())
            } else {
                repo.branch(
                    "feature",
                    &repo.head().unwrap().peel_to_commit().unwrap(),
                    false,
                )
                .unwrap();
                create_worktree(
                    root.to_str().unwrap(),
                    wt.to_str().unwrap(),
                    "feature",
                    false,
                    None,
                )
                .map(|_| ())
            }
        });
    }
}

#[test]
fn test_旧worktreeの所属repo_停止を欠損やgitファイルの復元へ変えない() {
    use super::super::test_helpers::assert_stops_at_each_checkpoint;
    // Given
    let (directory, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    let missing = directory.path().join("missing");
    std::fs::create_dir(&missing).unwrap();
    std::fs::write(missing.join(".git"), "gitdir: ../.git/worktrees/missing\n").unwrap();
    // When / Then
    for path in [directory.path(), missing.as_path()] {
        assert!(recorded_main_repo_path(path.to_str().unwrap())
            .unwrap()
            .is_some());
        assert_stops_at_each_checkpoint(|| {
            recorded_main_repo_path(path.to_str().unwrap())
                .map_err(crate::domain::repository::RepositoryError::from)
        });
    }
}

#[test]
fn test_worktree作成失敗_巻き戻しの停止を元のgitエラーへ変えず後続へ進まない() {
    use super::super::test_helpers::assert_stops_at_each_checkpoint;
    // Given / When / Then
    assert_stops_at_each_checkpoint(|| {
        let (directory, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        let path = directory.path().join("occupied");
        std::fs::create_dir(&path).unwrap();
        let error = create_worktree(
            directory.path().to_str().unwrap(),
            path.to_str().unwrap(),
            "rollback",
            true,
            None,
        )
        .unwrap_err();
        match error {
            RepositoryError::Technical(_) => Err(error),
            RepositoryError::External(_) => {
                assert!(repo.find_branch("rollback", BranchType::Local).is_err());
                Ok(())
            }
            other => panic!("unexpected error: {other:?}"),
        }
    });
}

#[test]
fn test_worktree一覧_コミットのないrepositoryも正常に返す() {
    for existing_base in [false, true] {
        let (directory, repo) = crate::test_support::git::create_test_repo();
        if existing_base {
            crate::test_support::git::create_initial_commit(&repo);
            let base = repo.head().unwrap().shorthand().unwrap().to_string();
            super::super::git_config::set_releash_base(
                directory.path().to_str().unwrap(),
                Some(&base),
            )
            .unwrap();
            repo.set_head("refs/heads/orphan").unwrap();
        }
        let entries = list_worktrees(directory.path().to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].branch, "(no commits)");
        assert!(entries[0].is_main);
        assert!(!entries[0].is_merged);
    }
}

#[test]
fn test_worktree一覧_登録を残して実体を消した行だけを除外する() {
    let (directory, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    let worktree = directory.path().join("linked");
    repo.worktree("linked", &worktree, None).unwrap();
    std::fs::remove_dir_all(&worktree).unwrap();
    assert!(repo.find_worktree("linked").is_ok());
    let rows = list_worktrees(directory.path().to_str().unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_main);
}

#[test]
fn test_worktree一覧_名前読取失敗を行の欠落に変換しない() {
    let (_directory, repo) = crate::test_support::git::create_test_repo();
    let config = repo.path().join("config");
    let mut bytes = std::fs::read(&config).unwrap();
    bytes.extend_from_slice(b"\n[remote \"\xff\"]\nurl = https://example.com/repo\n");
    std::fs::write(config, bytes).unwrap();
    let names = repo.remotes().unwrap();
    assert_eq!(names.len(), 1);
    assert!(names.get(0).is_err());
    let entries = each_worktree(&repo, &names).collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    assert!(
        matches!(&entries[0], Err(GitOperationError::Git(error)) if error.code() != git2::ErrorCode::NotFound)
    );
}

#[test]
fn test_worktree一覧_lockbranch読取失敗を正常な行に変換しない() {
    for failure in ["lock", "branch"] {
        let (directory, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        let worktree = directory.path().join("linked");
        let linked = repo.worktree("linked", &worktree, None).unwrap();
        let administration = repo.path().join("worktrees/linked");
        match failure {
            "lock" => std::fs::create_dir(administration.join("locked")).unwrap(),
            "branch" => std::fs::write(administration.join("HEAD"), "invalid head\n").unwrap(),
            _ => unreachable!(),
        }
        linked.validate().unwrap();
        assert!(
            list_worktrees(directory.path().to_str().unwrap()).is_err(),
            "{failure}"
        );
    }
}
