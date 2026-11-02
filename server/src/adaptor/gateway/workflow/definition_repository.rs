use std::fs;
use std::path::{Path, PathBuf};

use super::{builtin, mapper, storage};
use crate::domain::workflow::{
    WorkflowDefinition, WorkflowDefinitionRepository, WorkflowDefinitionSourceService,
    WorkflowError, WorkflowExecutionSummary, WorkflowSource, WorkflowSourceFormat,
    WorkflowSourceSaveError, WorkflowSummary,
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

    fn load_file(&self, name: &str, path: PathBuf) -> WorkflowDefinition {
        let format = storage::workflow_source_format(&path).unwrap_or_default();
        let content = storage::load_workflow(&path, &self.facets_base_dir)
            .map_err(WorkflowError::from)
            .and_then(mapper::schema_workflow_to_domain);
        WorkflowDefinition::restore(
            name.to_string(),
            self.workflows_dir.clone(),
            WorkflowSource::File { path },
            format,
            content,
        )
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
        running: &[WorkflowExecutionSummary],
    ) -> Result<Vec<WorkflowSummary>, WorkflowError> {
        let mut summaries = storage::workflow_files(&self.workflows_dir)?
            .into_iter()
            .map(|(name, path)| self.load_file(&name, path).summary(running))
            .collect::<Vec<_>>();
        for summary in builtin::list_builtin_workflows() {
            if !summaries.iter().any(|item| item.name == summary.name) {
                let mut summary = mapper::schema_workflow_summary_to_domain(summary);
                summary.is_running = running
                    .iter()
                    .any(|execution| execution.workflow_name == summary.name);
                summaries.push(summary);
            }
        }
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(summaries)
    }

    fn get(&self, name: &str) -> Result<WorkflowDefinition, WorkflowError> {
        match storage::resolve_workflow_path(&self.workflows_dir, name) {
            Ok(path) => Ok(self.load_file(name, path)),
            Err(storage::StorageError::NotFound { .. }) => {
                let workflow = builtin::load_builtin_workflow_resolved(name, &self.facets_base_dir)
                    .map_err(|error| WorkflowError::external(error.to_string()))?
                    .ok_or_else(|| WorkflowError::NotFound(name.to_string()))?;
                Ok(WorkflowDefinition::restore(
                    name.to_string(),
                    self.workflows_dir.clone(),
                    WorkflowSource::Builtin,
                    WorkflowSourceFormat::Yaml,
                    mapper::schema_workflow_to_domain(workflow),
                ))
            }
            Err(error) => Err(error.into()),
        }
    }

    fn find(&self, name: Option<&str>) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        match name.map(|name| self.get(name)) {
            None | Some(Err(WorkflowError::NotFound(_))) => Ok(None),
            Some(result) => result.map(Some),
        }
    }

    fn save(
        &self,
        definition: WorkflowDefinition,
        original_name: Option<&str>,
    ) -> Result<(), WorkflowError> {
        let path = definition.file_path()?;
        let schema =
            mapper::domain_workflow_to_schema(definition.content().map_err(Clone::clone)?)?;
        let content = serde_saphyr::to_string(&schema)
            .map_err(|error| WorkflowError::external(error.to_string()))?;
        write_definition(path, &content, &self.workflows_dir)?;
        remove_obsolete_file(&definition, original_name)
    }

    fn save_source(
        &self,
        definition: &WorkflowDefinition,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<(), WorkflowError> {
        write_definition(definition.file_path()?, source, &self.workflows_dir)?;
        remove_obsolete_file(definition, original_name)
    }

    fn delete(&self, definition: &WorkflowDefinition) -> Result<(), WorkflowError> {
        fs::remove_file(definition.file_path()?)
            .map_err(|error| WorkflowError::external(error.to_string()))
    }
}

fn write_definition(path: &Path, content: &str, workflows_dir: &Path) -> Result<(), WorkflowError> {
    storage::ensure_dir(workflows_dir)?;
    let tmp_path = path.with_extension(format!("yml.{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&tmp_path, content).map_err(|error| WorkflowError::external(error.to_string()))?;
    if let Err(error) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(WorkflowError::external(error.to_string()));
    }
    Ok(())
}

fn remove_obsolete_file(
    definition: &WorkflowDefinition,
    original_name: Option<&str>,
) -> Result<(), WorkflowError> {
    if let Some(path) = definition.obsolete_path(original_name) {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(WorkflowError::external(format!(
                    "旧ファイル削除失敗: {error}"
                )))
            }
        }
    }
    Ok(())
}

impl WorkflowDefinitionSourceService for WorkflowDefinitionFileSourceGateway {
    fn get_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
        match storage::load_workflow_source(&self.workflows_dir, file_stem) {
            Ok(source) => Ok(Some(source)),
            Err(storage::StorageError::NotFound { .. }) => {
                Ok(builtin::builtin_workflow_source(file_stem).map(str::to_owned))
            }
            Err(error) => Err(error.into()),
        }
    }

    fn parse(&self, source: &str) -> Result<WorkflowDefinition, WorkflowSourceSaveError> {
        let workflow =
            storage::parse_workflow_source(source, &self.facets_base_dir).map_err(|error| {
                match error {
                    storage::StorageError::Diagnostics(items) => {
                        WorkflowSourceSaveError::Diagnostics(items)
                    }
                    error => WorkflowSourceSaveError::Workflow(error.into()),
                }
            })?;
        let content = mapper::schema_workflow_to_domain(workflow)
            .map_err(WorkflowSourceSaveError::Workflow)?;
        WorkflowDefinition::new(self.workflows_dir.clone(), content)
            .map_err(WorkflowSourceSaveError::Workflow)
    }
}
