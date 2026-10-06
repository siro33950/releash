use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;

use git2::BranchType;
use git2::Repository;
use releash_lib::test_support::integration::platform::GitOperationError;
use releash_lib::test_support::integration::platform::OperationContext;
use releash_lib::test_support::integration::platform::OperationStopped;
use releash_lib::test_support::integration::repository::create_worktree;
use releash_lib::test_support::integration::repository::each_worktree;
use releash_lib::test_support::integration::repository::find_main_repo_path;
use releash_lib::test_support::integration::repository::list_worktrees;
use releash_lib::test_support::integration::repository::path_to_worktree_identity;
use releash_lib::test_support::integration::repository::prune_invalid_worktrees;
use releash_lib::test_support::integration::repository::recorded_main_repo_path;
use releash_lib::test_support::integration::repository::remove_worktree;
use releash_lib::test_support::integration::repository::RepositoryError;
use std::path::Path;

#[cfg(unix)]
#[test]
pub fn test_worktree識別パス_実体消失後も同じ表記を返す() {
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
pub fn test_worktree識別パス_解決エラーを返す() {
    // Given
    let path = Path::new("/invalid\0path");
    // When / Then
    assert!(path_to_worktree_identity(path).is_err());
}

#[test]
pub fn test_worktree列挙_途中の取り消しを欠損や成功に変えず返す() {
    use releash_lib::test_support::integration::platform::OperationContext;
    use releash_lib::test_support::integration::platform::OperationStopped;
    use std::sync::Arc;
    // Given
    let (directory, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    for name in ["first", "second", "third"] {
        repo.worktree(name, &directory.path().join(name), None)
            .unwrap();
    }
    let names = repo.worktrees().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let context = OperationContext::new(None, Arc::new(token));
    let mut entries = each_worktree(&repo, &names);
    assert!(entries.next().unwrap().is_ok());
    let mut visited = 0;
    // When
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        entries.try_for_each(|entry| {
            entry?;
            visited += 1;
            Ok::<_, releash_lib::test_support::integration::platform::GitOperationError>(())
        })
    });
    // Then
    assert!(
        matches!(result, Err(releash_lib::test_support::integration::platform::GitOperationError::Stopped(error)) if error == OperationStopped::Cancelled)
    );
    assert_eq!(visited, 0);
    assert!(entries.next().unwrap().is_ok());
}

#[test]
pub fn test_worktree列挙_途中の期限切れを欠損や成功に変えず返す() {
    use releash_lib::test_support::integration::platform::Deadline;

    use std::time::Instant;
    // Given
    let (directory, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    for name in ["first", "second", "third"] {
        repo.worktree(name, &directory.path().join(name), None)
            .unwrap();
    }
    let names = repo.worktrees().unwrap();

    let context = OperationContext::default().with_deadline(Deadline::new(Instant::now()));
    let mut entries = each_worktree(&repo, &names);
    assert!(entries.next().unwrap().is_ok());
    let mut visited = 0;
    // When
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        entries.try_for_each(|entry| {
            entry?;
            visited += 1;
            Ok::<_, releash_lib::test_support::integration::platform::GitOperationError>(())
        })
    });
    // Then
    assert!(
        matches!(result, Err(releash_lib::test_support::integration::platform::GitOperationError::Stopped(error)) if error == OperationStopped::Expired)
    );
    assert_eq!(visited, 0);
    assert!(entries.next().unwrap().is_ok());
}

