use super::branch::detect_default_branch;
use super::error::GitError;
use super::types::{WorktreeBranch, WorktreeEntry};
use git2::{
    BranchType, ErrorCode, Oid, Repository, StatusOptions, WorktreeAddOptions, WorktreePruneOptions,
};
use std::path::{Path, PathBuf};

pub fn get_main_repo_path(any_path: String) -> Result<String, GitError> {
    let path = Path::new(&any_path);
    let repo = Repository::discover(path)?;

    if repo.is_worktree() {
        let git_dir = repo.path();
        let commondir_file = git_dir.join("commondir");
        if commondir_file.exists() {
            let content = std::fs::read_to_string(&commondir_file)?;
            let commondir = git_dir.join(content.trim());
            let commondir = commondir.canonicalize()?;
            let main_workdir = commondir
                .parent()
                .ok_or_else(|| GitError::Custom("cannot determine main repo path".to_string()))?;
            return Ok(main_workdir
                .to_str()
                .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))?
                .to_string());
        }
    }

    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;
    Ok(workdir
        .to_str()
        .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))?
        .trim_end_matches('/')
        .to_string())
}

pub fn get_worktree_dirty_count(worktree_path: String) -> Result<u32, GitError> {
    let repo = Repository::open(&worktree_path)?;

    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);

    let statuses = repo.statuses(Some(&mut opts))?;

    let count = statuses
        .iter()
        .filter(|entry| !entry.status().contains(git2::Status::IGNORED))
        .count() as u32;

    Ok(count)
}

pub(crate) fn get_branch_name_for_repo(repo: &Repository) -> String {
    match repo.head() {
        Ok(head) => {
            if head.is_branch() {
                head.shorthand().unwrap_or("HEAD").to_string()
            } else {
                let oid = head.target().map(|o| o.to_string());
                match oid {
                    Some(h) => format!("({})", &h[..7.min(h.len())]),
                    None => "HEAD".to_string(),
                }
            }
        }
        Err(e) if e.code() == ErrorCode::UnbornBranch => "(no commits)".to_string(),
        Err(_) => "unknown".to_string(),
    }
}

fn get_dirty_count_for_path(path: &Path) -> u32 {
    match Repository::open(path) {
        Ok(repo) => {
            let mut opts = StatusOptions::new();
            opts.include_untracked(true);
            match repo.statuses(Some(&mut opts)) {
                Ok(statuses) => statuses
                    .iter()
                    .filter(|entry| !entry.status().contains(git2::Status::IGNORED))
                    .count() as u32,
                Err(_) => 0,
            }
        }
        Err(_) => 0,
    }
}

fn resolve_main_repo_path(repo: &Repository) -> Result<PathBuf, GitError> {
    repo.workdir()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))
}

pub fn list_worktrees(repo_path: String) -> Result<Vec<WorktreeEntry>, GitError> {
    let repo = Repository::open(&repo_path)?;
    let main_workdir = resolve_main_repo_path(&repo)?;
    let mut entries = Vec::new();

    let main_branch = get_branch_name_for_repo(&repo);
    let main_dirty = get_dirty_count_for_path(&main_workdir);
    let main_name = main_workdir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("main")
        .to_string();

    let main_base = repo.config().ok().and_then(|cfg| {
        cfg.get_string(&format!("branch.{}.releash-base", main_branch))
            .ok()
    });

    entries.push(WorktreeEntry {
        name: main_name,
        path: main_workdir
            .to_str()
            .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))?
            .trim_end_matches('/')
            .to_string(),
        branch: main_branch,
        is_main: true,
        is_locked: false,
        dirty_count: main_dirty,
        base_branch: main_base,
    });

    let wt_names = repo.worktrees()?;
    for i in 0..wt_names.len() {
        let wt_name = match wt_names.get(i) {
            Some(name) => name.to_string(),
            None => continue,
        };

        let wt = match repo.find_worktree(&wt_name) {
            Ok(wt) => wt,
            Err(_) => continue,
        };

        if wt.validate().is_err() {
            continue;
        }

        let wt_path = wt.path();
        let is_locked =
            matches!(wt.is_locked(), Ok(s) if !matches!(s, git2::WorktreeLockStatus::Unlocked));

        let (branch, dirty_count, base_branch) = match Repository::open(wt_path) {
            Ok(wt_repo) => {
                let branch = get_branch_name_for_repo(&wt_repo);
                let dirty = get_dirty_count_for_path(wt_path);
                let base = wt_repo.config().ok().and_then(|cfg| {
                    cfg.get_string(&format!("branch.{}.releash-base", branch))
                        .ok()
                });
                (branch, dirty, base)
            }
            Err(_) => ("unknown".to_string(), 0, None),
        };

        entries.push(WorktreeEntry {
            name: wt_name,
            path: wt_path
                .to_str()
                .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))?
                .trim_end_matches('/')
                .to_string(),
            branch,
            is_main: false,
            is_locked,
            dirty_count,
            base_branch,
        });
    }

    Ok(entries)
}

