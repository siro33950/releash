use std::path::PathBuf;

use super::{builtin, mapper, storage};
use crate::domain::workflow::{
    WorkflowDefinition, WorkflowDefinitionRepository, WorkflowError, WorkflowSource,
    WorkflowSourceFormat,
};

#[derive(Debug, Clone)]
pub struct WorkflowDefinitionFileRepository {
    workflows_dir: PathBuf,
    facets_base_dir: PathBuf,
}

impl WorkflowDefinitionFileRepository {
    pub fn new(workflows_dir: impl Into<PathBuf>, facets_base_dir: impl Into<PathBuf>) -> Self {
        Self {
            workflows_dir: workflows_dir.into(),
            facets_base_dir: facets_base_dir.into(),
        }
    }

    pub(crate) fn default_workflows_dir() -> PathBuf {
        storage::workflows_dir()
    }

    fn load_builtin(&self, name: &str) -> Result<WorkflowDefinition, WorkflowError> {
        let content = match builtin::load_builtin_workflow_resolved(name, &self.facets_base_dir) {
            Ok(Some(workflow)) => mapper::schema_workflow_to_domain(workflow),
            Ok(None) => return Err(WorkflowError::NotFound(name.to_string())),
            Err(error) => Err(WorkflowError::external(error.to_string())),
        };
        Ok(WorkflowDefinition::restore(
            self.workflows_dir.clone(),
            WorkflowSource::Builtin,
            WorkflowSourceFormat::Yaml,
            content,
        ))
    }

    fn load_file(&self, path: PathBuf) -> WorkflowDefinition {
        let format = storage::workflow_source_format(&path).unwrap_or_default();
        let content = storage::load_workflow(&path, &self.facets_base_dir)
            .map_err(WorkflowError::from)
            .and_then(mapper::schema_workflow_to_domain);
        WorkflowDefinition::restore(
            self.workflows_dir.clone(),
            WorkflowSource::File { path },
            format,
            content,
        )
    }
}

impl WorkflowDefinitionRepository for WorkflowDefinitionFileRepository {
    fn get(&self, name: &str) -> Result<WorkflowDefinition, WorkflowError> {
        match storage::resolve_workflow_path(&self.workflows_dir, name) {
            Ok(path) => Ok(self.load_file(path)),
            Err(storage::StorageError::NotFound { .. }) => self.load_builtin(name),
            Err(error) => Err(error.into()),
        }
    }

    fn find(&self, name: Option<&str>) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        match name.map(|name| self.get(name)) {
            None | Some(Err(WorkflowError::NotFound(_))) => Ok(None),
            Some(result) => result.map(Some),
        }
    }
}
