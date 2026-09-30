//! worktree 責務の gateway 実装。git2 によるワークツリー操作を封じ込める。

use crate::adaptor::gateway::shared::git_operation;
use crate::adaptor::gateway::shared::git_operation::{
    detect_default_branch, get_branch_name_for_repo,
};
use crate::common::operation_context::OperationStopped;
use crate::domain::repository::{
    normalize_repo_path, BaseAncestry, RepositoryError, Worktree, WorktreeRepository,
};
use crate::infrastructure::git::client;
use git2::{BranchType, Oid, Repository, StatusOptions, WorktreeAddOptions, WorktreePruneOptions};
use std::path::{Path, PathBuf};

pub(crate) fn get_main_repo_path(any_path: &str) -> Result<String, RepositoryError> {
    let repo = git_operation::run(|| client::discover(any_path))?;
    main_repo_path(&repo)
}

pub(crate) fn find_main_repo_path(any_path: &str) -> Result<Option<String>, RepositoryError> {
    match git_operation::run(|| client::discover(any_path)) {
        Ok(repo) => main_repo_path(&repo).map(Some),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn recorded_main_repo_path(
    path: &str,
) -> Result<Option<String>, crate::common::operation_context::OperationStopped> {
    if let Some(repository) = git_operation::optional(git_operation::run(|| client::open(path)))? {
        if let Ok(path) = main_repo_path(&repository) {
            return Ok(Some(path));
        }
    }
    Ok((|| {
        let git_file = std::fs::read_to_string(Path::new(path).join(".git")).ok()?;
        let git_dir = Path::new(path).join(git_file.trim().strip_prefix("gitdir: ")?);
        let worktrees = git_dir.parent()?;
        if worktrees.file_name()? != "worktrees" {
            return None;
        }
        let common_dir = worktrees.parent()?;
        if common_dir.file_name()? != ".git" {
            return None;
        }
        super::worktree_operation::worktree_identity(common_dir.parent()?.to_str()?)
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
    })())
}

fn main_repo_path(repo: &Repository) -> Result<String, RepositoryError> {
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
            return path_to_worktree_identity(main_workdir);
        }
    }

    let workdir = repo
        .workdir()
        .ok_or_else(|| RepositoryError::rule("bare repository"))?;
    path_to_worktree_identity(workdir)
}

fn path_to_normalized_repo_string(path: &Path) -> Result<String, RepositoryError> {
    let path = path
        .to_str()
        .ok_or_else(|| RepositoryError::rule("invalid path encoding"))?;
    Ok(normalize_repo_path(path))
}

fn path_to_worktree_identity(path: &Path) -> Result<String, RepositoryError> {
    let path = path_to_normalized_repo_string(path)?;
    let identity = super::worktree_operation::worktree_identity(&path)?;
    path_to_normalized_repo_string(&identity)
}

/// worktree の dirty 件数を算出する共通ロジック。
/// 未追跡ディレクトリ配下も再帰的に個別計上し、ignored は除外する。
/// 算出条件（`StatusOptions`）を 1 箇所に集約し、経路ごとの差異を排除する。
fn count_dirty_entries(repo: &Repository) -> Result<u32, RepositoryError> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = git_operation::run(|| repo.statuses(Some(&mut opts)))?;
    Ok(statuses
        .iter()
        .filter(|entry| !entry.status().contains(git2::Status::IGNORED))
        .count() as u32)
}

pub(crate) fn get_worktree_dirty_count(worktree_path: &str) -> Result<u32, RepositoryError> {
    let repo = git_operation::run(|| client::open(worktree_path))?;
    count_dirty_entries(&repo)
}

/// repo のリンク済み worktree を `(name, Worktree)` で列挙する。
/// `worktrees()` の index 走査と `find_worktree` の定型を集約する。
/// `validate()` / `prune` / パス比較は用途ごとに呼び出し側で行う。
pub(super) fn each_worktree<'a>(
    repo: &'a Repository,
    names: &'a git2::string_array::StringArray,
) -> impl Iterator<
    Item = Result<(String, git2::Worktree), crate::common::operation_context::OperationStopped>,
> + 'a {
    (0..names.len()).filter_map(move |i| {
        let name = match names.get(i) {
            Ok(Some(n)) => n.to_string(),
            _ => return None,
        };
        git_operation::optional(git_operation::run(|| repo.find_worktree(&name)))
            .map(|worktree| worktree.map(|wt| (name, wt)))
            .transpose()
    })
}

