use std::path::PathBuf;

use crate::adaptor::gateway::workflow::storage;
use crate::domain::workflow::{WorkflowDefinitionRepository, WorkflowError, WorkflowSummary};
use crate::domain::workflow::{WorkflowDefinitionSourceService, WorkflowSourceSaveError};

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
}

#[derive(Debug, Clone)]
pub struct WorkflowDefinitionFileSourceGateway {
    workflows_dir: PathBuf,
    facets_base_dir: PathBuf,
}

impl WorkflowDefinitionFileSourceGateway {
    pub fn new(workflows_dir: impl Into<PathBuf>, facets_base_dir: impl Into<PathBuf>) -> Self {
        Self {
            workflows_dir: workflows_dir.into(),
            facets_base_dir: facets_base_dir.into(),
        }
    }
}

impl WorkflowDefinitionRepository for WorkflowDefinitionFileRepository {
    fn list(
        &self,
        _running: &[crate::domain::workflow::WorkflowExecutionSummary],
    ) -> Result<Vec<crate::domain::workflow::WorkflowSummary>, crate::domain::workflow::WorkflowError>
    {
        todo!()
    }
    fn get(
        &self,
        _name: &str,
    ) -> Result<crate::domain::workflow::WorkflowDefinition, crate::domain::workflow::WorkflowError>
    {
        todo!()
    }
    fn find(
        &self,
        _name: Option<&str>,
    ) -> Result<
        Option<crate::domain::workflow::WorkflowDefinition>,
        crate::domain::workflow::WorkflowError,
    > {
        todo!()
    }
    fn save(
        &self,
        _definition: crate::domain::workflow::WorkflowDefinition,
        _original_name: Option<&str>,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        todo!()
    }
    fn save_source(
        &self,
        _definition: &crate::domain::workflow::WorkflowDefinition,
        _source: &str,
        _original_name: Option<&str>,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        todo!()
    }
    fn delete(
        &self,
        _definition: &crate::domain::workflow::WorkflowDefinition,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        todo!()
    }
}

impl WorkflowDefinitionSourceService for WorkflowDefinitionFileSourceGateway {
    fn get_source(&self, _file_stem: &str) -> Result<Option<String>, WorkflowError> {
        todo!()
    }
    fn parse(
        &self,
        _source: &str,
    ) -> Result<
        crate::domain::workflow::WorkflowDefinition,
        crate::domain::workflow::WorkflowSourceSaveError,
    > {
        todo!()
    }
}
