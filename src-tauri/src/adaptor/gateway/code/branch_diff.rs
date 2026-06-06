//! branch diff 責務の gateway 実装。git2 による merge-base diff を封じ込め、
//! read model（`BranchDiffSummaryDto`）を直接組み立てる。
//!
//! ベースブランチ名の解決ルール（`branch.<name>.releash-base` → `releash.base` →
//! `detect_default_branch()`）は `repository` ドメインが所有する。本モジュールは
//! usecase の orchestration（`CodeQueryService` が `BranchBaseResolver` 越しに
//! repository へ委譲）で解決済みの base 名を受け取り、merge-base diff の計算のみを担う。

use git2::Repository;

use crate::domain::code::CodeError;
use crate::usecase::code_dto::{BranchDiffSummaryDto, ChangedFileDto, DiffStatsDto};
use crate::usecase::code_query_service::BranchDiffQuery;

/// リポジトリにまだコミットが無い（HEAD が unborn branch を指す）場合に true。
fn is_unborn_branch(repo: &Repository) -> Result<bool, CodeError> {
    match repo.head() {
        Ok(_) => Ok(false),
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(true),
        Err(e) => Err(CodeError::from(e)),
    }
}

/// 現在ブランチの merge-base と作業ツリー（staged・untracked 含む）の差分サマリ。
///
/// unborn branch の場合は空サマリを返し、呼び出し側が一貫した空状態を表示できるようにする。
pub(crate) fn get_branch_diff_summary(
    repo_path: &str,
    base_name: Option<&str>,
    base_commit_oid: Option<&str>,
) -> Result<BranchDiffSummaryDto, CodeError> {
    let repo = Repository::open(repo_path)?;
    if is_unborn_branch(&repo)? {
        return Ok(BranchDiffSummaryDto {
            base_branch: String::new(),
            changed_files: Vec::new(),
            stats: DiffStatsDto {
                additions: 0,
                deletions: 0,
            },
        });
    }
    // base 名タグ（表示用）は usecase が解決済みの名前から。merge-base コミットは
    // 解決済み base OID から計算する（`None` は detached / 未設定で HEAD フォールバック）。
    let base_ref = base_name.unwrap_or("HEAD").to_string();
    let base_commit = super::resolve_merge_base_commit(&repo, base_commit_oid)?;
    let base_tree = base_commit.tree()?;

    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true);
    opts.recurse_untracked_dirs(true);
    opts.show_untracked_content(true);

    let diff = repo.diff_tree_to_workdir_with_index(Some(&base_tree), Some(&mut opts))?;
    let mut find_opts = git2::DiffFindOptions::new();
    find_opts.renames(true).copies(true);
    let mut diff = diff;
    diff.find_similar(Some(&mut find_opts))?;

    let stats = diff.stats()?;
    let total_additions = stats.insertions() as u32;
    let total_deletions = stats.deletions() as u32;

    let num_deltas = diff.deltas().len();
    let mut changed_files: Vec<ChangedFileDto> = Vec::with_capacity(num_deltas);

    for i in 0..num_deltas {
        let delta = diff
            .get_delta(i)
            .ok_or_else(|| CodeError::Rule(format!("invalid delta index: {i}")))?;

        let new_path = delta
            .new_file()
            .path()
            .map(|p| p.to_string_lossy().to_string());
        let old_path = delta
            .old_file()
            .path()
            .map(|p| p.to_string_lossy().to_string());
        let path = new_path
            .clone()
            .or_else(|| old_path.clone())
            .ok_or_else(|| CodeError::Rule(format!("delta {i} has no file path")))?;

        let status = match delta.status() {
            git2::Delta::Added | git2::Delta::Untracked => "added",
            git2::Delta::Deleted => "deleted",
            git2::Delta::Modified => "modified",
            git2::Delta::Renamed => "renamed",
            git2::Delta::Copied => "copied",
            git2::Delta::Typechange => "modified",
            _ => "modified",
        };

        let binary = delta.new_file().is_binary() || delta.old_file().is_binary();

        let (additions, deletions) = if binary {
            (0u32, 0u32)
        } else if let Some(patch) = git2::Patch::from_diff(&diff, i)? {
            let mut adds = 0u32;
            let mut dels = 0u32;
            for h in 0..patch.num_hunks() {
                let lines = patch.num_lines_in_hunk(h)?;
                for l in 0..lines {
                    let line = patch.line_in_hunk(h, l)?;
                    match line.origin() {
                        '+' => adds += 1,
                        '-' => dels += 1,
                        _ => {}
                    }
                }
            }
            (adds, dels)
        } else {
            (0u32, 0u32)
        };

        let old_path_opt = match status {
            "renamed" | "copied" => old_path,
            _ => None,
        };

        changed_files.push(ChangedFileDto {
            path,
            old_path: old_path_opt,
            status: status.to_string(),
            binary,
            stats: DiffStatsDto {
                additions,
                deletions,
            },
        });
    }

    Ok(BranchDiffSummaryDto {
        base_branch: base_ref,
        changed_files,
        stats: DiffStatsDto {
            additions: total_additions,
            deletions: total_deletions,
        },
    })
}

