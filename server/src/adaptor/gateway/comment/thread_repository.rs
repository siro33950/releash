use std::path::PathBuf;

use crate::domain::comment::{Error, Thread, ThreadRepository};
use crate::domain::workspace_tree::WorkspaceIdentity;

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
        _workspace: &WorkspaceIdentity,
        _thread_ids: &[String],
    ) -> Result<Vec<Thread>, Error> {
        todo!()
    }
}

#[cfg(test)]
#[path = "thread_repository_test.rs"]
mod thread_repository_tests;
