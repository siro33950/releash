//! ブランチカード（Query 側）の gateway 実装。
//!
//! ブランチ + worktree 配置 + dirty + マージ状態 + ahead/behind を突き合わせた表示用
//! read model（`BranchCardDto`）を、中間 Entity を介さずデータソース（git2）から直接構築する。
//! Command 側（`WorktreeRepository` 等の単一集約 I/O）とはファイルを分離する（CQRS）。

use super::util::resolve_branch_base;
use super::worktree::each_worktree;
use crate::domain::repository::RepositoryError;
use crate::infrastructure::git::client;
use crate::infrastructure::git::helpers::{detect_default_branch, get_branch_name_for_repo};
use crate::usecase::repository_dto::BranchCardDto;
use crate::usecase::repository_query_service::BranchCardQuery;
use git2::{BranchType, Oid, Repository};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[cfg(test)]
thread_local! {
    static DIRTY_WORKTREE_SCAN_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn normalize_worktree_path(path: &str) -> String {
    path.trim_end_matches('/').to_string()
}

struct DirtyCountSnapshot {
    counts_by_path: HashMap<String, usize>,
}

impl DirtyCountSnapshot {
    fn empty() -> Self {
        Self {
            counts_by_path: HashMap::new(),
        }
    }

    fn with_current(current_workdir: Option<&str>, current_dirty_count: usize) -> Self {
        let mut snapshot = Self::empty();
        if let Some(path) = current_workdir {
            snapshot
                .counts_by_path
                .insert(normalize_worktree_path(path), current_dirty_count);
        }
        snapshot
    }

    fn dirty_count_for_path(&mut self, path: &str) -> usize {
        let key = normalize_worktree_path(path);
        if let Some(count) = self.counts_by_path.get(&key) {
            return *count;
        }
        let count = calculate_dirty_count_for_path(Path::new(path)) as usize;
        self.counts_by_path.insert(key, count);
        count
    }
}

fn calculate_dirty_count_for_path(path: &Path) -> u32 {
    #[cfg(test)]
    DIRTY_WORKTREE_SCAN_COUNT.with(|count| count.set(count.get() + 1));
    super::worktree::get_dirty_count_for_path(path)
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

/// メイン workdir とリンク worktree を走査し、`branch 名 → workdir パス` のマップと
/// メイン workdir パスを返す。
fn build_worktree_map(repo: &Repository) -> (HashMap<String, String>, Option<String>) {
    let mut wt_map: HashMap<String, String> = HashMap::new();

    let main_branch = get_branch_name_for_repo(repo);
    let main_workdir = repo
        .workdir()
        .and_then(|p| p.to_str())
        .map(|s| s.trim_end_matches('/').to_string());
    if let Some(ref workdir) = main_workdir {
        wt_map.insert(main_branch, workdir.clone());
    }

    if let Ok(wt_names) = repo.worktrees() {
        for (_, wt) in each_worktree(repo, &wt_names) {
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

    (wt_map, main_workdir)
}

/// is_merged 判定の基準 OID を解決する: `releash.base`（設定）→ 既定ブランチ（fallback）。
fn resolve_base_target_oid(
    repo: &Repository,
    config: Option<&git2::Config>,
    default_oid: Option<Oid>,
) -> Option<Oid> {
    config
        .and_then(|cfg| cfg.get_string("releash.base").ok())
        .and_then(|base_name| repo.find_branch(&base_name, BranchType::Local).ok())
        .and_then(|b| b.get().target())
        .or(default_oid)
}

/// ローカルブランチ 1 件分のカードを構築する（worktree マッチ・dirty・is_merged・
/// ahead/behind・base_ahead・main_worktree 判定）。
struct BranchCardBuildContext<'a> {
    repo: &'a Repository,
    wt_map: &'a HashMap<String, String>,
    config: Option<&'a git2::Config>,
    base_target_oid: Option<Oid>,
    main_workdir: Option<&'a str>,
}

fn build_branch_card(
    branch: &git2::Branch,
    name: String,
    context: &BranchCardBuildContext,
    dirty_counts: &mut DirtyCountSnapshot,
) -> BranchCardDto {
    let (worktree_path, dirty_count) = match context.wt_map.get(&name) {
        Some(path) => {
            let dirty = dirty_counts.dirty_count_for_path(path);
            (Some(path.clone()), dirty)
        }
        None => (None, 0),
    };

    let is_merged = branch
        .get()
        .target()
        .map(|oid| compute_is_merged(context.repo, oid, context.base_target_oid))
        .unwrap_or(false);

    let upstream = branch.upstream().ok();
    let has_upstream = upstream.is_some();
    let (ahead, behind) = upstream
        .and_then(|u| {
            let local_oid = branch.get().target()?;
            let remote_oid = u.get().target()?;
            context.repo.graph_ahead_behind(local_oid, remote_oid).ok()
        })
        .unwrap_or((0, 0));

    let base_ahead = branch
        .get()
        .target()
        .and_then(|branch_oid| {
            let base_name = resolve_branch_base(context.repo, context.config, &name)?;
            let base_oid = context
                .repo
                .find_branch(&base_name, BranchType::Local)
                .ok()?
                .get()
                .target()?;
            context
                .repo
                .graph_ahead_behind(branch_oid, base_oid)
                .ok()
                .map(|(a, _)| a)
        })
        .unwrap_or(0);

    let is_main_wt = worktree_path.as_deref() == context.main_workdir;
    BranchCardDto {
        name,
        is_main_worktree: is_main_wt,
        worktree_path,
        dirty_count,
        is_merged,
        ahead,
        behind,
        has_upstream,
        base_ahead,
    }
}

/// ローカルブランチにマッチしなかった worktree（detached HEAD 等）のカードを構築する。
fn build_unmatched_worktree_card(
    wt_branch_name: &str,
    wt_path: &str,
    main_workdir: Option<&str>,
    dirty_counts: &mut DirtyCountSnapshot,
) -> BranchCardDto {
    let dirty_count = dirty_counts.dirty_count_for_path(wt_path);
    let is_main_wt = main_workdir == Some(wt_path);
    BranchCardDto {
        name: wt_branch_name.to_string(),
        is_main_worktree: is_main_wt,
        worktree_path: Some(wt_path.to_string()),
        dirty_count,
        is_merged: false,
        ahead: 0,
        behind: 0,
        has_upstream: false,
        base_ahead: 0,
    }
}

pub(crate) fn list_branches_with_status(
    repo_path: &str,
) -> Result<Vec<BranchCardDto>, RepositoryError> {
    let repo = client::open(repo_path)?;
    list_branches_with_status_for_repo(&repo, DirtyCountSnapshot::empty())
}

pub(crate) fn list_branches_with_status_for_scan(
    repo_path: &str,
    current_dirty_count: usize,
) -> Result<Vec<BranchCardDto>, RepositoryError> {
    let repo = client::open(repo_path)?;
    let current_workdir = repo
        .workdir()
        .and_then(|path| path.to_str())
        .map(normalize_worktree_path);
    list_branches_with_status_for_repo(
        &repo,
        DirtyCountSnapshot::with_current(current_workdir.as_deref(), current_dirty_count),
    )
}

fn list_branches_with_status_for_repo(
    repo: &Repository,
    mut dirty_counts: DirtyCountSnapshot,
) -> Result<Vec<BranchCardDto>, RepositoryError> {
    let default_branch = detect_default_branch(repo);

    let default_oid = default_branch.as_ref().and_then(|name| {
        repo.find_branch(name, BranchType::Local)
            .ok()?
            .get()
            .target()
    });

    let (wt_map, main_workdir) = build_worktree_map(repo);

    let local_branches = repo.branches(Some(BranchType::Local))?;
    let config = repo.config().ok();
    let base_target_oid = resolve_base_target_oid(repo, config.as_ref(), default_oid);
    let context = BranchCardBuildContext {
        repo,
        wt_map: &wt_map,
        config: config.as_ref(),
        base_target_oid,
        main_workdir: main_workdir.as_deref(),
    };

    let mut cards = Vec::new();
    let mut matched_wt_keys: HashSet<String> = HashSet::new();
    for branch in local_branches {
        let (branch, _) = branch?;
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue,
        };

        if wt_map.contains_key(&name) {
            matched_wt_keys.insert(name.clone());
        }

        cards.push(build_branch_card(
            &branch,
            name,
            &context,
            &mut dirty_counts,
        ));
    }

    // wt_map に残ったエントリ（ローカルブランチにマッチしなかったワークツリー）を追加
    // detached HEAD（rebase中等）のワークツリーがここに該当する
    for (wt_branch_name, wt_path) in &wt_map {
        if matched_wt_keys.contains(wt_branch_name) {
            continue;
        }
        cards.push(build_unmatched_worktree_card(
            wt_branch_name,
            wt_path,
            main_workdir.as_deref(),
            &mut dirty_counts,
        ));
    }

    Ok(cards)
}

/// `BranchCardQuery`（Query 側）の git2 実装。表示用 read model をデータソースから直接構築する。
pub struct BranchCardGateway;

impl BranchCardQuery for BranchCardGateway {
    fn list_branch_cards(&self, repo_path: &str) -> Result<Vec<BranchCardDto>, RepositoryError> {
        list_branches_with_status(repo_path)
    }

    fn list_branch_cards_for_scan(
        &self,
        repo_path: &str,
        current_dirty_count: usize,
    ) -> Result<Vec<BranchCardDto>, RepositoryError> {
        list_branches_with_status_for_scan(repo_path, current_dirty_count)
    }
}

#[cfg(test)]
mod branch_card_gateway_tests {
    use super::*;
    use crate::adaptor::gateway::repository::git_config::set_releash_base;
    use crate::test_support::git::*;
    use git2::build::CheckoutBuilder;
    use git2::WorktreeAddOptions;
    use std::fs;
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

    fn s(p: &Path) -> String {
        p.to_str().unwrap().to_string()
    }

    fn reset_dirty_scan_count() {
        DIRTY_WORKTREE_SCAN_COUNT.with(|count| count.set(0));
    }

    fn dirty_scan_count() -> usize {
        DIRTY_WORKTREE_SCAN_COUNT.with(|count| count.get())
    }

    #[test]
    fn test_ブランチカード一覧_既定を含む() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let cards = list_branches_with_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(cards.len(), 1);
        assert!(cards[0].is_main_worktree);
    }

    #[test]
    fn test_ブランチカード一覧_非既定表示() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-a", &head, false).unwrap();
        repo.branch("feature-b", &head, false).unwrap();

        let cards = list_branches_with_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(cards.len(), 3);
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"feature-a"));
        assert!(names.contains(&"feature-b"));
        for card in cards.iter().filter(|c| !c.is_main_worktree) {
            assert!(card.worktree_path.is_none());
            assert_eq!(card.dirty_count, 0);
        }
    }

    #[test]
    fn test_ブランチカード一覧_worktree付き() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-feat", "feat-wt");
        fs::write(wt_path.join("dirty.txt"), "dirty").unwrap();

        let cards = list_branches_with_status(repo_dir.to_str().unwrap()).unwrap();
        let feat_card = cards.iter().find(|c| c.name == "feat-wt").unwrap();
        assert!(feat_card.worktree_path.is_some());
        assert_eq!(feat_card.dirty_count, 1);
    }

    #[test]
    fn list_branches_for_scan_reuses_current_dirty_and_scans_each_linked_worktree_once() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        fs::write(repo_dir.join("main-dirty.txt"), "dirty").unwrap();
        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-feat", "feat-wt");
        fs::write(wt_path.join("linked-dirty.txt"), "dirty").unwrap();

        reset_dirty_scan_count();
        let cards = list_branches_with_status_for_scan(repo_dir.to_str().unwrap(), 1).unwrap();

        let main_card = cards.iter().find(|card| card.is_main_worktree).unwrap();
        let linked_card = cards.iter().find(|card| card.name == "feat-wt").unwrap();
        assert_eq!(main_card.dirty_count, 1);
        assert_eq!(linked_card.dirty_count, 1);
        assert_eq!(dirty_scan_count(), 1);
    }

    #[test]
    fn test_ブランチカード一覧_behindは未マージ() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-behind", &head, false).unwrap();

        add_and_commit(&repo, "after.txt", "after", "commit after branch");

        let cards = list_branches_with_status(dir.path().to_str().unwrap()).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-behind").unwrap();
        assert!(
            !card.is_merged,
            "branch with no unique commits should not be merged when base advances"
        );
    }

    #[test]
    fn test_ブランチカード一覧_マージコミット経由() {
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
        repo.set_head(&format!("refs/heads/{default_branch}"))
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

        let cards = list_branches_with_status(dir.path().to_str().unwrap()).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-merged").unwrap();
        assert!(
            card.is_merged,
            "branch merged via merge commit should be detected as merged"
        );
    }

    #[test]
    fn test_ブランチカード一覧_未マージ() {
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
        repo.set_head(&format!("refs/heads/{default_branch}"))
            .unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();

        let cards = list_branches_with_status(dir.path().to_str().unwrap()).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-unmerged").unwrap();
        assert!(!card.is_merged);
    }

    #[test]
    fn test_ブランチカード一覧_同一oidは未マージ() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-same", &head, false).unwrap();

        let cards = list_branches_with_status(dir.path().to_str().unwrap()).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-same").unwrap();
        assert!(!card.is_merged);
    }

    #[test]
    fn test_ブランチカード一覧_releash_base経由マージ() {
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
        repo.set_head(&format!("refs/heads/{default_branch}"))
            .unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();

        let repo_path = s(dir.path());

        let cards = list_branches_with_status(&repo_path).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-x").unwrap();
        assert!(
            !card.is_merged,
            "without releash.base, should not be merged into main"
        );

        set_releash_base(&repo_path, Some("develop")).unwrap();

        let cards = list_branches_with_status(&repo_path).unwrap();
        let card = cards.iter().find(|c| c.name == "feature-x").unwrap();
        assert!(
            card.is_merged,
            "with releash.base=develop, should be merged"
        );
    }

    #[test]
    fn test_ブランチカード一覧_ahead_behind() {
        let (_parent, clone_dir, repo) = setup_remote_repo();

        add_and_commit(&repo, "local.txt", "local", "local commit");

        let cards = list_branches_with_status(clone_dir.to_str().unwrap()).unwrap();
        let card = cards.iter().find(|c| c.name == "main").unwrap();
        assert_eq!(card.ahead, 1);
        assert_eq!(card.behind, 0);
    }

    #[test]
    fn test_ブランチカード一覧_upstreamなし() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("no-upstream", &head, false).unwrap();

        let cards = list_branches_with_status(dir.path().to_str().unwrap()).unwrap();
        let card = cards.iter().find(|c| c.name == "no-upstream").unwrap();
        assert_eq!(card.ahead, 0);
        assert_eq!(card.behind, 0);
    }

    #[test]
    fn test_ブランチカード一覧_detached_head_worktree() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let wt_path = create_worktree_helper(&repo, _parent.path(), "wt-rebase", "feat-rebase");

        let wt_repo = Repository::open(&wt_path).unwrap();
        let head_commit = wt_repo.head().unwrap().peel_to_commit().unwrap();
        wt_repo.set_head_detached(head_commit.id()).unwrap();

        let cards = list_branches_with_status(repo_dir.to_str().unwrap()).unwrap();

        let detached_card = cards.iter().find(|c| {
            c.name != "feat-rebase"
                && c.name != "master"
                && c.worktree_path.is_some()
                && c.worktree_path.as_deref().unwrap().ends_with("wt-rebase")
        });
        assert!(
            detached_card.is_some(),
            "detached HEAD worktree should be included in list_branches_with_status"
        );

        let card = detached_card.unwrap();
        assert!(card.worktree_path.is_some());
        assert!(
            card.name.starts_with('('),
            "detached HEAD card name should start with '(', got: {}",
            card.name
        );
    }

    #[test]
    fn test_main_worktree判定_既定ブランチ上() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        create_worktree_helper(&repo, _parent.path(), "wt-feat", "feat-a");

        let cards = list_branches_with_status(repo_dir.to_str().unwrap()).unwrap();

        let main_card = cards.iter().find(|c| c.name == "master").unwrap();
        assert!(main_card.is_main_worktree);

        let wt_card = cards.iter().find(|c| c.name == "feat-a").unwrap();
        assert!(!wt_card.is_main_worktree);
    }

    #[test]
    fn test_main_worktree判定_featureブランチ上() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feat-current", &head, false).unwrap();
        repo.set_head("refs/heads/feat-current").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();

        let cards = list_branches_with_status(repo_dir.to_str().unwrap()).unwrap();

        let feat_card = cards.iter().find(|c| c.name == "feat-current").unwrap();
        assert!(feat_card.is_main_worktree);

        let master_card = cards.iter().find(|c| c.name == "master").unwrap();
        assert!(!master_card.is_main_worktree);
    }

    #[test]
    fn test_main_worktree判定_detached_headメインリポジトリ() {
        let (_parent, repo_dir, repo) = create_test_repo_with_parent();
        create_initial_commit(&repo);

        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.set_head_detached(head_commit.id()).unwrap();

        let cards = list_branches_with_status(repo_dir.to_str().unwrap()).unwrap();

        let detached_card = cards.iter().find(|c| c.name.starts_with('(')).unwrap();
        assert!(detached_card.is_main_worktree);
    }
}
