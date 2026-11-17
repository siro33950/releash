pub mod entities;
pub mod error;
pub mod gateway;
pub mod repository;
pub mod service;
pub mod value_objects;

pub use entities::{Branch, ChangeKind, ConflictKind, FileStatus, RepositoryStatusScan, Worktree};
pub use error::GitError;
pub use gateway::WorktreeTerminalGateway;
pub use repository::{RepoLocator, RepoPathsRepository};
pub use service::GitService;
pub use value_objects::{
    normalize_repo_path, repository_names, validate_worktree_branches, worktree_dir, worktree_path,
    BranchState, CommitMessage, ConflictedFile, Continuation, DiscardPlan, DiscardSummary,
    FileChange, GitHubRepository, LineChanges, LineCount, Operation, OperationKind, Push,
    RebaseProgress, RepositoryNameSource, Tracking, WorktreeBranch, WorktreeStatus,
};

pub(crate) mod file_watcher;

pub(crate) mod worktree_operation;
