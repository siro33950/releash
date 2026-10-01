use std::fs;
use std::path::{Path, PathBuf};

use crate::adaptor::gateway::workflow::{builtin, storage};
use crate::domain::workflow::{
    validation, WorkflowDefinition, WorkflowDefinitionRepository, WorkflowError, WorkflowSummary,
};
use crate::usecase::workflow::ports::{WorkflowDefinitionSourceGateway, WorkflowSourceSaveError};

use super::mapper;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowDefinitionFileRepository {
    workflows_dir: PathBuf,
    facets_base_dir: PathBuf,
}

impl WorkflowDefinitionFileRepository {
    pub(crate) fn new(
        workflows_dir: impl Into<PathBuf>,
        facets_base_dir: impl Into<PathBuf>,
    ) -> Self {
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
pub(crate) struct WorkflowDefinitionFileSourceGateway {
    workflows_dir: PathBuf,
    facets_base_dir: PathBuf,
}

impl WorkflowDefinitionFileSourceGateway {
    pub(crate) fn new(
        workflows_dir: impl Into<PathBuf>,
        facets_base_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            workflows_dir: workflows_dir.into(),
            facets_base_dir: facets_base_dir.into(),
        }
    }
}

#[derive(Debug, Clone)]
struct WorkflowSavePlan {
    name: String,
    original_name: Option<String>,
}

impl WorkflowSavePlan {
    fn is_rename(&self) -> bool {
        self.original_name
            .as_deref()
            .is_some_and(|original_name| original_name != self.name)
    }
}

fn validate_and_prepare_save(
    workflows_dir: &Path,
    name: &str,
    original_name: Option<&str>,
) -> Result<WorkflowSavePlan, WorkflowError> {
    validation::validate_name(name).map_err(|e| WorkflowError::validation(e.to_string()))?;
    if builtin::is_builtin_workflow(name) {
        return Err(WorkflowError::validation(format!(
            "ワークフロー名 '{name}' はビルトイン名と重複するため使用できません"
        )));
    }
    if let Some(original_name) = original_name {
        validation::validate_name(original_name)
            .map_err(|e| WorkflowError::validation(e.to_string()))?;
        if builtin::is_builtin_workflow(original_name) {
            return Err(WorkflowError::validation(
                "ビルトインワークフローは編集できません",
            ));
        }
        if workflows_dir.join(format!("{original_name}.lua")).exists() {
            return Err(WorkflowError::validation(
                "Lua workflow は保存できません。外部エディタで編集してください",
            ));
        }
    }

    let is_new = original_name.is_none();
    let is_rename = original_name.is_some_and(|original_name| original_name != name);
    if (is_new || is_rename)
        && ["yml", "lua"]
            .iter()
            .any(|extension| workflows_dir.join(format!("{name}.{extension}")).exists())
    {
        return Err(WorkflowError::validation(format!(
            "ワークフロー '{name}' は既に存在します"
        )));
    }

    Ok(WorkflowSavePlan {
        name: name.to_string(),
        original_name: original_name.map(str::to_string),
    })
}

fn remove_renamed_workflow_file_after_success(
    workflows_dir: &Path,
    plan: &WorkflowSavePlan,
) -> Result<(), WorkflowError> {
    if !plan.is_rename() {
        return Ok(());
    }
    let original_name = plan
        .original_name
        .as_deref()
        .expect("rename plan must retain original name");
    let old_path = workflows_dir.join(format!("{original_name}.yml"));
    if old_path.exists() {
        fs::remove_file(&old_path)
            .map_err(|e| WorkflowError::external(format!("旧ファイル削除失敗: {e}")))?;
    }
    Ok(())
}

impl WorkflowDefinitionRepository for WorkflowDefinitionFileRepository {
    fn list(&self, running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError> {
        let mut summaries: Vec<_> =
            storage::list_workflows_with_facets(&self.workflows_dir, &self.facets_base_dir)
                .map_err(|e| WorkflowError::external(e.to_string()))?
                .into_iter()
                .map(mapper::schema_workflow_summary_to_domain)
                .collect();
        for summary in &mut summaries {
            summary.is_running = running_names.contains(&summary.name);
        }
        Ok(summaries)
    }

    fn get(&self, file_stem: &str) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        match storage::resolve_workflow_path(&self.workflows_dir, file_stem) {
            Ok(path) => storage::load_workflow(&path, &self.facets_base_dir)
                .map_err(|e| WorkflowError::external(e.to_string()))
                .and_then(|workflow| mapper::schema_workflow_to_domain(workflow).map(Some)),
            Err(storage::StorageError::NotFound { .. }) => {
                builtin::load_builtin_workflow_resolved(file_stem)
                    .map_err(|e| WorkflowError::external(e.to_string()))?
                    .map(mapper::schema_workflow_to_domain)
                    .transpose()
            }
            Err(e) => Err(WorkflowError::external(e.to_string())),
        }
    }

