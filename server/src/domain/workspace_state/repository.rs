use crate::domain::local_event::StreamVersion;
use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateError, WorkspaceStateKey};

#[async_trait::async_trait]
pub trait WorkspaceStateRepository: Send + Sync {
    async fn migrate_legacy(
        &self,
        repositories: &[String],
        worktrees: &[String],
    ) -> Result<(), WorkspaceStateError>;
    async fn load_repository_group(
        &self,
        repository_path: &str,
    ) -> Result<bool, WorkspaceStateError>;
    async fn read_repository_group(
        &self,
        repository_path: &str,
    ) -> Result<(StreamVersion, bool), WorkspaceStateError>;
    async fn save_repository_group(
        &self,
        repository_path: &str,
        collapsed: bool,
        expected: StreamVersion,
    ) -> Result<(), WorkspaceStateError>;
    async fn filter_missing_files(
        &self,
        state: WorkspaceState,
        worktree_root: &str,
    ) -> Result<WorkspaceState, WorkspaceStateError>;
    async fn read(
        &self,
        worktree_name: &str,
    ) -> Result<(StreamVersion, Option<WorkspaceState>), WorkspaceStateError>;
    async fn save(
        &self,
        worktree_name: &str,
        state: WorkspaceState,
        expected: StreamVersion,
    ) -> Result<(), WorkspaceStateError>;
    async fn states(&self) -> Result<Vec<(WorkspaceStateKey, StreamVersion)>, WorkspaceStateError>;
    async fn invalidate(
        &self,
        key: &WorkspaceStateKey,
        expected: StreamVersion,
    ) -> Result<bool, WorkspaceStateError>;
}
