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
    }

    let is_new = original_name.is_none();
    let is_rename = original_name.is_some_and(|original_name| original_name != name);
    if (is_new || is_rename) && workflows_dir.join(format!("{name}.yml")).exists() {
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
        let mut summaries: Vec<_> = storage::list_workflows(&self.workflows_dir)
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

    fn save_source(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowError> {
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

fn storage_error_to_source_save_error(error: storage::StorageError) -> WorkflowSourceSaveError {
    match error {
        storage::StorageError::Diagnostics(items) => {
            let diagnostics = items
                .into_iter()
                .map(|item| serde_json::to_value(item).unwrap_or(serde_json::Value::Null))
                .collect();
            WorkflowSourceSaveError::Diagnostics(diagnostics)
        }
        other => WorkflowSourceSaveError::Workflow(WorkflowError::external(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{FacetRefs, NodeDefinition, NodeKind, SessionSpec};
    use crate::usecase::workflow::ports::WorkflowDefinitionSourceGateway;
    use tempfile::TempDir;

    fn definition(name: &str) -> WorkflowDefinition {
        WorkflowDefinition {
            name: name.to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "node".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    permission: Some("edit".to_string()),
                    facets: FacetRefs {
                        instruction: Some("implement".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
        }
    }

    fn seed_instruction_facet(facets: &TempDir) {
        let instructions = facets.path().join("instructions");
        fs::create_dir_all(&instructions).unwrap();
        fs::write(instructions.join("implement.md"), "Implement.").unwrap();
    }

    fn source(name: &str) -> String {
        format!(
            r#"# keep this comment
name: {name}
description: source workflow
nodes:
  - name: node
    session:
      permission: edit
      gate: auto
      facets:
        instruction: implement
"#
        )
    }

    fn invalid_legacy_source(name: &str) -> String {
        format!(
            r#"
name: {name}
description: invalid workflow
nodes:
  - name: node
    type: agent
    instruction: implement
"#
        )
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

    #[test]
    fn source_gateway_saves_and_reads_verbatim_workflow_source() {
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        seed_instruction_facet(&facets);
        let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
        let source = source("wf");

        let saved = gateway.save_source(&source, None).unwrap();
        let loaded = gateway.get_source("wf").unwrap().unwrap();

        assert_eq!(saved.name, "wf");
        assert_eq!(loaded, source);
    }

    #[test]
    fn source_gateway_rejects_invalid_source_without_overwriting_existing_file() {
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        seed_instruction_facet(&facets);
        let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
        let original = source("stable");
        gateway.save_source(&original, None).unwrap();

        let err = gateway
            .save_source(&invalid_legacy_source("stable"), Some("stable"))
            .unwrap_err();
        let loaded = gateway.get_source("stable").unwrap().unwrap();

        assert!(
            matches!(err, WorkflowError::External(message) if message.contains("workflow_diagnostics") && message.contains("WFS005"))
        );
        assert_eq!(loaded, original);
    }

    #[test]
    fn source_gateway_rename_removes_old_workflow_file_after_successful_save() {
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        seed_instruction_facet(&facets);
        let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
        gateway.save_source(&source("old"), None).unwrap();

        gateway.save_source(&source("new"), Some("old")).unwrap();

        assert!(!workflows.path().join("old.yml").exists());
        assert!(workflows.path().join("new.yml").exists());
    }

    #[test]
    fn source_gateway_rejects_builtin_name_collision() {
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        seed_instruction_facet(&facets);
        let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
        let builtin_name = builtin::list_builtin_workflows()
            .first()
            .expect("builtin workflow fixture must exist")
            .name
            .clone();

        let err = gateway
            .save_source(&source(&builtin_name), None)
            .unwrap_err();

        assert!(err.to_string().contains("ビルトイン名と重複"));
        assert!(!workflows
            .path()
            .join(format!("{builtin_name}.yml"))
            .exists());
    }

    #[test]
    fn source_gateway_rejects_invalid_workflow_name() {
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        seed_instruction_facet(&facets);
        let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());

        let err = gateway.save_source(&source("-invalid"), None).unwrap_err();

        assert!(
            matches!(err, WorkflowError::External(message) if message.contains("workflow_diagnostics") && message.contains("WFS006"))
        );
        assert!(!workflows.path().join("-invalid.yml").exists());
    }

    #[test]
    fn source_gateway_returns_structured_diagnostic_for_missing_knowledge() {
        let workflows = TempDir::new().unwrap();
        let facets = TempDir::new().unwrap();
        let knowledge = facets.path().join("knowledge");
        fs::create_dir_all(&knowledge).unwrap();
        fs::write(knowledge.join("known.md"), "Known context.").unwrap();
        let gateway = WorkflowDefinitionFileSourceGateway::new(workflows.path(), facets.path());
        let source = r#"
name: missing-knowledge
description: missing knowledge diagnostic
nodes:
  - name: node
    session:
      permission: edit
      gate: auto
      facets:
        knowledge: [known, missing-name]
"#;

        let error = gateway
            .save_source_with_diagnostics(source, None)
            .unwrap_err();
        let WorkflowSourceSaveError::Diagnostics(items) = error else {
            panic!("missing knowledge must remain a structured diagnostic");
        };
        let diagnostic = items
            .iter()
            .find(|item| item["code"] == "FAC002")
            .expect("missing knowledge FAC002");
        assert_eq!(diagnostic["workflow_name"], "missing-knowledge");
        assert_eq!(diagnostic["node_name"], "node");
        assert_eq!(diagnostic["facet_key"], "missing-name");
        assert_eq!(diagnostic["facet_kind"], "knowledge");
        assert_eq!(diagnostic["field"], "knowledge");
        assert!(diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("missing-name")));
        assert!(!workflows.path().join("missing-knowledge.yml").exists());
    }
}