    fn save(
        &self,
        definition: WorkflowDefinition,
        original_name: Option<&str>,
    ) -> Result<(), WorkflowError> {
        let plan = validate_and_prepare_save(&self.workflows_dir, &definition.name, original_name)?;
        let schema = mapper::domain_workflow_to_schema(&definition)?;
        storage::save_workflow(&self.workflows_dir, &schema)
            .map_err(|e| WorkflowError::external(e.to_string()))?;
        remove_renamed_workflow_file_after_success(&self.workflows_dir, &plan)
    }

    fn delete(&self, name: &str) -> Result<(), WorkflowError> {
        storage::delete_workflow(&self.workflows_dir, name)
            .map_err(|e| WorkflowError::external(e.to_string()))
    }
}

impl WorkflowDefinitionSourceGateway for WorkflowDefinitionFileSourceGateway {
    fn get_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
        match storage::load_workflow_source(&self.workflows_dir, file_stem) {
            Ok(source) => Ok(Some(source)),
            Err(storage::StorageError::NotFound { .. }) => {
                Ok(builtin::builtin_workflow_source(file_stem).map(str::to_owned))
            }
            Err(e) => Err(WorkflowError::external(e.to_string())),
        }
    }

    fn source_format(
        &self,
        file_stem: &str,
    ) -> Result<crate::domain::workflow::WorkflowSourceFormat, WorkflowError> {
        match storage::resolve_workflow_path(&self.workflows_dir, file_stem) {
            Ok(path) => storage::workflow_source_format(&path).ok_or_else(|| {
                WorkflowError::external("workflow source has an unsupported extension")
            }),
            Err(storage::StorageError::NotFound { .. })
                if builtin::is_builtin_workflow(file_stem) =>
            {
                Ok(crate::domain::workflow::WorkflowSourceFormat::Yaml)
            }
            Err(error) => Err(WorkflowError::external(error.to_string())),
        }
    }

    fn save_source(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        reject_lua_source_save(self, original_name)?;
        let workflow = storage::parse_workflow_source(source, &self.facets_base_dir)
            .map_err(|e| WorkflowError::external(e.to_string()))?;
        let plan = validate_and_prepare_save(&self.workflows_dir, &workflow.name, original_name)?;
        let saved =
            storage::save_workflow_source(&self.workflows_dir, &self.facets_base_dir, source)
                .map_err(|e| WorkflowError::external(e.to_string()))?;
        remove_renamed_workflow_file_after_success(&self.workflows_dir, &plan)?;
        mapper::schema_workflow_to_domain(saved)
    }

    fn save_source_with_diagnostics(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowSourceSaveError> {
        reject_lua_source_save(self, original_name).map_err(WorkflowSourceSaveError::Workflow)?;
        let workflow = storage::parse_workflow_source(source, &self.facets_base_dir)
            .map_err(storage_error_to_source_save_error)?;
        let plan = validate_and_prepare_save(&self.workflows_dir, &workflow.name, original_name)
            .map_err(WorkflowSourceSaveError::Workflow)?;
        let saved =
            storage::save_workflow_source(&self.workflows_dir, &self.facets_base_dir, source)
                .map_err(storage_error_to_source_save_error)?;
        remove_renamed_workflow_file_after_success(&self.workflows_dir, &plan)
            .map_err(WorkflowSourceSaveError::Workflow)?;
        mapper::schema_workflow_to_domain(saved).map_err(WorkflowSourceSaveError::Workflow)
    }
}

fn reject_lua_source_save(
    gateway: &WorkflowDefinitionFileSourceGateway,
    original_name: Option<&str>,
) -> Result<(), WorkflowError> {
    if let Some(original_name) = original_name {
        if gateway.source_format(original_name)?
            == crate::domain::workflow::WorkflowSourceFormat::Lua
        {
            return Err(WorkflowError::validation(
                "Lua workflow は保存できません。外部エディタで編集してください",
            ));
        }
    }
    Ok(())
}

fn storage_error_to_source_save_error(error: storage::StorageError) -> WorkflowSourceSaveError {
    match error {
        storage::StorageError::Diagnostics(items) => WorkflowSourceSaveError::Diagnostics(items),
        other => WorkflowSourceSaveError::Workflow(WorkflowError::external(other.to_string())),
    }
}

#[cfg(test)]
#[path = "definition_repository_test.rs"]
mod definition_repository_tests;