fn is_on_first_parent_line(repo: &Repository, ancestor_oid: Oid, descendant_oid: Oid) -> bool {
    let mut current = descendant_oid;
    const MAX_DEPTH: usize = 10_000;
    for _ in 0..MAX_DEPTH {
        if current == ancestor_oid {
            return true;
        }
        match repo.find_commit(current) {
            Ok(commit) if commit.parent_count() > 0 => match commit.parent_id(0) {
                Ok(parent_id) => current = parent_id,
                Err(_) => return false,
            },
            _ => return false,
        }
    }
    false
}

fn compute_is_merged(repo: &Repository, branch_oid: Oid, base_target_oid: Option<Oid>) -> bool {
    base_target_oid
        .and_then(|t_oid| {
            if branch_oid == t_oid {
                return Some(false);
            }
            let merge_base = repo.merge_base(branch_oid, t_oid).ok()?;
            if merge_base != branch_oid {
                return Some(false);
            }
            Some(!is_on_first_parent_line(repo, branch_oid, t_oid))
        })
        .unwrap_or(false)
}

pub fn list_branches_with_status(repo_path: String) -> Result<Vec<WorktreeBranch>, GitError> {
    let repo = Repository::open(&repo_path)?;
    let default_branch = detect_default_branch(&repo);

    let default_oid = default_branch.as_ref().and_then(|name| {
        repo.find_branch(name, BranchType::Local)
            .ok()?
            .get()
            .target()
    });

    let mut wt_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    let main_branch = get_branch_name_for_repo(&repo);
    if let Some(workdir) = repo.workdir() {
        wt_map.insert(
            main_branch.clone(),
            workdir
                .to_str()
                .unwrap_or("")
                .trim_end_matches('/')
                .to_string(),
        );
    }

    if let Ok(wt_names) = repo.worktrees() {
        for i in 0..wt_names.len() {
            let wt_name = match wt_names.get(i) {
                Some(name) => name.to_string(),
                None => continue,
            };
            let wt = match repo.find_worktree(&wt_name) {
                Ok(wt) => wt,
                Err(_) => continue,
            };
            if wt.validate().is_err() {
                continue;
            }
            let wt_path = wt.path();
            if let Ok(wt_repo) = Repository::open(wt_path) {
                let branch = get_branch_name_for_repo(&wt_repo);
                wt_map.insert(
                    branch,
                    wt_path
                        .to_str()
                        .unwrap_or("")
                        .trim_end_matches('/')
                        .to_string(),
                );
            }
        }
    }

    let local_branches = repo.branches(Some(BranchType::Local))?;

    let config = repo.config().ok();

    let releash_base_name = config
        .as_ref()
        .and_then(|cfg| cfg.get_string("releash.base").ok());

    let base_target_oid = releash_base_name
        .as_ref()
        .and_then(|base_name| repo.find_branch(base_name, BranchType::Local).ok())
        .and_then(|b| b.get().target())
        .or(default_oid);

    let mut cards = Vec::new();
    for branch in local_branches {
        let (branch, _) = branch?;
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue,
        };

        let is_default = default_branch.as_deref() == Some(&name);

        let (worktree_path, dirty_count) = match wt_map.get(&name) {
            Some(path) => {
                let dirty = get_dirty_count_for_path(Path::new(path)) as usize;
                (Some(path.clone()), dirty)
            }
            None => (None, 0),
        };

        let is_merged = branch
            .get()
            .target()
            .map(|oid| compute_is_merged(&repo, oid, base_target_oid))
            .unwrap_or(false);

        let upstream = branch.upstream().ok();
        let has_upstream = upstream.is_some();
        let (ahead, behind) = upstream
            .and_then(|u| {
                let local_oid = branch.get().target()?;
                let remote_oid = u.get().target()?;
                repo.graph_ahead_behind(local_oid, remote_oid).ok()
            })
            .unwrap_or((0, 0));

        // base_ahead: branch.<name>.releash-base → releash.base → detect_default_branch
        let base_ahead = branch.get().target().and_then(|branch_oid| {
            let base_oid = config
                .as_ref()
                .and_then(|cfg| cfg.get_string(&format!("branch.{name}.releash-base")).ok())
                .and_then(|base_name| {
                    repo.find_branch(&base_name, BranchType::Local)
                        .ok()?
                        .get()
                        .target()
                })
                .or(base_target_oid);
            let base_oid = base_oid?;
            repo.graph_ahead_behind(branch_oid, base_oid)
                .ok()
                .map(|(a, _)| a)
        }).unwrap_or(0);

        cards.push(WorktreeBranch {
            name,
            is_default,
            worktree_path,
            dirty_count,
            is_merged,
            has_pr: false,
            pr_number: None,
            pr_url: None,
            ahead,
            behind,
            has_upstream,
            base_ahead,
        });
    }

    let local_names: std::collections::HashSet<String> =
        cards.iter().map(|c| c.name.clone()).collect();

    // GC: 存在しないブランチの branch.*.releash-base エントリを削除
    if let Ok(mut gc_cfg) = repo.config() {
        if let Ok(snap) = gc_cfg.snapshot() {
            let mut to_remove = Vec::new();
            if let Ok(mut entries) = snap.entries(Some("branch.*.releash-base")) {
                while let Some(Ok(entry)) = entries.next() {
                    if let Some(entry_name) = entry.name() {
                        if let Some(branch_name) = entry_name
                            .strip_prefix("branch.")
                            .and_then(|s| s.strip_suffix(".releash-base"))
                        {
                            if !local_names.contains(branch_name) {
                                to_remove.push(entry_name.to_string());
                            }
                        }
                    }
                }
            }
            drop(snap);
            for key in &to_remove {
                let _ = gc_cfg.remove(key);
            }
        }
    }

    Ok(cards)
}

