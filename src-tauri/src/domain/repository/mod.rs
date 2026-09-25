pub mod entities;
pub mod error;
pub mod gateway;
// `docs/architecture/DOMAIN.md` 規約に従い永続化抽象は `repository.rs` に
// 置く（module_inception はこの規約名を優先して許容）。
#[allow(clippy::module_inception)]
pub mod repository;
pub mod value_objects;

pub use entities::{Branch, FileDiffStat, FileStatus, RepositoryStatusScan, Worktree};
pub use error::RepositoryError;
pub use gateway::{RepoPathsNotifier, WorktreeTerminalGateway};
pub use repository::{
    BranchRepository, GitConfigRepository, RepoLocator, RepoPathsRepository, StatusRepository,
    WorktreeRepository,
};
pub use value_objects::{normalize_repo_path, worktree_dir, worktree_path};

pub(crate) mod file_watcher;

pub(crate) mod watch_subscriptions;

pub(crate) mod worktree_operation;

mod branch_inventory;
pub use branch_inventory::classify_branch_cards;

pub(crate) mod background_failure;