fn resolve_main_repo_path(repo: &Repository) -> Result<PathBuf, RepositoryError> {
    repo.workdir()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| RepositoryError::rule("bare repository"))
}

/// 壊れた（`validate()` 失敗）linked worktree を working tree ごと prune する。
/// 個別エントリの prune 失敗は無視する（best-effort）。`create_worktree` の事前掃除に使う。
fn prune_invalid_worktrees(repo: &Repository) -> Result<(), RepositoryError> {
    if let Some(wt_names) = git_operation::optional(git_operation::run(|| repo.worktrees()))? {
        for entry in each_worktree(repo, &wt_names) {
            let (_, wt) = entry?;
            if git_operation::optional(git_operation::run(|| wt.validate()))?.is_none() {
                let mut prune_opts = WorktreePruneOptions::new();
                prune_opts.working_tree(true);
                git_operation::optional(git_operation::run(|| wt.prune(Some(&mut prune_opts))))?;
            }
        }
    }
    Ok(())
}

pub(crate) fn registered_worktree_paths(
    repo_path: &str,
) -> Result<Vec<(String, String)>, RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let main = resolve_main_repo_path(&repo)?;
    let name = main
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("main")
        .to_string();
    let mut entries = vec![(name, path_to_normalized_repo_string(&main)?)];
    let names = git_operation::run(|| repo.worktrees())?;
    for index in 0..names.len() {
        let Some(name) = names.get(index)? else {
            continue;
        };
        let worktree = git_operation::run(|| repo.find_worktree(name))?;
        entries.push((
            name.to_string(),
            path_to_normalized_repo_string(worktree.path())?,
        ));
    }
    Ok(entries)
}

fn is_on_first_parent_line(
    repo: &Repository,
    ancestor_oid: Oid,
    descendant_oid: Oid,
) -> Result<bool, OperationStopped> {
    let mut current = descendant_oid;
    const MAX_DEPTH: usize = 10_000;
    for _ in 0..MAX_DEPTH {
        if current == ancestor_oid {
            return Ok(true);
        }
        let Some(commit) =
            git_operation::optional(git_operation::run(|| repo.find_commit(current)))?
        else {
            return Ok(false);
        };
        if commit.parent_count() == 0 {
            return Ok(false);
        }
        let Some(parent_id) = git_operation::optional(git_operation::run(|| commit.parent_id(0)))?
        else {
            return Ok(false);
        };
        current = parent_id;
    }
    Ok(false)
}

/// merge 先の base の先頭を解決する: `releash.base`（設定）→ 既定ブランチ（fallback）。
fn resolve_base_target_oid(repo: &Repository) -> Result<Option<Oid>, OperationStopped> {
    let local_tip = |name: &str| -> Result<Option<Oid>, OperationStopped> {
        Ok(git_operation::optional(git_operation::run(|| {
            repo.find_branch(name, BranchType::Local)
        }))?
        .and_then(|branch| branch.get().target()))
    };
    let config = git_operation::optional(git_operation::run(|| repo.config()))?;
    let base_name = config
        .map(|config| {
            git_operation::optional(git_operation::run(|| config.get_string("releash.base")))
        })
        .transpose()?
        .flatten();
    if let Some(oid) = base_name
        .map(|name| local_tip(&name))
        .transpose()?
        .flatten()
    {
        return Ok(Some(oid));
    }
    detect_default_branch(repo)?
        .map(|name| local_tip(&name))
        .transpose()
        .map(Option::flatten)
}

/// ブランチの先頭と base の履歴の関係を読む。判定は domain（`BaseAncestry`）が行う。
fn base_ancestry(
    repo: &Repository,
    branch_name: &str,
    base_target_oid: Option<Oid>,
) -> Result<Option<BaseAncestry>, OperationStopped> {
    let Some(branch_oid) = git_operation::optional(git_operation::run(|| {
        repo.find_branch(branch_name, BranchType::Local)
    }))?
    .and_then(|branch| branch.get().target()) else {
        return Ok(None);
    };
    let Some(target) = base_target_oid else {
        return Ok(None);
    };
    let in_base_history =
        git_operation::optional(git_operation::run(|| repo.merge_base(branch_oid, target)))?
            == Some(branch_oid);
    Ok(Some(BaseAncestry {
        in_base_history,
        on_base_first_parent: in_base_history && is_on_first_parent_line(repo, branch_oid, target)?,
    }))
}