#[test]
pub fn test_worktree列挙_通常の取得失敗は従来どおり読み飛ばす() {
    // Given
    let (directory, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    let path = directory.path().join("missing");
    repo.worktree("missing", &path, None).unwrap();
    let names = repo.worktrees().unwrap();
    std::fs::remove_dir_all(&path).unwrap();
    repo.find_worktree("missing").unwrap().prune(None).unwrap();
    // When
    let result = each_worktree(&repo, &names).next();
    // Then
    assert!(result.is_none());
}

#[test]
pub fn test_worktree一覧と掃除_各git操作の停止を成功に変えず後続へ進まない() {
    use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;
    // Given
    let (directory, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    repo.worktree("linked", &directory.path().join("linked"), None)
        .unwrap();
    let path = directory.path().to_str().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| list_worktrees(path));
    assert_stops_at_each_checkpoint(|| prune_invalid_worktrees(&repo));
}

#[test]
pub fn test_worktree掃除_無効な登録のpruneでも停止を返す() {
    // Given / When / Then
    assert_stops_at_each_checkpoint(|| {
        let (directory, repo) = crate::test_support_git::create_test_repo();
        crate::test_support_git::create_initial_commit(&repo);
        let path = directory.path().join("missing");
        repo.worktree("missing", &path, None).unwrap();
        std::fs::remove_dir_all(path).unwrap();
        prune_invalid_worktrees(&repo)
    });
}

#[test]
pub fn test_worktree変更_既存branchの作成と削除は各操作で停止する() {
    // Given / When / Then
    for remove in [false, true] {
        assert_stops_at_each_checkpoint(|| {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("repo");
            let repo = Repository::init(&root).unwrap();
            crate::test_support_git::create_initial_commit(&repo);
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
pub fn test_旧worktreeの所属repo_停止を欠損やgitファイルの復元へ変えない() {
    // Given
    let (directory, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    let missing = directory.path().join("missing");
    std::fs::create_dir(&missing).unwrap();
    std::fs::write(missing.join(".git"), "gitdir: ../.git/worktrees/missing\n").unwrap();
    // When / Then
    for path in [directory.path(), missing.as_path()] {
        assert!(recorded_main_repo_path(path.to_str().unwrap())
            .unwrap()
            .is_some());
        assert_stops_at_each_checkpoint(|| recorded_main_repo_path(path.to_str().unwrap()));
    }
}

#[test]
pub fn test_worktree作成失敗_巻き戻しの停止を元のgitエラーへ変えず後続へ進まない() {
    // Given / When / Then
    assert_stops_at_each_checkpoint(|| {
        let (directory, repo) = crate::test_support_git::create_test_repo();
        crate::test_support_git::create_initial_commit(&repo);
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
pub fn test_worktree一覧_コミットのないrepositoryも正常に返す() {
    // Given
    let (directory, _repo) = crate::test_support_git::create_test_repo();

    // When
    let entries = list_worktrees(directory.path().to_str().unwrap()).unwrap();
    // Then
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].branch, "(no commits)");
    assert!(entries[0].is_main);
    assert!(!entries[0].is_merged);
}

#[test]
pub fn test_worktree一覧_baseがあってもコミットのないbranchを正常に返す() {
    // Given
    let (directory, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    let base = repo.head().unwrap().shorthand().unwrap().to_string();
    releash_lib::test_support::integration::repository::set_releash_base(
        directory.path().to_str().unwrap(),
        Some(&base),
    )
    .unwrap();
    repo.set_head("refs/heads/orphan").unwrap();
    // When
    let entries = list_worktrees(directory.path().to_str().unwrap()).unwrap();
    // Then
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].branch, "(no commits)");
    assert!(entries[0].is_main);
    assert!(!entries[0].is_merged);
}

#[test]
pub fn test_worktree一覧_登録を残して実体を消した行だけを除外する() {
    // Given
    let (directory, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    let worktree = directory.path().join("linked");
    repo.worktree("linked", &worktree, None).unwrap();
    // When
    std::fs::remove_dir_all(&worktree).unwrap();
    // Then
    assert!(repo.find_worktree("linked").is_ok());
    let rows = list_worktrees(directory.path().to_str().unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_main);
}

#[test]
pub fn test_worktree一覧_名前読取失敗を行の欠落に変換しない() {
    // Given
    let (_directory, repo) = crate::test_support_git::create_test_repo();
    let config = repo.path().join("config");
    let mut bytes = std::fs::read(&config).unwrap();
    bytes.extend_from_slice(b"\n[remote \"\xff\"]\nurl = https://example.com/repo\n");
    std::fs::write(config, bytes).unwrap();
    // When
    let names = repo.remotes().unwrap();
    // Then
    assert_eq!(names.len(), 1);
    assert!(names.get(0).is_err());
    let entries = each_worktree(&repo, &names).collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    assert!(
        matches!(&entries[0], Err(GitOperationError::Git(error)) if error.code() != git2::ErrorCode::NotFound)
    );
}

#[test]
pub fn test_worktree一覧_lock読取失敗を正常な行に変換しない() {
    // Given
    let (directory, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    let worktree = directory.path().join("linked");
    let linked = repo.worktree("linked", &worktree, None).unwrap();
    let administration = repo.path().join("worktrees/linked");
    std::fs::create_dir(administration.join("locked")).unwrap();
    // When
    linked.validate().unwrap();
    // Then
    assert!(
        list_worktrees(directory.path().to_str().unwrap()).is_err(),
        "lock"
    );
}

#[test]
pub fn test_worktree一覧_branch読取失敗を正常な行に変換しない() {
    // Given
    let (directory, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    let worktree = directory.path().join("linked");
    let linked = repo.worktree("linked", &worktree, None).unwrap();
    let administration = repo.path().join("worktrees/linked");
    std::fs::write(administration.join("HEAD"), "invalid head\n").unwrap();
    // When
    linked.validate().unwrap();
    // Then
    assert!(
        list_worktrees(directory.path().to_str().unwrap()).is_err(),
        "branch"
    );
}

#[test]
pub fn test_git任意読取_find_main_repo_path_リポジトリでないディレクトリは不在を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        git2::Repository::discover(directory.path())
            .err()
            .unwrap()
            .code(),
        git2::ErrorCode::NotFound
    );
    let path = directory.path().to_path_buf();
    // When
    let result = find_main_repo_path(path.to_str().unwrap());
    // Then
    assert_eq!(result.unwrap(), None);
}

#[test]
pub fn test_git任意読取_find_main_repo_path_存在しないパスは不在を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        git2::Repository::discover(directory.path())
            .err()
            .unwrap()
            .code(),
        git2::ErrorCode::NotFound
    );
    let path = directory.path().join("missing");
    // When
    let result = find_main_repo_path(path.to_str().unwrap());
    // Then
    assert_eq!(result.unwrap(), None);
}
pub(crate) mod worktree_gateway_tests {

    use crate::test_support_git::*;
    use git2::BranchType;
    use git2::Repository;
    use git2::WorktreeAddOptions;
    use git2::WorktreePruneOptions;
    use releash_lib::test_support::integration::platform::detect_default_branch;
    use releash_lib::test_support::integration::repository::create_worktree;
    use releash_lib::test_support::integration::repository::find_main_repo_path;
    use releash_lib::test_support::integration::repository::get_current_branch;
    use releash_lib::test_support::integration::repository::get_main_repo_path;
    use releash_lib::test_support::integration::repository::get_worktree_dirty_count;
    use releash_lib::test_support::integration::repository::list_worktrees;
    use releash_lib::test_support::integration::repository::recorded_main_repo_path;
    use releash_lib::test_support::integration::repository::registered_worktree_paths;
    use releash_lib::test_support::integration::repository::remove_worktree;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;

    fn create_test_repo_with_parent() -> (tempfile::TempDir, PathBuf, Repository) {
        let parent = tempfile::TempDir::new().unwrap();
        let repo_dir = parent.path().join("main-repo");
        fs::create_dir(&repo_dir).unwrap();
        let repo = Repository::init(&repo_dir).unwrap();

        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        (parent, repo_dir, repo)
    }

    fn create_worktree_helper(
        repo: &Repository,
        parent_dir: &Path,
        wt_name: &str,
        branch_name: &str,
    ) -> PathBuf {
        let wt_path = parent_dir.join(wt_name);
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let branch = repo.branch(branch_name, &head, false).unwrap();
        let reference = branch.into_reference();
        let mut opts = WorktreeAddOptions::new();
        opts.reference(Some(&reference));
        repo.worktree(wt_name, &wt_path, Some(&opts)).unwrap();
        wt_path
    }

    #[test]
    pub fn test_メインリポジトリパス取得_メインから() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let result = get_main_repo_path(dir.path().to_str().unwrap()).unwrap();
        let expected = dir.path().canonicalize().unwrap();
        let result_canon = PathBuf::from(&result).canonicalize().unwrap();
        assert_eq!(result_canon, expected);
        assert_eq!(
            find_main_repo_path(dir.path().to_str().unwrap()).unwrap(),
            Some(result)
        );
    }

    #[test]
    pub fn test_メインリポジトリパス取得_worktreeから() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-test", "feat-test");

        let result = get_main_repo_path(wt_path.to_str().unwrap()).unwrap();
        let expected = repo_dir.canonicalize().unwrap();
        let result_canon = PathBuf::from(&result).canonicalize().unwrap();
        assert_eq!(result_canon, expected);
        assert_eq!(
            find_main_repo_path(wt_path.to_str().unwrap()).unwrap(),
            Some(result)
        );
    }

    #[test]
    pub fn test_旧worktreeの所属repo_git管理情報がなければ親repoを所属と推定しない() {
        // Given
        let (directory, _) = create_test_repo();
        let path = directory.path().join("removed-linked-worktree");
        std::fs::create_dir(&path).unwrap();
        // When / Then
        assert_eq!(
            recorded_main_repo_path(path.to_str().unwrap()).unwrap(),
            None
        );
        std::fs::remove_dir(&path).unwrap();
        assert_eq!(
            recorded_main_repo_path(path.to_str().unwrap()).unwrap(),
            None
        );
    }

    #[test]
    pub fn test_旧worktreeの所属repo_相対gitdirでも登録消失後の正規パスを返す() {
        // Given
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        let path = create_worktree_helper(&repo, parent.path(), "linked", "feature");
        std::fs::write(
            path.join(".git"),
            format!(
                "gitdir: ../{}/.git/worktrees/linked\n",
                repo_dir.file_name().unwrap().to_str().unwrap()
            ),
        )
        .unwrap();
        let mut options = WorktreePruneOptions::new();
        options.valid(true).working_tree(false);
        repo.find_worktree("linked")
            .unwrap()
            .prune(Some(&mut options))
            .unwrap();
        // When / Then
        assert_eq!(
            recorded_main_repo_path(path.to_str().unwrap()).unwrap(),
            Some(
                repo_dir
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        );
    }

    #[test]
    pub fn test_メインリポジトリパス取得_不正パス() {
        let result = get_main_repo_path("/nonexistent/invalid/path");
        assert!(result.is_err());
        assert_eq!(
            find_main_repo_path("/nonexistent/invalid/path").unwrap(),
            None
        );
        assert!(find_main_repo_path("\0").is_err());
    }

    #[test]
    pub fn test_dirty_count取得_クリーン() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let count = get_worktree_dirty_count(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    pub fn test_dirty_count取得_変更あり() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();

        let count = get_worktree_dirty_count(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    pub fn test_gc用一覧_フォルダ消失後もgit登録を保持し登録削除後は除外する() {
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        let path = create_worktree_helper(&repo, parent.path(), "gc", "gc")
            .canonicalize()
            .unwrap();
        fs::remove_dir_all(&path).unwrap();
        let listed = registered_worktree_paths(repo_dir.to_str().unwrap()).unwrap();
        assert!(listed
            .iter()
            .any(|(_, found)| found == path.to_str().unwrap()));
        repo.find_worktree("gc").unwrap().prune(None).unwrap();
        fs::create_dir(&path).unwrap();
        let listed = registered_worktree_paths(repo_dir.to_str().unwrap()).unwrap();
        assert!(!listed
            .iter()
            .any(|(_, found)| found == path.to_str().unwrap()));
        assert!(
            registered_worktree_paths(parent.path().join("missing").to_str().unwrap()).is_err()
        );
    }

    #[test]
    pub fn test_worktree一覧_メインのみ() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let entries = list_worktrees(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_main);
        assert!(!entries[0].is_locked);
    }

    #[test]
    pub fn test_worktree一覧_リンク済み() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        create_worktree_helper(&repo, _parent.path(), "wt-linked", "feat-linked");

        let entries = list_worktrees(repo_dir.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 2);

        let main_entry = entries.iter().find(|e| e.is_main).unwrap();
        assert!(main_entry.is_main);

        let linked_entry = entries.iter().find(|e| !e.is_main).unwrap();
        assert_eq!(linked_entry.name, "wt-linked");
        assert_eq!(linked_entry.branch, "feat-linked");
    }

    #[test]
    pub fn test_worktree一覧_ロック済み() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        create_worktree_helper(&repo, _parent.path(), "wt-lock", "feat-lock");
        let wt = repo.find_worktree("wt-lock").unwrap();
        wt.lock(None).unwrap();

        let entries = list_worktrees(repo_dir.to_str().unwrap()).unwrap();
        let locked_entry = entries.iter().find(|e| e.name == "wt-lock").unwrap();
        assert!(locked_entry.is_locked);
    }

    fn checkout(repo: &Repository, reference: &str) {
        repo.set_head(reference).unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
    }

    fn add_worktree_for_branch(repo: &Repository, parent_dir: &Path, branch_name: &str) {
        let branch = repo.find_branch(branch_name, BranchType::Local).unwrap();
        let reference = branch.into_reference();
        let mut opts = WorktreeAddOptions::new();
        opts.reference(Some(&reference));
        let name = format!("wt-{branch_name}");
        repo.worktree(&name, &parent_dir.join(&name), Some(&opts))
            .unwrap();
    }

    fn merge_into_head(repo: &Repository, merged: &git2::Commit<'_>, message: &str) {
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head, merged])
            .unwrap();
    }

    fn merged_of(repo_dir: &Path, branch: &str) -> bool {
        list_worktrees(repo_dir.to_str().unwrap())
            .unwrap()
            .into_iter()
            .find(|entry| entry.branch == branch)
            .unwrap()
            .is_merged
    }

    #[test]
    pub fn test_worktree一覧_baseが進んだだけのブランチは未マージ() {
        // Given: 固有の commit を持たないブランチの後に base が進む
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        create_worktree_helper(&repo, parent.path(), "wt-behind", "feature-behind");
        add_and_commit(&repo, "after.txt", "after", "commit after branch");

        // When / Then
        assert!(!merged_of(&repo_dir, "feature-behind"));
    }

    #[test]
    pub fn test_worktree一覧_baseと同じ先頭のブランチは未マージ() {
        // Given
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        create_worktree_helper(&repo, parent.path(), "wt-same", "feature-same");

        // When / Then
        assert!(!merged_of(&repo_dir, "feature-same"));
        assert!(!list_worktrees(repo_dir.to_str().unwrap()).unwrap()[0].is_merged);
    }

    #[test]
    pub fn test_worktree一覧_固有のcommitを持つブランチは未マージ() {
        // Given
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        let default_branch = detect_default_branch(&repo).unwrap().unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-unmerged", &head, false).unwrap();
        checkout(&repo, "refs/heads/feature-unmerged");
        add_and_commit(&repo, "feat.txt", "feat", "feature commit");
        checkout(&repo, &format!("refs/heads/{default_branch}"));
        add_worktree_for_branch(&repo, parent.path(), "feature-unmerged");

        // When / Then
        assert!(!merged_of(&repo_dir, "feature-unmerged"));
    }

    #[test]
    pub fn test_worktree一覧_merge_commit経由で取り込まれたブランチはマージ済み() {
        // Given
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        let default_branch = detect_default_branch(&repo).unwrap().unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-merged", &head, false).unwrap();
        checkout(&repo, "refs/heads/feature-merged");
        add_and_commit(&repo, "feat.txt", "feat", "feature commit");
        let feature_commit = repo.head().unwrap().peel_to_commit().unwrap();
        checkout(&repo, &format!("refs/heads/{default_branch}"));
        add_and_commit(&repo, "main.txt", "main", "main commit");
        merge_into_head(&repo, &feature_commit, "Merge feature-merged");
        add_worktree_for_branch(&repo, parent.path(), "feature-merged");

        // When / Then
        assert!(merged_of(&repo_dir, "feature-merged"));
    }

    #[test]
    pub fn test_worktree一覧_releash_baseが指すブランチへの取り込みでマージ済みを判定する() {
        // Given: develop へ merge 済みで、既定ブランチへは未 merge のブランチ
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        let default_branch = detect_default_branch(&repo).unwrap().unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("develop", &head, false).unwrap();
        checkout(&repo, "refs/heads/develop");
        add_and_commit(&repo, "dev.txt", "dev", "develop commit");
        let develop_head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-x", &develop_head, false).unwrap();
        checkout(&repo, "refs/heads/feature-x");
        add_and_commit(&repo, "feat.txt", "feat", "feature commit");
        let feature_commit = repo.head().unwrap().peel_to_commit().unwrap();
        checkout(&repo, "refs/heads/develop");
        add_and_commit(&repo, "dev2.txt", "dev2", "develop commit 2");
        merge_into_head(&repo, &feature_commit, "Merge feature-x into develop");
        checkout(&repo, &format!("refs/heads/{default_branch}"));
        add_worktree_for_branch(&repo, parent.path(), "feature-x");

        // When / Then
        assert!(!merged_of(&repo_dir, "feature-x"));
        releash_lib::test_support::integration::repository::set_releash_base(
            repo_dir.to_str().unwrap(),
            Some("develop"),
        )
        .unwrap();
        assert!(merged_of(&repo_dir, "feature-x"));
    }

    #[test]
    pub fn test_worktree一覧_detached_headのworktreeはブランチ名を括弧で表し未マージ() {
        // Given
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        let wt_path = create_worktree_helper(&repo, parent.path(), "wt-rebase", "feat-rebase");
        let wt_repo = Repository::open(&wt_path).unwrap();
        let head_commit = wt_repo.head().unwrap().peel_to_commit().unwrap();
        wt_repo.set_head_detached(head_commit.id()).unwrap();

        // When
        let entries = list_worktrees(repo_dir.to_str().unwrap()).unwrap();

        // Then
        let detached = entries
            .iter()
            .find(|entry| entry.name == "wt-rebase")
            .unwrap();
        assert!(detached.branch.starts_with('('), "{}", detached.branch);
        assert!(!detached.is_merged);
    }

    #[test]
    pub fn test_worktree作成_新規ブランチ() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = _parent.path().join("wt-new");
        let entry = create_worktree(
            repo_dir.to_str().unwrap(),
            wt_path.to_str().unwrap(),
            "feat-new",
            true,
            None,
        )
        .unwrap();

        assert_eq!(entry.branch, "feat-new");
        assert!(!entry.is_main);
        assert!(wt_path.exists());

        assert!(repo.find_branch("feat-new", BranchType::Local).is_ok());
    }

    #[test]
    pub fn test_worktree作成_既存ブランチ() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("existing-branch", &head, false).unwrap();

        let wt_path = _parent.path().join("wt-existing");
        let entry = create_worktree(
            repo_dir.to_str().unwrap(),
            wt_path.to_str().unwrap(),
            "existing-branch",
            false,
            None,
        )
        .unwrap();

        assert_eq!(entry.branch, "existing-branch");
        assert!(wt_path.exists());
    }

    // releash-base の設定は usecase オーケストレーションへ引き上げたため、gateway
    // 単体では検証しない（usecase 側 test_worktree作成をdtoへ合成する で担保）。

    #[test]
    pub fn test_worktree作成_base引数でも作成成功() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let main_branch = get_current_branch(repo_dir.to_str().unwrap()).unwrap();

        let wt_path = _parent.path().join("wt-withbase");
        let entry = create_worktree(
            repo_dir.to_str().unwrap(),
            wt_path.to_str().unwrap(),
            "feat-withbase",
            true,
            Some(&main_branch),
        )
        .unwrap();

        // gateway は worktree 作成のみ。base 指定でも worktree が作られる。
        assert_eq!(entry.branch, "feat-withbase");
        assert!(wt_path.exists());
    }

    #[test]
    pub fn test_worktree作成_重複ブランチ() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = _parent.path().join("wt-dup1");
        create_worktree(
            repo_dir.to_str().unwrap(),
            wt_path.to_str().unwrap(),
            "feat-dup",
            true,
            None,
        )
        .unwrap();

        let wt_path2 = _parent.path().join("wt-dup2");
        let result = create_worktree(
            repo_dir.to_str().unwrap(),
            wt_path2.to_str().unwrap(),
            "feat-dup",
            true,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    pub fn test_worktree作成_親ディレクトリ未存在() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = _parent.path().join("nested").join("deep").join("wt-new");
        assert!(!_parent.path().join("nested").exists());

        let entry = create_worktree(
            repo_dir.to_str().unwrap(),
            wt_path.to_str().unwrap(),
            "feat-nested",
            true,
            None,
        )
        .unwrap();

        assert_eq!(entry.branch, "feat-nested");
        assert!(wt_path.exists());
    }

    #[test]
    pub fn test_worktree作成_失敗時ブランチロールバック() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path1 = _parent.path().join("wt-occupy");
        create_worktree(
            repo_dir.to_str().unwrap(),
            wt_path1.to_str().unwrap(),
            "feat-occupy",
            true,
            None,
        )
        .unwrap();

        let wt_path2 = _parent.path().join("other").join("wt-occupy");
        let result = create_worktree(
            repo_dir.to_str().unwrap(),
            wt_path2.to_str().unwrap(),
            "feat-rollback",
            true,
            None,
        );
        assert!(result.is_err());

        assert!(repo
            .find_branch("feat-rollback", BranchType::Local)
            .is_err());
    }

    #[test]
    pub fn test_worktree削除_クリーン() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-rm", "feat-rm");
        assert!(wt_path.exists());

        remove_worktree(repo_dir.to_str().unwrap(), wt_path.to_str().unwrap(), false).unwrap();

        assert!(!wt_path.exists());
    }

    #[test]
    pub fn test_worktree削除_dirty_forceなし() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-dirty", "feat-dirty");
        fs::write(wt_path.join("dirty.txt"), "uncommitted").unwrap();

        let result = remove_worktree(repo_dir.to_str().unwrap(), wt_path.to_str().unwrap(), false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("uncommitted change"));
        assert!(wt_path.exists());
    }

    #[test]
    pub fn test_worktree削除_dirty_force() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-dirtyf", "feat-dirtyf");
        fs::write(wt_path.join("dirty.txt"), "uncommitted").unwrap();

        remove_worktree(repo_dir.to_str().unwrap(), wt_path.to_str().unwrap(), true).unwrap();

        assert!(!wt_path.exists());
    }

    #[test]
    pub fn test_worktree削除_ロック_forceなし() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-locked", "feat-locked");
        let wt = repo.find_worktree("wt-locked").unwrap();
        wt.lock(None).unwrap();

        let result = remove_worktree(repo_dir.to_str().unwrap(), wt_path.to_str().unwrap(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("locked"));
    }

    #[test]
    pub fn test_worktree削除_未発見() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let result = remove_worktree(
            dir.path().to_str().unwrap(),
            dir.path().join("nonexistent").to_str().unwrap(),
            false,
        );
        assert!(result.is_err());
    }
}
