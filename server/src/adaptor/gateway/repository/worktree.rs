//! worktree 責務の gateway 実装。git2 によるワークツリー操作を封じ込める。

use crate::adaptor::gateway::shared::git_operation;
use crate::adaptor::gateway::shared::git_operation::GitOperationError;
use crate::adaptor::gateway::shared::git_operation::{
    detect_default_branch, get_branch_name_for_repo,
};
use crate::domain::repository::{
    normalize_repo_path, BaseAncestry, RepositoryError, Worktree, WorktreeRepository,
};
use crate::infrastructure::git::client;
use git2::{BranchType, Oid, Repository, StatusOptions, WorktreeAddOptions, WorktreePruneOptions};
use std::path::{Path, PathBuf};

pub fn get_main_repo_path(any_path: &str) -> Result<String, RepositoryError> {
    let repo = git_operation::run(|| client::discover(any_path))?;
    main_repo_path(&repo)
}

pub fn find_main_repo_path(any_path: &str) -> Result<Option<String>, RepositoryError> {
    match git_operation::run(|| client::discover(any_path)) {
        Ok(repo) => main_repo_path(&repo).map(Some),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn recorded_main_repo_path(path: &str) -> Result<Option<String>, RepositoryError> {
    if let Some(repository) = git_operation::optional(git_operation::run(|| client::open(path)))? {
        return main_repo_path(&repository).map(Some);
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

pub fn path_to_worktree_identity(path: &Path) -> Result<String, RepositoryError> {
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

pub fn get_worktree_dirty_count(worktree_path: &str) -> Result<u32, RepositoryError> {
    let repo = git_operation::run(|| client::open(worktree_path))?;
    count_dirty_entries(&repo)
}

/// repo のリンク済み worktree を `(name, Worktree)` で列挙する。
/// `worktrees()` の index 走査と `find_worktree` の定型を集約する。
/// `validate()` / `prune` / パス比較は用途ごとに呼び出し側で行う。
pub fn each_worktree<'a>(
    repo: &'a Repository,
    names: &'a git2::string_array::StringArray,
) -> impl Iterator<
    Item = Result<
        (String, git2::Worktree),
        crate::adaptor::gateway::shared::git_operation::GitOperationError,
    >,
> + 'a {
    (0..names.len()).filter_map(move |i| {
        let result = (|| {
            let name = names
                .get(i)?
                .ok_or_else(|| git2::Error::from_str("Invalid worktree name"))?
                .to_string();
            let wt = git_operation::run(|| repo.find_worktree(&name))?;
            Ok((name, wt))
        })();
        match result {
            Err(crate::adaptor::gateway::shared::git_operation::GitOperationError::Git(error))
                if error.code() == git2::ErrorCode::NotFound =>
            {
                None
            }
            result => Some(result),
        }
    })
}

fn resolve_main_repo_path(repo: &Repository) -> Result<PathBuf, RepositoryError> {
    repo.workdir()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| RepositoryError::rule("bare repository"))
}

/// 壊れた（`validate()` 失敗）linked worktree を working tree ごと prune する。
/// 個別エントリの prune 失敗は無視する（best-effort）。`create_worktree` の事前掃除に使う。
pub fn prune_invalid_worktrees(repo: &Repository) -> Result<(), RepositoryError> {
    if let Some(wt_names) = git_operation::optional(git_operation::run(|| repo.worktrees()))? {
        for entry in each_worktree(repo, &wt_names) {
            let (_, wt) = entry?;
            if match git_operation::run(|| wt.validate()) {
                Ok(()) => false,
                Err(git_operation::GitOperationError::Git(_)) => true,
                Err(error) => return Err(error.into()),
            } {
                let mut prune_opts = WorktreePruneOptions::new();
                prune_opts.working_tree(true);
                git_operation::optional(git_operation::run(|| wt.prune(Some(&mut prune_opts))))?;
            }
        }
    }
    Ok(())
}

pub fn registered_worktree_paths(
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
) -> Result<bool, GitOperationError> {
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
fn resolve_base_target_oid(repo: &Repository) -> Result<Option<Oid>, GitOperationError> {
    let local_tip = |name: &str| -> Result<Option<Oid>, GitOperationError> {
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
) -> Result<Option<BaseAncestry>, GitOperationError> {
    let Some(target) = base_target_oid else {
        return Ok(None);
    };
    let Some(branch_oid) = git_operation::optional(git_operation::run(|| {
        repo.find_branch(branch_name, BranchType::Local)
    }))?
    .and_then(|branch| branch.get().target()) else {
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

pub fn list_worktrees(repo_path: &str) -> Result<Vec<Worktree>, RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let main_workdir = resolve_main_repo_path(&repo)?;
    let base_target_oid = resolve_base_target_oid(&repo)?;
    let is_merged = |source: &Repository| -> Result<bool, GitOperationError> {
        if base_target_oid.is_none() {
            return Ok(false);
        }
        let Some(head) =
            git_operation::optional(git_operation::run(|| source.find_reference("HEAD")))?
        else {
            return Ok(false);
        };
        let Some(head) = git_operation::optional(git_operation::run(|| head.resolve()))? else {
            return Ok(false);
        };
        if !head.is_branch() {
            return Ok(false);
        }
        Ok(base_ancestry(&repo, head.shorthand()?, base_target_oid)?
            .is_some_and(BaseAncestry::is_merged))
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
        is_merged: is_merged(&repo)?,
        branch: main_branch,
        is_main: true,
        is_locked: false,
    });

    let wt_names = git_operation::run(|| repo.worktrees())?;
    for entry in each_worktree(&repo, &wt_names) {
        let (wt_name, wt) = entry?;
        if match git_operation::run(|| wt.validate()) {
            Ok(()) => false,
            Err(git_operation::GitOperationError::Git(_)) => true,
            Err(error) => return Err(error.into()),
        } {
            continue;
        }

        let wt_path = wt.path();
        let is_locked = !matches!(
            git_operation::run(|| wt.is_locked())?,
            git2::WorktreeLockStatus::Unlocked
        );

        let wt_repo = git_operation::run(|| Repository::open(wt_path))?;
        let branch = get_branch_name_for_repo(&wt_repo)?;

        entries.push(Worktree {
            name: wt_name,
            path: path_to_worktree_identity(wt_path)?,
            is_merged: is_merged(&wt_repo)?,
            branch,
            is_main: false,
            is_locked,
        });
    }

    Ok(entries)
}

pub fn create_worktree(
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

    let wt_name = wt_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| RepositoryError::rule("invalid worktree path"))?;

    let names = git_operation::run(|| repo.worktrees())?;
    let names = git_operation::run(|| names.iter().collect::<Result<Vec<_>, _>>())?;
    let mut unique_name = wt_name.to_string();
    let mut suffix = 1;
    while names.iter().flatten().any(|name| *name == unique_name) {
        unique_name = format!("{wt_name}{suffix}");
        suffix += 1;
    }

    let reference = if create_branch {
        let base = base_branch.unwrap_or("HEAD");
        let obj = git_operation::run(|| repo.revparse_single(base))?;
        let commit = git_operation::run(|| obj.peel_to_commit())?;
        git_operation::run(|| repo.branch(branch, &commit, false))?.into_reference()
    } else {
        git_operation::run(|| repo.find_branch(branch, BranchType::Local))?.into_reference()
    };

    if let Some(parent) = wt_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            RepositoryError::rule(format!("failed to create parent directory: {e}"))
        })?;
    }

    let mut opts = WorktreeAddOptions::new();
    opts.reference(Some(&reference));

    if let Err(e) = git_operation::run(|| repo.worktree(&unique_name, wt_path, Some(&opts))) {
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
        name: unique_name,
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

pub fn remove_worktree(
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
    fn find_main_repo_path(&self, path: &str) -> Result<Option<String>, RepositoryError> {
        find_main_repo_path(path)
    }
    fn main_repo_path(&self, any_path: &str) -> Result<String, RepositoryError> {
        get_main_repo_path(any_path)
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
