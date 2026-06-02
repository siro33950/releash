//! worktree 責務の gateway 実装。git2 によるワークツリー操作を封じ込める。

use crate::domain::repository::{RepositoryError, Worktree, WorktreeRepository};
use crate::infrastructure::git::client;
use crate::infrastructure::git::helpers::get_branch_name_for_repo;
use git2::{BranchType, Repository, StatusOptions, WorktreeAddOptions, WorktreePruneOptions};
use std::path::{Path, PathBuf};

pub(crate) fn get_main_repo_path(any_path: &str) -> Result<String, RepositoryError> {
    let repo = client::discover(any_path)?;

    if repo.is_worktree() {
        let git_dir = repo.path();
        let commondir_file = git_dir.join("commondir");
        if commondir_file.exists() {
            let content = std::fs::read_to_string(&commondir_file)?;
            let commondir = git_dir.join(content.trim());
            let commondir = commondir.canonicalize()?;
            let main_workdir = commondir
                .parent()
                .ok_or_else(|| RepositoryError::rule("cannot determine main repo path"))?;
            return Ok(main_workdir
                .to_str()
                .ok_or_else(|| RepositoryError::rule("invalid path encoding"))?
                .to_string());
        }
    }

    let workdir = repo
        .workdir()
        .ok_or_else(|| RepositoryError::rule("bare repository"))?;
    Ok(workdir
        .to_str()
        .ok_or_else(|| RepositoryError::rule("invalid path encoding"))?
        .trim_end_matches('/')
        .to_string())
}

/// worktree の dirty 件数を算出する共通ロジック。
/// 未追跡ディレクトリ配下も再帰的に個別計上し、ignored は除外する。
/// 算出条件（`StatusOptions`）を 1 箇所に集約し、経路ごとの差異を排除する。
fn count_dirty_entries(repo: &Repository) -> Result<u32, RepositoryError> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = repo.statuses(Some(&mut opts))?;
    Ok(statuses
        .iter()
        .filter(|entry| !entry.status().contains(git2::Status::IGNORED))
        .count() as u32)
}

pub(crate) fn get_worktree_dirty_count(worktree_path: &str) -> Result<u32, RepositoryError> {
    let repo = client::open(worktree_path)?;
    count_dirty_entries(&repo)
}

/// パスから worktree を開いて dirty 件数を返す。開けない・取得失敗時は 0。
/// 一覧・カード集計で使う寛容版。Query 側（`branch_card`）からも参照する。
pub(super) fn get_dirty_count_for_path(path: &Path) -> u32 {
    Repository::open(path)
        .ok()
        .and_then(|repo| count_dirty_entries(&repo).ok())
        .unwrap_or(0)
}

/// repo のリンク済み worktree を `(name, Worktree)` で列挙する。
/// `worktrees()` の index 走査と `find_worktree` の定型を集約する。
/// `validate()` / `prune` / パス比較は用途ごとに呼び出し側で行う。
/// Query 側（`branch_card`）からも参照する。
pub(super) fn each_worktree<'a>(
    repo: &'a Repository,
    names: &'a git2::string_array::StringArray,
) -> impl Iterator<Item = (String, git2::Worktree)> + 'a {
    (0..names.len()).filter_map(move |i| {
        let name = match names.get(i) {
            Ok(Some(n)) => n.to_string(),
            _ => return None,
        };
        let wt = repo.find_worktree(&name).ok()?;
        Some((name, wt))
    })
}

fn resolve_main_repo_path(repo: &Repository) -> Result<PathBuf, RepositoryError> {
    repo.workdir()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| RepositoryError::rule("bare repository"))
}

/// 壊れた（`validate()` 失敗）linked worktree を working tree ごと prune する。
/// 個別エントリの prune 失敗は無視する（best-effort）。`create_worktree` の
/// 事前掃除と `prune_invalid` プリミティブの両方から使う。
fn prune_invalid_worktrees(repo: &Repository) {
    if let Ok(wt_names) = repo.worktrees() {
        for (_, wt) in each_worktree(repo, &wt_names) {
            if wt.validate().is_err() {
                let mut prune_opts = WorktreePruneOptions::new();
                prune_opts.working_tree(true);
                let _ = wt.prune(Some(&mut prune_opts));
            }
        }
    }
}

pub(crate) fn prune_invalid(repo_path: &str) -> Result<(), RepositoryError> {
    let repo = client::open(repo_path)?;
    prune_invalid_worktrees(&repo);
    Ok(())
}

