use super::{
    facts,
    repository_impl::{repository_group_key, storage_key},
};
use crate::adaptor::gateway::local_event_store::{
    workspace_state_codec::StoredWorkspaceState, LocalEventStore,
};
use crate::domain::local_event::{LocalDomainEvent, StreamId};
use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateError};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) async fn migrate(
    app_data_dir: &Path,
    store: &Arc<LocalEventStore>,
    repositories: &[String],
    worktrees: &[String],
) -> Result<(), WorkspaceStateError> {
    let worktree_result = migrate_worktrees(app_data_dir, store, worktrees).await;
    let group_result = migrate_groups(app_data_dir, store, repositories).await;
    worktree_result.and(group_result)
}
async fn migrate_worktrees(
    app_data_dir: &Path,
    store: &Arc<LocalEventStore>,
    worktrees: &[String],
) -> Result<(), WorkspaceStateError> {
    let directory = app_data_dir.join("workspace_state");
    let candidates = worktrees
        .iter()
        .map(|path| (path.clone(), storage_key(path)))
        .collect::<Vec<_>>();
    let mut files = files(&directory)?;
    files.sort_by_key(|file| {
        !file.file_stem().is_some_and(|stem| {
            candidates
                .iter()
                .any(|(_, key)| key == &stem.to_string_lossy())
        })
    });
    let mut failure = None;
    for file in files {
        let Some(name) = file.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(key) =
            crate::domain::workspace_state::services::legacy_state_path(name, &candidates)
        else {
            continue;
        };
        let result = async {
            let stream = facts::workspace_stream(key);
            let (head, event) = facts::latest(store, stream.clone())
                .await
                .map_err(WorkspaceStateError::from)?;
            if event.is_none() {
                let stored: StoredWorkspaceState = serde_json::from_slice(
                    &std::fs::read(&file).map_err(WorkspaceStateError::from)?,
                )
                .map_err(WorkspaceStateError::from)?;
                let state: WorkspaceState = stored.try_into()?;
                facts::append(
                    store,
                    stream,
                    LocalDomainEvent::WorkspaceStateChanged(Box::new(state)),
                    head,
                )
                .await?;
            }
            move_migrated(&directory, &file)
        }
        .await;
        retain_failure(&file, result, &mut failure);
    }
    finish_directory(&directory)?;
    failure.map_or(Ok(()), Err)
}
async fn migrate_groups(
    app_data_dir: &Path,
    store: &Arc<LocalEventStore>,
    repositories: &[String],
) -> Result<(), WorkspaceStateError> {
    let directory = app_data_dir.join("repository_group_state");
    let mut failure = None;
    for file in files(&directory)? {
        let Some(name) = file.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(key) = repositories
            .iter()
            .find(|path| repository_group_key(path) == name)
        else {
            continue;
        };
        let result = import_group(store, facts::repository_group_stream(key), &file)
            .await
            .and_then(|()| move_migrated(&directory, &file));
        retain_failure(&file, result, &mut failure);
    }
    finish_directory(&directory)?;
    failure.map_or(Ok(()), Err)
}
async fn import_group(
    store: &Arc<LocalEventStore>,
    stream: StreamId,
    file: &Path,
) -> Result<(), WorkspaceStateError> {
    let (head, event) = facts::latest(store, stream.clone())
        .await
        .map_err(WorkspaceStateError::from)?;
    if event.is_none() {
        let collapsed =
            serde_json::from_slice(&std::fs::read(file).map_err(WorkspaceStateError::from)?)
                .map_err(WorkspaceStateError::from)?;
        facts::append(
            store,
            stream,
            LocalDomainEvent::RepositoryGroupChanged(collapsed),
            head,
        )
        .await?;
    }
    Ok(())
}
fn files(directory: &Path) -> Result<Vec<PathBuf>, WorkspaceStateError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(WorkspaceStateError::from(error)),
    };
    Ok(entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(WorkspaceStateError::from)?
        .into_iter()
        .filter(|file| file.extension().is_some_and(|ext| ext == "json"))
        .collect())
}
fn move_migrated(directory: &Path, file: &Path) -> Result<(), WorkspaceStateError> {
    let migrated = directory.with_extension("migrated");
    std::fs::create_dir_all(&migrated).map_err(WorkspaceStateError::from)?;
    let target = migrated.join(file.file_name().expect("directory entry"));
    if target.try_exists().map_err(WorkspaceStateError::from)? {
        std::fs::remove_file(file).map_err(WorkspaceStateError::from)
    } else {
        std::fs::rename(file, target).map_err(WorkspaceStateError::from)
    }
}
fn finish_directory(directory: &Path) -> Result<(), WorkspaceStateError> {
    match std::fs::read_dir(directory) {
        Ok(mut entries) => {
            if entries.next().is_none() {
                std::fs::remove_dir(directory).map_err(WorkspaceStateError::from)
            } else {
                Ok(())
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WorkspaceStateError::from(error)),
    }
}
fn retain_failure(
    file: &Path,
    result: Result<(), WorkspaceStateError>,
    failure: &mut Option<WorkspaceStateError>,
) {
    if let Err(error) = result {
        log::warn!("UI state migration retained {}: {error}", file.display());
        *failure = Some(error);
    }
}
