pub mod entities;
pub mod error;
// `docs/architecture/DOMAIN.md` 規約に従い永続化／外部リソース抽象は
// `repository.rs` に置く（module_inception はこの規約名を優先して許容）。
#[allow(clippy::module_inception)]
pub mod repository;
pub mod value_objects;

pub use entities::{Branch, Commit, FileDiffStat, FileStatus, RepositoryStatusScan, Worktree};
pub use error::RepositoryError;
pub use repository::{
    BranchRepository, GitConfigRepository, LogRepository, RepoLocator, RepoPathsNotifier,
    RepoPathsRepository, StatusRepository, WorktreeRepository,
};
pub use value_objects::normalize_repo_path;
