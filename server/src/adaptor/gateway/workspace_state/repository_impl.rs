use super::facts;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::adaptor::gateway::shared::workspace_state::unexpected;
use crate::domain::local_event::{LocalDomainEvent, StreamId, StreamVersion};
use crate::domain::workspace_state::services::filter_missing_files;
use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateKey, WorkspaceStateRepository,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct WorkspaceStateStore {
    app_data_dir: PathBuf,
    store: Arc<LocalEventStore>,
}
impl WorkspaceStateStore {
    pub fn with_store(app_data_dir: PathBuf, store: Arc<LocalEventStore>) -> Self {
        Self {
            app_data_dir,
            store,
        }
    }
}
#[cfg(any(test, feature = "test-support"))]
fn state_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("workspace_state")
}
pub(crate) const ABSOLUTE_STATE_KEY_PREFIX: &str = "worktree-";
pub(crate) fn state_key_from_hash(hash: &str) -> WorkspaceStateKey {
    WorkspaceStateKey::from_storage(format!("{ABSOLUTE_STATE_KEY_PREFIX}{hash}"))
}
pub fn storage_key(key: &str) -> String {
    if Path::new(key).is_absolute() {
        use sha2::Digest;
        state_key_from_hash(&hex::encode(sha2::Sha256::digest(key.as_bytes())))
            .storage_value()
            .to_owned()
    } else {
        key.replace(['/', '\\'], "_")
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn state_file(app_data_dir: &Path, key: &str) -> PathBuf {
    state_dir(app_data_dir).join(format!("{}.json", storage_key(key)))
}
pub(super) fn repository_group_key(path: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(
        crate::domain::repository::normalize_repo_path(path).as_bytes(),
    ))
}

#[async_trait::async_trait]
impl WorkspaceStateRepository for WorkspaceStateStore {
    async fn migrate_legacy(
        &self,
        repositories: &[String],
        worktrees: &[String],
    ) -> Result<(), WorkspaceStateError> {
        super::legacy_import::migrate(&self.app_data_dir, &self.store, repositories, worktrees)
            .await
    }
    async fn load_repository_group(&self, path: &str) -> Result<bool, WorkspaceStateError> {
        Ok(self.read_repository_group(path).await?.1)
    }
    async fn read_repository_group(
        &self,
        path: &str,
    ) -> Result<(StreamVersion, bool), WorkspaceStateError> {
        let (head, event) = facts::latest(&self.store, facts::repository_group_stream(path))
            .await
            .map_err(WorkspaceStateError::from)?;
        let collapsed = match event {
            Some(LocalDomainEvent::RepositoryGroupChanged(collapsed)) => collapsed,
            None => false,
            _ => return Err(unexpected("Unexpected repository group event")),
        };
        Ok((head, collapsed))
    }
    async fn save_repository_group(
        &self,
        path: &str,
        collapsed: bool,
        expected: StreamVersion,
    ) -> Result<(), WorkspaceStateError> {
        let stream = facts::repository_group_stream(path);
        facts::append(
            &self.store,
            stream,
            LocalDomainEvent::RepositoryGroupChanged(collapsed),
            expected,
        )
        .await
    }

    async fn filter_missing_files(
        &self,
        state: WorkspaceState,
        root: &str,
    ) -> Result<WorkspaceState, WorkspaceStateError> {
        let root = root.to_owned();
        crate::common::operation_context::spawn_blocking(move || {
            filter_missing_files(state, |path| {
                let path = Path::new(path);
                if path.is_absolute() {
                    path.exists()
                } else {
                    Path::new(&root).join(path).exists()
                }
            })
        })
        .await
        .map_err(WorkspaceStateError::from)
    }
    async fn read(
        &self,
        key: &str,
    ) -> Result<(StreamVersion, Option<WorkspaceState>), WorkspaceStateError> {
        let (head, event) = facts::latest(&self.store, facts::workspace_stream(key))
            .await
            .map_err(WorkspaceStateError::from)?;
        let state = match event {
            Some(LocalDomainEvent::WorkspaceStateChanged(state)) => Some(*state),
            None | Some(LocalDomainEvent::WorkspaceStateInvalidated) => None,
            _ => return Err(unexpected("Unexpected workspace state event")),
        };
        Ok((head, state))
    }
    async fn save(
        &self,
        key: &str,
        state: WorkspaceState,
        expected: StreamVersion,
    ) -> Result<(), WorkspaceStateError> {
        facts::append(
            &self.store,
            facts::workspace_stream(key),
            LocalDomainEvent::WorkspaceStateChanged(Box::new(state)),
            expected,
        )
        .await
    }
    async fn states(&self) -> Result<Vec<(WorkspaceStateKey, StreamVersion)>, WorkspaceStateError> {
        use crate::domain::local_event::{
            LocalEventQuery, LocalEventQueryResult, LocalEventTransactionRepository,
        };
        let result = self.store.query(LocalEventQuery::StreamHeads {
            namespace: crate::domain::local_event::identifiers::WORKSPACE_STATE_PREFIX.into(),
            event_type: crate::adaptor::gateway::local_event_store::workspace_state_codec::WORKSPACE_STATE_CHANGED.into(),
        }).await.map_err(WorkspaceStateError::from)?;
        let LocalEventQueryResult::StreamHeads(heads) = result else {
            return Err(unexpected("Unexpected stream heads result"));
        };
        heads
            .into_iter()
            .map(|(stream, version)| {
                let hash = stream
                    .as_str()
                    .strip_prefix(crate::domain::local_event::identifiers::WORKSPACE_STATE_PREFIX)
                    .ok_or_else(|| unexpected("Unexpected workspace state stream"))?;
                Ok((state_key_from_hash(hash), version))
            })
            .collect()
    }
    async fn invalidate(
        &self,
        key: &WorkspaceStateKey,
        expected: StreamVersion,
    ) -> Result<bool, WorkspaceStateError> {
        let hash = key
            .storage_value()
            .strip_prefix(ABSOLUTE_STATE_KEY_PREFIX)
            .ok_or_else(|| unexpected("Unexpected workspace state key"))?;
        let stream = StreamId::parse(&format!(
            "{}{hash}",
            crate::domain::local_event::identifiers::WORKSPACE_STATE_PREFIX
        ))
        .map_err(unexpected)?;
        let (head, event) = facts::latest(&self.store, stream.clone())
            .await
            .map_err(WorkspaceStateError::from)?;
        if head != expected || !matches!(event, Some(LocalDomainEvent::WorkspaceStateChanged(_))) {
            return Ok(false);
        }
        facts::append(
            &self.store,
            stream,
            LocalDomainEvent::WorkspaceStateInvalidated,
            head,
        )
        .await?;
        Ok(true)
    }
}