pub(crate) fn list_worktrees(repo_path: &str) -> Result<Vec<Worktree>, RepositoryError> {
    let repo = client::open(repo_path)?;
    let main_workdir = resolve_main_repo_path(&repo)?;
    let mut entries = Vec::new();

    let main_branch = get_branch_name_for_repo(&repo);
    let main_name = main_workdir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("main")
        .to_string();

    entries.push(Worktree {
        name: main_name,
        path: main_workdir
            .to_str()
            .ok_or_else(|| RepositoryError::rule("invalid path encoding"))?
            .trim_end_matches('/')
            .to_string(),
        branch: main_branch,
        is_main: true,
        is_locked: false,
    });

    let wt_names = repo.worktrees()?;
    for (wt_name, wt) in each_worktree(&repo, &wt_names) {
        if wt.validate().is_err() {
            continue;
        }

        let wt_path = wt.path();
        let is_locked =
            matches!(wt.is_locked(), Ok(s) if !matches!(s, git2::WorktreeLockStatus::Unlocked));

        let branch = match Repository::open(wt_path) {
            Ok(wt_repo) => get_branch_name_for_repo(&wt_repo),
            Err(_) => "unknown".to_string(),
        };

        entries.push(Worktree {
            name: wt_name,
            path: wt_path
                .to_str()
                .ok_or_else(|| RepositoryError::rule("invalid path encoding"))?
                .trim_end_matches('/')
                .to_string(),
            branch,
            is_main: false,
            is_locked,
        });
    }

    Ok(entries)
}

pub(crate) fn create_worktree(
    repo_path: &str,
    worktree_path: &str,
    branch: &str,
    create_branch: bool,
    base_branch: Option<&str>,
) -> Result<Worktree, RepositoryError> {
    let repo = client::open(repo_path)?;
    let wt_path = Path::new(worktree_path);

    // 壊れた worktree エントリを事前に掃除
    prune_invalid_worktrees(&repo);

    let reference = if create_branch {
        let base = base_branch.unwrap_or("HEAD");
        let obj = repo.revparse_single(base)?;
        let commit = obj.peel_to_commit()?;
        repo.branch(branch, &commit, false)?.into_reference()
    } else {
        repo.find_branch(branch, BranchType::Local)?
            .into_reference()
    };

    let wt_name = wt_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| RepositoryError::rule("invalid worktree path"))?;

    if let Some(parent) = wt_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            RepositoryError::rule(format!("failed to create parent directory: {e}"))
        })?;
    }

    let mut opts = WorktreeAddOptions::new();
    opts.reference(Some(&reference));

    if let Err(e) = repo.worktree(wt_name, wt_path, Some(&opts)) {
        if create_branch {
            let _ = repo
                .find_branch(branch, BranchType::Local)
                .and_then(|mut b| b.delete());
        }
        return Err(e.into());
    }

    Ok(Worktree {
        name: wt_name.to_string(),
        path: wt_path
            .to_str()
            .ok_or_else(|| RepositoryError::rule("invalid path encoding"))?
            .to_string(),
        branch: branch.to_string(),
        is_main: false,
        is_locked: false,
    })
}

pub(crate) fn remove_worktree(
    repo_path: &str,
    worktree_path: &str,
    force: bool,
) -> Result<Option<String>, RepositoryError> {
    let repo = client::open(repo_path)?;

    let target_path = Path::new(worktree_path)
        .canonicalize()
        .map_err(|e| RepositoryError::rule(format!("invalid worktree path: {e}")))?;

    let wt_names = repo.worktrees()?;

    let mut found_name: Option<String> = None;
    for (name, wt) in each_worktree(&repo, &wt_names) {
        if let Ok(canonical) = wt.path().canonicalize() {
            if canonical == target_path {
                found_name = Some(name);
                break;
            }
        }
    }

    let wt_name = found_name.ok_or_else(|| RepositoryError::rule("worktree not found"))?;
    let wt = repo.find_worktree(&wt_name)?;

    // worktree削除前にブランチ名を取得
    let wt_branch = Repository::open(wt.path())
        .ok()
        .map(|wt_repo| get_branch_name_for_repo(&wt_repo));

    let is_locked =
        matches!(wt.is_locked(), Ok(s) if !matches!(s, git2::WorktreeLockStatus::Unlocked));

    if !force {
        if is_locked {
            return Err(RepositoryError::rule("worktree is locked"));
        }
        let dirty = get_dirty_count_for_path(wt.path());
        if dirty > 0 {
            return Err(RepositoryError::rule(format!(
                "worktree has {dirty} uncommitted change(s). Use force to remove."
            )));
        }
    }

    let mut prune_opts = WorktreePruneOptions::new();
    prune_opts.valid(true).working_tree(true);
    if is_locked {
        prune_opts.locked(true);
    }
    wt.prune(Some(&mut prune_opts))?;

    if target_path.exists() {
        std::fs::remove_dir_all(&target_path).map_err(|e| {
            RepositoryError::rule(format!("failed to remove worktree directory: {e}"))
        })?;
    }

    // 対応ブランチの releash-base 後始末は usecase が wt_branch を使って行う。
    Ok(wt_branch)
}