pub(crate) fn list_worktrees(repo_path: &str) -> Result<Vec<Worktree>, RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let main_workdir = resolve_main_repo_path(&repo)?;
    let base_target_oid = resolve_base_target_oid(&repo)?;
    let is_merged = |branch: &str| -> Result<bool, OperationStopped> {
        Ok(base_ancestry(&repo, branch, base_target_oid)?.is_some_and(BaseAncestry::is_merged))
    };
    let mut entries = Vec::new();

    let main_branch = get_branch_name_for_repo(&repo)?;
    let main_name = main_workdir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("main")
        .to_string();

    entries.push(Worktree {
        name: main_name,
        path: path_to_worktree_identity(&main_workdir)?,
        is_merged: is_merged(&main_branch)?,
        branch: main_branch,
        is_main: true,
        is_locked: false,
    });

    let wt_names = git_operation::run(|| repo.worktrees())?;
    for entry in each_worktree(&repo, &wt_names) {
        let (wt_name, wt) = entry?;
        if git_operation::optional(git_operation::run(|| wt.validate()))?.is_none() {
            continue;
        }

        let wt_path = wt.path();
        let is_locked = matches!(git_operation::optional(git_operation::run(|| wt.is_locked()))?, Some(s) if !matches!(s, git2::WorktreeLockStatus::Unlocked));

        let branch = match git_operation::run(|| Repository::open(wt_path)) {
            Ok(wt_repo) => get_branch_name_for_repo(&wt_repo)?,
            Err(git_operation::GitOperationError::Stopped(error)) => return Err(error.into()),
            Err(_) => "unknown".to_string(),
        };

        entries.push(Worktree {
            name: wt_name,
            path: path_to_worktree_identity(wt_path)?,
            is_merged: is_merged(&branch)?,
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
    let repo = git_operation::run(|| client::open(repo_path))?;
    let wt_path = Path::new(worktree_path);

    // 壊れた worktree エントリを事前に掃除
    prune_invalid_worktrees(&repo)?;

    let reference = if create_branch {
        let base = base_branch.unwrap_or("HEAD");
        let obj = git_operation::run(|| repo.revparse_single(base))?;
        let commit = git_operation::run(|| obj.peel_to_commit())?;
        git_operation::run(|| repo.branch(branch, &commit, false))?.into_reference()
    } else {
        git_operation::run(|| repo.find_branch(branch, BranchType::Local))?.into_reference()
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

    if let Err(e) = git_operation::run(|| repo.worktree(wt_name, wt_path, Some(&opts))) {
        if let git_operation::GitOperationError::Stopped(stopped) = e {
            return Err(stopped.into());
        }
        if create_branch {
            git_operation::optional(
                git_operation::run(|| repo.find_branch(branch, BranchType::Local))
                    .and_then(|mut b| git_operation::run(|| b.delete())),
            )?;
        }
        return Err(e.into());
    }

    Ok(Worktree {
        name: wt_name.to_string(),
        path: path_to_worktree_identity(wt_path)?,
        branch: branch.to_string(),
        is_main: false,
        is_locked: false,
        is_merged: false,
    })
}

fn removal_target(
    repo_path: &str,
    worktree_path: &str,
    force: bool,
) -> Result<(git2::Worktree, PathBuf, Option<String>, bool), RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;

    let target_path = Path::new(worktree_path)
        .canonicalize()
        .map_err(|e| RepositoryError::rule(format!("invalid worktree path: {e}")))?;

    let wt_names = git_operation::run(|| repo.worktrees())?;

    let mut found_name: Option<String> = None;
    for entry in each_worktree(&repo, &wt_names) {
        let (name, wt) = entry?;
        if let Ok(canonical) = wt.path().canonicalize() {
            if canonical == target_path {
                found_name = Some(name);
                break;
            }
        }
    }

    let wt_name = found_name.ok_or_else(|| RepositoryError::rule("worktree not found"))?;
    let wt = git_operation::run(|| repo.find_worktree(&wt_name))?;

    // worktree削除前にブランチ名を取得
    let wt_branch = git_operation::optional(git_operation::run(|| Repository::open(wt.path())))?
        .map(|wt_repo| get_branch_name_for_repo(&wt_repo))
        .transpose()?;

    let is_locked = !matches!(
        git_operation::run(|| wt.is_locked())?,
        git2::WorktreeLockStatus::Unlocked
    );
    Worktree {
        name: wt_name,
        path: target_path.to_string_lossy().into_owned(),
        branch: wt_branch.clone().unwrap_or_default(),
        is_main: false,
        is_locked,
        is_merged: false,
    }
    .authorize_removal(
        force,
        if force {
            0
        } else {
            get_worktree_dirty_count(worktree_path)?
        },
    )?;
    Ok((wt, target_path, wt_branch, is_locked))
}

pub(crate) fn remove_worktree(
    repo_path: &str,
    worktree_path: &str,
    force: bool,
) -> Result<Option<String>, RepositoryError> {
    let (wt, target_path, wt_branch, is_locked) = removal_target(repo_path, worktree_path, force)?;
    let mut prune_opts = WorktreePruneOptions::new();
    prune_opts.valid(true).working_tree(true);
    if is_locked {
        prune_opts.locked(true);
    }
    git_operation::run(|| wt.prune(Some(&mut prune_opts)))?;

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
    fn validate_removal(
        &self,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<String, RepositoryError> {
        let (_, path, _, _) = removal_target(repo_path, worktree_path, force)?;
        path_to_normalized_repo_string(&path)
    }
    fn remove(
        &self,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<Option<String>, RepositoryError> {
        remove_worktree(repo_path, worktree_path, force)
    }
}

#[cfg(test)]
#[path = "worktree_test.rs"]
mod worktree_tests;

#[cfg(test)]
mod worktree_gateway_tests {
    use super::*;
    use crate::adaptor::gateway::repository::branch::get_current_branch;
    use crate::test_support::git::*;
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
        assert_eq!(
            find_main_repo_path(dir.path().to_str().unwrap()).unwrap(),
            Some(result)
        );
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
        assert_eq!(
            find_main_repo_path(wt_path.to_str().unwrap()).unwrap(),
            Some(result)
        );
    }

    #[test]
    fn test_旧worktreeの所属repo_git管理情報がなければ親repoを所属と推定しない() {
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
    fn test_旧worktreeの所属repo_相対gitdirでも登録消失後の正規パスを返す() {
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
    fn test_メインリポジトリパス正規化_unc_prefix保持() {
        let path = Path::new(r"\\server\share\repo");

        let result = path_to_normalized_repo_string(path).unwrap();

        assert_eq!(result, "//server/share/repo");
    }

    #[test]
    fn test_メインリポジトリパス取得_不正パス() {
        let result = get_main_repo_path("/nonexistent/invalid/path");
        assert!(result.is_err());
        assert_eq!(
            find_main_repo_path("/nonexistent/invalid/path").unwrap(),
            None
        );
        assert!(find_main_repo_path("\0").is_err());
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
    fn test_gc用一覧_フォルダ消失後もgit登録を保持し登録削除後は除外する() {
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
    fn test_worktree一覧_baseが進んだだけのブランチは未マージ() {
        // Given: 固有の commit を持たないブランチの後に base が進む
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        create_worktree_helper(&repo, parent.path(), "wt-behind", "feature-behind");
        add_and_commit(&repo, "after.txt", "after", "commit after branch");

        // When / Then
        assert!(!merged_of(&repo_dir, "feature-behind"));
    }

    #[test]
    fn test_worktree一覧_baseと同じ先頭のブランチは未マージ() {
        // Given
        let (parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);
        create_worktree_helper(&repo, parent.path(), "wt-same", "feature-same");

        // When / Then
        assert!(!merged_of(&repo_dir, "feature-same"));
        assert!(!list_worktrees(repo_dir.to_str().unwrap()).unwrap()[0].is_merged);
    }

    #[test]
    fn test_worktree一覧_固有のcommitを持つブランチは未マージ() {
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
    fn test_worktree一覧_merge_commit経由で取り込まれたブランチはマージ済み() {
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
    fn test_worktree一覧_releash_baseが指すブランチへの取り込みでマージ済みを判定する() {
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
        crate::adaptor::gateway::repository::git_config::set_releash_base(
            repo_dir.to_str().unwrap(),
            Some("develop"),
        )
        .unwrap();
        assert!(merged_of(&repo_dir, "feature-x"));
    }

    #[test]
    fn test_worktree一覧_detached_headのworktreeはブランチ名を括弧で表し未マージ() {
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
