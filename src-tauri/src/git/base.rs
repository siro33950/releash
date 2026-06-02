//! code ドメイン（diff / branch_diff）向けのベースブランチ解決。
//!
//! 「`branch.<name>.releash-base` → `releash.base` → `detect_default_branch()`」という
//! 解決順序（業務ルール）は infrastructure の責務外。code ドメインは adaptor/gateway へ
//! 依存できない（逆方向依存禁止）ため、repository gateway 側
//! （`adaptor::gateway::repository::util::resolve_branch_base`）とは独立に、最下層の
//! infrastructure プリミティブ（`detect_default_branch`）を組み合わせてここで構成する。

use git2::Repository;

use crate::infrastructure::git::helpers::detect_default_branch;

/// ベースブランチ名をフォールバックチェーンで解決する。
/// `branch.<name>.releash-base` → `releash.base` → `detect_default_branch()`。
pub(crate) fn resolve_branch_base(
    repo: &Repository,
    config: Option<&git2::Config>,
    branch_name: &str,
) -> Option<String> {
    if let Some(cfg) = config {
        if let Ok(base) = cfg.get_string(&format!("branch.{branch_name}.releash-base")) {
            return Some(base);
        }
        if let Ok(base) = cfg.get_string("releash.base") {
            return Some(base);
        }
    }
    detect_default_branch(repo)
}
