use std::path::PathBuf;

use crate::domain::comment::{project_thread, Error, Thread, ThreadRepository};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::comment::ReviewEventStore;

pub(crate) struct ThreadRepositoryImpl {
    app_data_dir: PathBuf,
}

impl ThreadRepositoryImpl {
    pub(crate) fn new(app_data_dir: PathBuf) -> Self {
        Self { app_data_dir }
    }
}

impl ThreadRepository for ThreadRepositoryImpl {
    fn find_by_ids(
        &self,
        workspace: &WorkspaceIdentity,
        thread_ids: &[String],
    ) -> Result<Vec<Thread>, Error> {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let events =
            super::FileReviewEventStore::default().load(&self.app_data_dir, workspace.as_str())?;
        thread_ids
            .iter()
            .map(|id| {
                project_thread(workspace.as_str(), id, &events)
                    .ok_or_else(|| Error::NotFound(format!("Thread not found: {id}")))
            })
            .collect()
    }
}