pub fn create_worktree(
    repo_path: String,
    worktree_path: String,
    branch: String,
    create_branch: bool,
    base_branch: Option<String>,
) -> Result<WorktreeEntry, GitError> {
    let repo = Repository::open(&repo_path)?;
    let wt_path = Path::new(&worktree_path);

    // 壊れた worktree エントリを事前に掃除
    if let Ok(wt_names) = repo.worktrees() {
        for i in 0..wt_names.len() {
            if let Some(name) = wt_names.get(i) {
                if let Ok(wt) = repo.find_worktree(name) {
                    if wt.validate().is_err() {
                        let mut prune_opts = WorktreePruneOptions::new();
                        prune_opts.working_tree(true);
                        let _ = wt.prune(Some(&mut prune_opts));
                    }
                }
            }
        }
    }

    let reference = if create_branch {
        let base = base_branch.as_deref().unwrap_or("HEAD");
        let obj = repo.revparse_single(base)?;
        let commit = obj.peel_to_commit()?;
        repo.branch(&branch, &commit, false)?.into_reference()
    } else {
        repo.find_branch(&branch, BranchType::Local)?
            .into_reference()
    };

    let wt_name = wt_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| GitError::Custom("invalid worktree path".to_string()))?;

    if let Some(parent) = wt_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| GitError::Custom(format!("failed to create parent directory: {e}")))?;
    }

    let mut opts = WorktreeAddOptions::new();
    opts.reference(Some(&reference));

    if let Err(e) = repo.worktree(wt_name, wt_path, Some(&opts)) {
        if create_branch {
            let _ = repo
                .find_branch(&branch, BranchType::Local)
                .and_then(|mut b| b.delete());
        }
        return Err(e.into());
    }

    if let Some(ref base) = base_branch {
        let mut config = repo.config()?;
        config.set_str(&format!("branch.{}.releash-base", branch), base)?;
    }

    Ok(WorktreeEntry {
        name: wt_name.to_string(),
        path: wt_path
            .to_str()
            .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))?
            .to_string(),
        branch,
        is_main: false,
        is_locked: false,
        dirty_count: 0,
        base_branch,
    })
}