/// `BranchDiffQuery` の git2 実装。
pub struct BranchDiffGateway;

impl BranchDiffQuery for BranchDiffGateway {
    fn summary(
        &self,
        repo_path: &str,
        base_name: Option<&str>,
        base_commit_oid: Option<&str>,
    ) -> Result<BranchDiffSummaryDto, CodeError> {
        get_branch_diff_summary(repo_path, base_name, base_commit_oid)
    }
}

#[cfg(test)]
mod branch_diff_gateway_tests {
    use super::*;
    use crate::git::test_helpers::*;
    use git2::build::CheckoutBuilder;

    fn repo_path_str(repo: &Repository) -> String {
        repo.workdir().unwrap().to_str().unwrap().to_string()
    }

    /// feature ブランチを作成・チェックアウトし、分岐元（base）の (ブランチ名, コミット OID)
    /// を返す。base 名解決と ref→OID 解決は repository ドメインの責務に移ったため、gateway の
    /// merge-base 計算テストは解決済みの base 名タグと base コミット OID を明示的に渡す。
    fn setup_feature_branch(repo: &Repository) -> (String, String) {
        let base = repo.head().unwrap().shorthand().unwrap().to_string();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let base_oid = head.id().to_string();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        (base, base_oid)
    }

    #[test]
    fn test_branch_diff_単一変更ファイル() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        let (base, base_oid) = setup_feature_branch(&repo);
        add_and_commit(
            &repo,
            "file.txt",
            "line1\nmodified\nline3\n",
            "modify line2",
        );

        let summary =
            get_branch_diff_summary(&repo_path_str(&repo), Some(&base), Some(&base_oid)).unwrap();
        assert_eq!(summary.changed_files.len(), 1);
        assert_eq!(summary.changed_files[0].path, "file.txt");
        assert_eq!(summary.changed_files[0].status, "modified");
        assert!(!summary.changed_files[0].binary);
    }

    #[test]
    fn test_branch_diff_追加ファイル() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "existing.txt", "content\n", "add existing");

        let (base, base_oid) = setup_feature_branch(&repo);
        add_and_commit(&repo, "new.txt", "new\n", "add new file");

        let summary =
            get_branch_diff_summary(&repo_path_str(&repo), Some(&base), Some(&base_oid)).unwrap();
        let added = summary
            .changed_files
            .iter()
            .find(|f| f.path == "new.txt")
            .expect("new.txt should be in diff");
        assert_eq!(added.status, "added");
    }

    #[test]
    fn test_branch_diff_変更なし() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content\n", "add file");

        let (base, base_oid) = setup_feature_branch(&repo);
        let summary =
            get_branch_diff_summary(&repo_path_str(&repo), Some(&base), Some(&base_oid)).unwrap();
        assert!(summary.changed_files.is_empty());
        assert_eq!(summary.stats.additions, 0);
        assert_eq!(summary.stats.deletions, 0);
    }

    #[test]
    fn test_branch_diff_未追跡ファイルを含む() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "existing.txt", "content\n", "add existing");

        let (base, base_oid) = setup_feature_branch(&repo);
        let workdir = repo.workdir().unwrap();
        std::fs::write(workdir.join("untracked.txt"), "hello\n").unwrap();

        let summary =
            get_branch_diff_summary(&repo_path_str(&repo), Some(&base), Some(&base_oid)).unwrap();
        let untracked = summary
            .changed_files
            .iter()
            .find(|f| f.path == "untracked.txt")
            .expect("untracked.txt should be included in the branch diff");
        assert_eq!(untracked.status, "added");
    }

    #[test]
    fn test_branch_diff_unborn_branchは空() {
        let (_dir, repo) = create_test_repo();
        let summary = get_branch_diff_summary(&repo_path_str(&repo), None, None).unwrap();
        assert!(summary.changed_files.is_empty());
        assert_eq!(summary.stats.additions, 0);
        assert_eq!(summary.stats.deletions, 0);
        assert_eq!(summary.base_branch, "");
    }

    #[test]
    fn test_branch_diff_base未指定_headフォールバック() {
        // base 名が None（detached / base 未設定）の場合、merge-base ではなく HEAD と
        // workdir の差分になる。ここではコミット済みのため変更なし、untracked のみ検出される。
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content\n", "add file");
        let workdir = repo.workdir().unwrap();
        std::fs::write(workdir.join("untracked.txt"), "hello\n").unwrap();

        let summary = get_branch_diff_summary(&repo_path_str(&repo), None, None).unwrap();
        assert_eq!(summary.base_branch, "HEAD");
        assert!(summary
            .changed_files
            .iter()
            .any(|f| f.path == "untracked.txt"));
        assert!(!summary.changed_files.iter().any(|f| f.path == "file.txt"));
    }
}
