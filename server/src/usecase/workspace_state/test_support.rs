use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::local_event::StreamVersion;
use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateKey, WorkspaceStateRepository,
};

#[derive(Default)]
pub struct MemoryWorkspaceStateRepository {
    states: Mutex<HashMap<String, (StreamVersion, WorkspaceState)>>,
    pub next_save_error: Mutex<Option<WorkspaceStateError>>,
}

#[async_trait::async_trait]
impl WorkspaceStateRepository for MemoryWorkspaceStateRepository {
    async fn read(
        &self,
        workspace: &str,
    ) -> Result<(StreamVersion, Option<WorkspaceState>), WorkspaceStateError> {
        Ok(self.states.lock().get(workspace).map_or_else(
            || (StreamVersion::new(0).unwrap(), None),
            |(version, state)| (*version, Some(state.clone())),
        ))
    }

    async fn save(
        &self,
        workspace: &str,
        state: WorkspaceState,
        expected: StreamVersion,
    ) -> Result<StreamVersion, WorkspaceStateError> {
        if let Some(error) = self.next_save_error.lock().take() {
            return Err(error);
        }
        let mut states = self.states.lock();
        let version = states
            .get(workspace)
            .map_or(0, |(version, _)| version.value());
        if version != expected.value() {
            return Err(WorkspaceStateError::VersionConflict);
        }
        let saved = StreamVersion::new(version + 1).unwrap();
        states.insert(workspace.into(), (saved, state));
        Ok(saved)
    }

    async fn filter_missing_files(
        &self,
        state: WorkspaceState,
        _: &str,
    ) -> Result<WorkspaceState, WorkspaceStateError> {
        Ok(state)
    }

    async fn migrate_legacy(&self, _: &[String], _: &[String]) -> Result<(), WorkspaceStateError> {
        panic!("unexpected migration")
    }

    async fn load_repository_group(&self, _: &str) -> Result<bool, WorkspaceStateError> {
        panic!("unexpected repository group read")
    }

    async fn read_repository_group(
        &self,
        _: &str,
    ) -> Result<(StreamVersion, bool), WorkspaceStateError> {
        panic!("unexpected repository group read")
    }

    async fn save_repository_group(
        &self,
        _: &str,
        _: bool,
        _: StreamVersion,
    ) -> Result<(), WorkspaceStateError> {
        panic!("unexpected repository group write")
    }

    async fn states(&self) -> Result<Vec<(WorkspaceStateKey, StreamVersion)>, WorkspaceStateError> {
        panic!("unexpected state enumeration")
    }

    async fn invalidate(
        &self,
        _: &WorkspaceStateKey,
        _: StreamVersion,
    ) -> Result<bool, WorkspaceStateError> {
        panic!("unexpected state invalidation")
    }
}

pub fn test_session_tabs() -> super::session_tabs::SessionTabsUsecase {
    super::session_tabs::SessionTabsUsecase::new(
        Arc::new(MemoryWorkspaceStateRepository::default()),
        crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
        crate::test_support::state_subscription::test_subscriptions(),
    )
}
