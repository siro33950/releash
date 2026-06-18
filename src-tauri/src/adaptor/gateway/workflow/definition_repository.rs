use std::fs;
use std::path::PathBuf;

use crate::adaptor::gateway::workflow::{builtin, storage};
use crate::domain::workflow::{
    validation, WorkflowDefinition, WorkflowDefinitionRepository, WorkflowError, WorkflowSummary,
};

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

impl WorkflowDefinitionRepository for WorkflowDefinitionFileRepository {
    fn list(&self, running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError> {
        let mut summaries: Vec<_> = storage::list_workflows(&self.workflows_dir)
            .map_err(|e| WorkflowError::external(e.to_string()))?
            .into_iter()
            .map(mapper::legacy_workflow_summary_to_domain)
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
                .and_then(|workflow| mapper::legacy_workflow_to_domain(workflow).map(Some)),
            Err(storage::StorageError::NotFound { .. }) => {
                builtin::load_builtin_workflow_resolved(file_stem)
                    .map_err(|e| WorkflowError::external(e.to_string()))?
                    .map(mapper::legacy_workflow_to_domain)
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
        validation::validate_name(&definition.name)
            .map_err(|e| WorkflowError::validation(e.to_string()))?;
        if builtin::is_builtin_workflow(&definition.name) {
            return Err(WorkflowError::validation(format!(
                "ワークフロー名 '{}' はビルトイン名と重複するため使用できません",
                definition.name
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
        }

        let is_new = original_name.is_none();
        let is_rename = original_name.is_some_and(|name| name != definition.name);
        if (is_new || is_rename)
            && self
                .workflows_dir
                .join(format!("{}.yml", definition.name))
                .exists()
        {
            return Err(WorkflowError::validation(format!(
                "ワークフロー '{}' は既に存在します",
                definition.name
            )));
        }

        let legacy = mapper::domain_workflow_to_legacy(&definition)?;
        storage::save_workflow(&self.workflows_dir, &self.facets_base_dir, &legacy)
            .map_err(|e| WorkflowError::external(e.to_string()))?;

        if let Some(original_name) = original_name {
            if original_name != definition.name {
                let old_path = self.workflows_dir.join(format!("{original_name}.yml"));
                if old_path.exists() {
                    fs::remove_file(&old_path)
                        .map_err(|e| WorkflowError::external(format!("旧ファイル削除失敗: {e}")))?;
                }
            }
        }
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), WorkflowError> {
        storage::delete_workflow(&self.workflows_dir, name)
            .map_err(|e| WorkflowError::external(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{NodeDefinition, NodeType};
    use tempfile::TempDir;

    fn definition(name: &str) -> WorkflowDefinition {
        WorkflowDefinition {
            name: name.to_string(),
            description: "desc".to_string(),
            builtin: false,
            variables: Default::default(),
            nodes: vec![NodeDefinition {
                name: "step".to_string(),
                node_type: NodeType::Agent,
                inline_prompt: Some("body".to_string()),
                permission: Some("edit".to_string()),
                ..Default::default()
            }],
        }
    }

    #[test]
    fn saves_and_loads_workflow_yaml_through_existing_storage() {
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        let repo = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());

        repo.save(definition("wf"), None).unwrap();
        let loaded = repo.get("wf").unwrap().unwrap();

        assert_eq!(loaded.name, "wf");
        assert!(workflows.path().join("wf.yml").exists());
    }

    #[test]
    fn rename_removes_old_workflow_file_after_successful_save() {
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        let repo = WorkflowDefinitionFileRepository::new(workflows.path(), facets.path());
        repo.save(definition("old"), None).unwrap();

        repo.save(definition("new"), Some("old")).unwrap();

        assert!(!workflows.path().join("old.yml").exists());
        assert!(workflows.path().join("new.yml").exists());
    }
}
