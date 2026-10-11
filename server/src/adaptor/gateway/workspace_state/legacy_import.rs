use super::{
    facts,
    repository_impl::{repository_group_key, storage_key, StoredWorkspaceState},
};
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateError};
use std::path::Path;
use std::sync::Arc;

pub(super) fn migrate(
    app_data_dir: &Path,
    store: &Arc<LocalEventStore>,
    repositories: &[String],
    worktrees: &[String],
) -> Result<(), WorkspaceStateError> {
    let mut failure = None;
    for (directory, group) in [("workspace_state", false), ("repository_group_state", true)] {
        if let Err(error) = migrate_directory(
            app_data_dir,
            store,
            repositories,
            worktrees,
            directory,
            group,
        ) {
            log::warn!("UI state migration retained legacy data under {directory}: {error}");
            failure = Some(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn migrate_directory(
    app_data_dir: &Path,
    store: &Arc<LocalEventStore>,
    repositories: &[String],
    worktrees: &[String],
    directory: &str,
    group: bool,
) -> Result<(), WorkspaceStateError> {
    let directory = app_data_dir.join(directory);
    let files = match std::fs::read_dir(&directory) {
        Ok(files) => files,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(facts::error(error)),
    };
    let mut files = files
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(facts::error)?;
    files.sort_by_key(|file| {
        !file.file_stem().is_some_and(|stem| {
            worktrees
                .iter()
                .any(|path| storage_key(path) == stem.to_string_lossy())
        })
    });
    let mut failure = None;
    for file in files {
        if file.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(name) = file.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let key = if group {
            repositories
                .iter()
                .find(|path| repository_group_key(path) == name)
                .map(String::as_str)
        } else {
            worktrees
                .iter()
                .find(|path| storage_key(path) == name)
                .map(String::as_str)
                .or_else(|| {
                    crate::domain::workspace_state::services::legacy_worktree_path(name, worktrees)
                })
                .or_else(|| {
                    if worktrees
                        .iter()
                        .any(|path| Path::new(path).file_name().is_some_and(|part| part == name))
                    {
                        return None;
                    }
                    let mut paths = worktrees
                        .iter()
                        .filter(|path| path.replace(['/', '\\'], "_") == name);
                    let first = paths.next()?;
                    paths.next().is_none().then_some(first.as_str())
                })
        };
        let Some(key) = key else {
            continue;
        };
        let result = (|| {
            let stream = facts::stream(key, group);
            if facts::run(facts::latest(store, &stream))?.1.is_none() {
                let bytes = std::fs::read(&file).map_err(facts::error)?;
                let event = if group {
                    LocalDomainEvent::RepositoryGroupChanged(
                        serde_json::from_slice(&bytes).map_err(facts::error)?,
                    )
                } else {
                    let stored: StoredWorkspaceState =
                        serde_json::from_slice(&bytes).map_err(facts::error)?;
                    let state: WorkspaceState = stored.try_into()?;
                    LocalDomainEvent::WorkspaceStateChanged(Box::new(state))
                };
                facts::run(facts::append(store, stream, event))?;
            }
            let migrated = directory.with_extension("migrated");
            std::fs::create_dir_all(&migrated).map_err(facts::error)?;
            let target = migrated.join(file.file_name().expect("directory entry"));
            if !target.try_exists().map_err(facts::error)? {
                std::fs::rename(&file, target).map_err(facts::error)?;
            }
            Ok::<_, WorkspaceStateError>(())
        })();
        if let Err(error) = result {
            log::warn!("UI state migration retained {}: {error}", file.display());
            failure = Some(error);
        }
    }
    if std::fs::read_dir(&directory)
        .map_err(facts::error)?
        .next()
        .is_none()
    {
        std::fs::remove_dir(&directory).map_err(facts::error)?;
    }
    failure.map_or(Ok(()), Err)
}
