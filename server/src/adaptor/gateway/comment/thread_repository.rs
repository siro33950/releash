use std::collections::HashMap;
use std::path::PathBuf;

use crate::domain::comment::{project_thread, Error, Thread, ThreadRepository};
use crate::usecase::comment::ReviewEventStore;

pub(crate) struct FileThreadRepository {
    app_data_dir: PathBuf,
}

impl FileThreadRepository {
    pub(crate) fn new(app_data_dir: PathBuf) -> Self {
        Self { app_data_dir }
    }
}

impl ThreadRepository for FileThreadRepository {
    fn find_by_ids(&self, thread_ids: &[String]) -> Result<Vec<Thread>, Error> {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let config = super::super::app_config::read_config_if_exists(
            &self.app_data_dir.join("releash.toml"),
        )
        .map_err(Error::Io)?;
        let repositories = config
            .map(|config| config.app.last_repo_paths)
            .unwrap_or_default();
        let store = super::FileReviewEventStore::default();
        let mut found = HashMap::new();
        'repositories: for repository in repositories {
            let worktrees = super::super::repository::worktree::list_worktrees(&repository)
                .map_err(|error| match error {
                    crate::domain::repository::RepositoryError::Technical(error) => {
                        Error::Technical(error)
                    }
                    crate::domain::repository::RepositoryError::Rule(message) => {
                        Error::InvalidInput(message)
                    }
                    crate::domain::repository::RepositoryError::External(message) => {
                        Error::Io(message)
                    }
                })?;
            for worktree in worktrees {
                let events = store.load(&self.app_data_dir, &worktree.path)?;
                for id in thread_ids {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        found.entry(id.clone())
                    {
                        if let Some(thread) = project_thread(&worktree.path, id, &events) {
                            entry.insert(thread);
                        }
                    }
                }
                if thread_ids.iter().all(|id| found.contains_key(id)) {
                    break 'repositories;
                }
            }
        }
        thread_ids
            .iter()
            .map(|id| {
                found
                    .get(id)
                    .cloned()
                    .ok_or_else(|| Error::NotFound(format!("Thread not found: {id}")))
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "thread_repository_test.rs"]
mod thread_repository_tests;