pub fn remove_worktree(
    repo_path: String,
    worktree_path: String,
    force: bool,
) -> Result<(), GitError> {
    let repo = Repository::open(&repo_path)?;

    let target_path = Path::new(&worktree_path)
        .canonicalize()
        .map_err(|e| GitError::Custom(format!("invalid worktree path: {e}")))?;

    let wt_names = repo.worktrees()?;

    let mut found_name: Option<String> = None;
    for i in 0..wt_names.len() {
        let name = match wt_names.get(i) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if let Ok(wt) = repo.find_worktree(&name) {
            if let Ok(canonical) = wt.path().canonicalize() {
                if canonical == target_path {
                    found_name = Some(name);
                    break;
                }
            }
        }
    }

    let wt_name = found_name.ok_or_else(|| GitError::Custom("worktree not found".to_string()))?;
    let wt = repo.find_worktree(&wt_name)?;

    // worktree削除前にブランチ名を取得
    let wt_branch = Repository::open(wt.path())
        .ok()
        .map(|wt_repo| get_branch_name_for_repo(&wt_repo));

    let is_locked =
        matches!(wt.is_locked(), Ok(s) if !matches!(s, git2::WorktreeLockStatus::Unlocked));

    if !force {
        if is_locked {
            return Err(GitError::Custom("worktree is locked".to_string()));
        }
        let dirty = get_dirty_count_for_path(wt.path());
        if dirty > 0 {
            return Err(GitError::Custom(format!(
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
        std::fs::remove_dir_all(&target_path)
            .map_err(|e| GitError::Custom(format!("failed to remove worktree directory: {e}")))?;
    }

    // worktree削除成功後、対応ブランチの releash-base を config から削除
    if let Some(branch_name) = wt_branch {
        if let Ok(mut config) = repo.config() {
            let key = format!("branch.{branch_name}.releash-base");
            match config.remove(&key) {
                Ok(()) | Err(_) => {}
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::branch::{detect_default_branch, get_current_branch};
    use crate::git::config::set_releash_base;
    use crate::git::test_helpers::*;
    use git2::build::CheckoutBuilder;
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
    fn test_get_main_repo_path_from_main_repo() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let result = get_main_repo_path(dir.path().to_str().unwrap().to_string()).unwrap();
        let expected = dir.path().canonicalize().unwrap();
        let result_canon = PathBuf::from(&result).canonicalize().unwrap();
        assert_eq!(result_canon, expected);
    }

    #[test]
    fn test_get_main_repo_path_from_worktree() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-test", "feat-test");

        let result = get_main_repo_path(wt_path.to_str().unwrap().to_string()).unwrap();
        let expected = repo_dir.canonicalize().unwrap();
        let result_canon = PathBuf::from(&result).canonicalize().unwrap();
        assert_eq!(result_canon, expected);
    }

    #[test]
    fn test_get_main_repo_path_invalid_path() {
        let result = get_main_repo_path("/nonexistent/invalid/path".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_get_worktree_dirty_count_clean() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let count = get_worktree_dirty_count(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_get_worktree_dirty_count_with_changes() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();

        let count = get_worktree_dirty_count(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_list_worktrees_main_only() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let entries = list_worktrees(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_main);
        assert!(!entries[0].is_locked);
    }

    #[test]
    fn test_list_worktrees_with_linked() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        create_worktree_helper(&repo, _parent.path(), "wt-linked", "feat-linked");

        let entries = list_worktrees(repo_dir.to_str().unwrap().to_string()).unwrap();
        assert_eq!(entries.len(), 2);

        let main_entry = entries.iter().find(|e| e.is_main).unwrap();
        assert!(main_entry.is_main);

        let linked_entry = entries.iter().find(|e| !e.is_main).unwrap();
        assert_eq!(linked_entry.name, "wt-linked");
        assert_eq!(linked_entry.branch, "feat-linked");
    }

    #[test]
    fn test_list_worktrees_locked() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        create_worktree_helper(&repo, _parent.path(), "wt-lock", "feat-lock");
        let wt = repo.find_worktree("wt-lock").unwrap();
        wt.lock(None).unwrap();

        let entries = list_worktrees(repo_dir.to_str().unwrap().to_string()).unwrap();
        let locked_entry = entries.iter().find(|e| e.name == "wt-lock").unwrap();
        assert!(locked_entry.is_locked);
    }

    #[test]
    fn test_list_worktrees_with_base() {
        let (_parent, repo_dir, _repo) = create_test_repo_with_parent();
        create_initial_commit(&_repo);

        let wt_path = _parent.path().join("wt-base");
        create_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            "feat-base".to_string(),
            true,
            Some("HEAD".to_string()),
        )
        .unwrap();

        let entries = list_worktrees(repo_dir.to_str().unwrap().to_string()).unwrap();
        let wt_entry = entries.iter().find(|e| e.name == "wt-base").unwrap();
        assert_eq!(wt_entry.base_branch, Some("HEAD".to_string()));
    }

    #[test]
    fn test_create_worktree_new_branch() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = _parent.path().join("wt-new");
        let entry = create_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            "feat-new".to_string(),
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
    fn test_create_worktree_existing_branch() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("existing-branch", &head, false).unwrap();

        let wt_path = _parent.path().join("wt-existing");
        let entry = create_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            "existing-branch".to_string(),
            false,
            None,
        )
        .unwrap();

        assert_eq!(entry.branch, "existing-branch");
        assert!(wt_path.exists());
    }

    #[test]
    fn test_create_worktree_with_base() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let main_branch = get_current_branch(repo_dir.to_str().unwrap().to_string()).unwrap();

        let wt_path = _parent.path().join("wt-withbase");
        create_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            "feat-withbase".to_string(),
            true,
            Some(main_branch.clone()),
        )
        .unwrap();

        let config = repo.config().unwrap();
        let base = config
            .get_string("branch.feat-withbase.releash-base")
            .unwrap();
        assert_eq!(base, main_branch);
    }

    #[test]
    fn test_create_worktree_duplicate_branch() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = _parent.path().join("wt-dup1");
        create_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            "feat-dup".to_string(),
            true,
            None,
        )
        .unwrap();

        let wt_path2 = _parent.path().join("wt-dup2");
        let result = create_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path2.to_str().unwrap().to_string(),
            "feat-dup".to_string(),
            true,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_create_worktree_parent_dir_not_exists() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = _parent.path().join("nested").join("deep").join("wt-new");
        assert!(!_parent.path().join("nested").exists());

        let entry = create_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            "feat-nested".to_string(),
            true,
            None,
        )
        .unwrap();

        assert_eq!(entry.branch, "feat-nested");
        assert!(wt_path.exists());
    }

    #[test]
    fn test_create_worktree_rollback_branch_on_failure() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path1 = _parent.path().join("wt-occupy");
        create_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path1.to_str().unwrap().to_string(),
            "feat-occupy".to_string(),
            true,
            None,
        )
        .unwrap();

        let wt_path2 = _parent.path().join("other").join("wt-occupy");
        let result = create_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path2.to_str().unwrap().to_string(),
            "feat-rollback".to_string(),
            true,
            None,
        );
        assert!(result.is_err());

        assert!(repo
            .find_branch("feat-rollback", BranchType::Local)
            .is_err());
    }

    #[test]
    fn test_remove_worktree_clean() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-rm", "feat-rm");
        assert!(wt_path.exists());

        remove_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            false,
        )
        .unwrap();

        assert!(!wt_path.exists());
    }

    #[test]
    fn test_remove_worktree_dirty_no_force() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-dirty", "feat-dirty");
        fs::write(wt_path.join("dirty.txt"), "uncommitted").unwrap();

        let result = remove_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            false,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("uncommitted change"));
        assert!(wt_path.exists());
    }

    #[test]
    fn test_remove_worktree_dirty_force() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-dirtyf", "feat-dirtyf");
        fs::write(wt_path.join("dirty.txt"), "uncommitted").unwrap();

        remove_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            true,
        )
        .unwrap();

        assert!(!wt_path.exists());
    }

    #[test]
    fn test_remove_worktree_locked_no_force() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-locked", "feat-locked");
        let wt = repo.find_worktree("wt-locked").unwrap();
        wt.lock(None).unwrap();

        let result = remove_worktree(
            repo_dir.to_str().unwrap().to_string(),
            wt_path.to_str().unwrap().to_string(),
            false,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("locked"));
    }

    #[test]
    fn test_remove_worktree_not_found() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let result = remove_worktree(
            dir.path().to_str().unwrap().to_string(),
            dir.path().join("nonexistent").to_str().unwrap().to_string(),
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_list_branches_with_status_includes_default() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let cards = list_branches_with_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(cards.len(), 1);
        assert!(cards[0].is_default);
    }

    #[test]
    fn test_list_branches_with_status_shows_non_default() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-a", &head, false).unwrap();
        repo.branch("feature-b", &head, false).unwrap();

        let cards = list_branches_with_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(cards.len(), 3);
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"feature-a"));
        assert!(names.contains(&"feature-b"));
        for card in cards.iter().filter(|c| !c.is_default) {
            assert!(card.worktree_path.is_none());
            assert_eq!(card.dirty_count, 0);
        }
    }

    #[test]
    fn test_list_branches_with_status_with_worktree() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-feat", "feat-wt");
        fs::write(wt_path.join("dirty.txt"), "dirty").unwrap();

        let cards = list_branches_with_status(repo_dir.to_str().unwrap().to_string()).unwrap();
        let feat_card = cards.iter().find(|c| c.name == "feat-wt").unwrap();
        assert!(feat_card.worktree_path.is_some());
        assert_eq!(feat_card.dirty_count, 1);
    }

    #[test]
    fn test_list_branches_behind_not_merged() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-behind", &head, false).unwrap();

        add_and_commit(&repo, "after.txt", "after", "commit after branch");

        let cards = list_branches_with_status(dir.path().to_str().unwrap().to_string()).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-behind").unwrap();
        assert!(
            !card.is_merged,
            "branch with no unique commits should not be merged when base advances"
        );
    }

    #[test]
    fn test_list_branches_merged_via_merge_commit() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let branch_ref = repo.branch("feature-merged", &head, false).unwrap();
        let branch_ref = branch_ref.into_reference();

        repo.set_head(branch_ref.name().unwrap()).unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        add_and_commit(&repo, "feat.txt", "feat", "feature commit");
        let feature_commit = repo.head().unwrap().peel_to_commit().unwrap();

        let default_branch = detect_default_branch(&repo).unwrap();
        repo.set_head(&format!("refs/heads/{}", default_branch))
            .unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();

        add_and_commit(&repo, "main.txt", "main", "main commit");
        let main_commit = repo.head().unwrap().peel_to_commit().unwrap();

        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "Merge feature-merged",
            &tree,
            &[&main_commit, &feature_commit],
        )
        .unwrap();

        let cards = list_branches_with_status(dir.path().to_str().unwrap().to_string()).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-merged").unwrap();
        assert!(
            card.is_merged,
            "branch merged via merge commit should be detected as merged"
        );
    }

    #[test]
    fn test_list_branches_unmerged_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let branch_ref = repo.branch("feature-unmerged", &head, false).unwrap();
        let branch_ref = branch_ref.into_reference();
        repo.set_head(branch_ref.name().unwrap()).unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        add_and_commit(&repo, "feat.txt", "feat", "feature commit");

        let default_branch = detect_default_branch(&repo).unwrap();
        repo.set_head(&format!("refs/heads/{}", default_branch))
            .unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();

        let cards = list_branches_with_status(dir.path().to_str().unwrap().to_string()).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-unmerged").unwrap();
        assert!(!card.is_merged);
    }

    #[test]
    fn test_list_branches_same_oid_not_merged() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-same", &head, false).unwrap();

        let cards = list_branches_with_status(dir.path().to_str().unwrap().to_string()).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-same").unwrap();
        assert!(!card.is_merged);
    }

    #[test]
    fn test_list_branches_merged_via_releash_base() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let develop_branch = repo.branch("develop", &head, false).unwrap();
        let develop_ref = develop_branch.into_reference();

        repo.set_head(develop_ref.name().unwrap()).unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        add_and_commit(&repo, "dev.txt", "dev", "develop commit");

        let develop_head = repo.head().unwrap().peel_to_commit().unwrap();
        let feature_ref = repo.branch("feature-x", &develop_head, false).unwrap();
        let feature_ref = feature_ref.into_reference();

        repo.set_head(feature_ref.name().unwrap()).unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        add_and_commit(&repo, "feat.txt", "feat", "feature commit");
        let feature_commit = repo.head().unwrap().peel_to_commit().unwrap();

        repo.set_head("refs/heads/develop").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        add_and_commit(&repo, "dev2.txt", "dev2", "develop commit 2");
        let develop_commit = repo.head().unwrap().peel_to_commit().unwrap();

        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "Merge feature-x into develop",
            &tree,
            &[&develop_commit, &feature_commit],
        )
        .unwrap();

        let default_branch = detect_default_branch(&repo).unwrap();
        repo.set_head(&format!("refs/heads/{}", default_branch))
            .unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();

        let repo_path = dir.path().to_str().unwrap().to_string();

        let cards = list_branches_with_status(repo_path.clone()).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-x").unwrap();
        assert!(
            !card.is_merged,
            "without releash.base, should not be merged into main"
        );

        set_releash_base(repo_path.clone(), Some("develop".to_string())).unwrap();

        let cards = list_branches_with_status(repo_path).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-x").unwrap();
        assert!(
            card.is_merged,
            "with releash.base=develop, should be merged"
        );
    }

    #[test]
    fn test_list_branches_with_status_ahead_behind() {
        let (_parent, clone_dir, repo) = setup_remote_repo();

        add_and_commit(&repo, "local.txt", "local", "local commit");

        let cards = list_branches_with_status(clone_dir.to_str().unwrap().to_string()).unwrap();
        let card = cards.iter().find(|c| c.name == "main").unwrap();
        assert_eq!(card.ahead, 1);
        assert_eq!(card.behind, 0);
    }

    #[test]
    fn test_list_branches_with_status_no_upstream() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("no-upstream", &head, false).unwrap();

        let cards = list_branches_with_status(dir.path().to_str().unwrap().to_string()).unwrap();
        let card = cards.iter().find(|c| c.name == "no-upstream").unwrap();
        assert_eq!(card.ahead, 0);
        assert_eq!(card.behind, 0);
    }
}