/// `WorktreeRepository` の git2 実装。
pub struct WorktreeGateway;

impl WorktreeRepository for WorktreeGateway {
    fn main_repo_path(&self, any_path: &str) -> Result<String, RepositoryError> {
        get_main_repo_path(any_path)
    }
    fn dirty_count(&self, worktree_path: &str) -> Result<u32, RepositoryError> {
        get_worktree_dirty_count(worktree_path)
    }
    fn list(&self, repo_path: &str) -> Result<Vec<Worktree>, RepositoryError> {
        list_worktrees(repo_path)
    }
    fn create(
        &self,
        repo_path: &str,
        worktree_path: &str,
        branch: &str,
        create_branch: bool,
        base_branch: Option<&str>,
    ) -> Result<Worktree, RepositoryError> {
        create_worktree(repo_path, worktree_path, branch, create_branch, base_branch)
    }
    fn remove(
        &self,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<Option<String>, RepositoryError> {
        remove_worktree(repo_path, worktree_path, force)
    }
    fn prune_invalid(&self, repo_path: &str) -> Result<(), RepositoryError> {
        prune_invalid(repo_path)
    }
}

#[cfg(test)]
mod worktree_gateway_tests {
    use super::*;
    use crate::adaptor::gateway::repository::branch::get_current_branch;
    use crate::git::test_helpers::*;
    use std::fs;

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
    fn test_メインリポジトリパス取得_メインから() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let result = get_main_repo_path(dir.path().to_str().unwrap()).unwrap();
        let expected = dir.path().canonicalize().unwrap();
        let result_canon = PathBuf::from(&result).canonicalize().unwrap();
        assert_eq!(result_canon, expected);
    }

    #[test]
    fn test_メインリポジトリパス取得_worktreeから() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-test", "feat-test");

        let result = get_main_repo_path(wt_path.to_str().unwrap()).unwrap();
        let expected = repo_dir.canonicalize().unwrap();
        let result_canon = PathBuf::from(&result).canonicalize().unwrap();
        assert_eq!(result_canon, expected);
    }

    #[test]
    fn test_メインリポジトリパス取得_不正パス() {
        let result = get_main_repo_path("/nonexistent/invalid/path");
        assert!(result.is_err());
    }

    #[test]
    fn test_dirty_count取得_クリーン() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let count = get_worktree_dirty_count(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_dirty_count取得_変更あり() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();

        let count = get_worktree_dirty_count(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_worktree一覧_メインのみ() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let entries = list_worktrees(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_main);
        assert!(!entries[0].is_locked);
    }

    #[test]
    fn test_worktree一覧_リンク済み() {
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
    fn test_worktree一覧_ロック済み() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        create_worktree_helper(&repo, _parent.path(), "wt-lock", "feat-lock");
        let wt = repo.find_worktree("wt-lock").unwrap();
        wt.lock(None).unwrap();

        let entries = list_worktrees(repo_dir.to_str().unwrap()).unwrap();
        let locked_entry = entries.iter().find(|e| e.name == "wt-lock").unwrap();
        assert!(locked_entry.is_locked);
    }

    #[test]
    fn test_worktree作成_新規ブランチ() {
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
    fn test_worktree作成_既存ブランチ() {
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
    fn test_worktree作成_base引数でも作成成功() {
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
    fn test_worktree作成_重複ブランチ() {
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
    fn test_worktree作成_親ディレクトリ未存在() {
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
    fn test_worktree作成_失敗時ブランチロールバック() {
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
    fn test_worktree削除_クリーン() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-rm", "feat-rm");
        assert!(wt_path.exists());

        remove_worktree(repo_dir.to_str().unwrap(), wt_path.to_str().unwrap(), false).unwrap();

        assert!(!wt_path.exists());
    }

    #[test]
    fn test_worktree削除_dirty_forceなし() {
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
    fn test_worktree削除_dirty_force() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-dirtyf", "feat-dirtyf");
        fs::write(wt_path.join("dirty.txt"), "uncommitted").unwrap();

        remove_worktree(repo_dir.to_str().unwrap(), wt_path.to_str().unwrap(), true).unwrap();

        assert!(!wt_path.exists());
    }

    #[test]
    fn test_worktree削除_ロック_forceなし() {
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
    fn test_worktree削除_未発見() {
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
