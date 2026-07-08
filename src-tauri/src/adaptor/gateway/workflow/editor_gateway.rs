use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adaptor::gateway::external_editor::TauriEditorLauncherGateway;
use crate::adaptor::gateway::workflow::{builtin, facet, storage};
use crate::domain::app_config::ConfigRepository;
use crate::domain::external_editor::EditorLauncherGateway as _;
use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::ExternalEditorGateway;

#[derive(Clone)]
pub(crate) struct TauriWorkflowExternalEditorGateway<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    config: Arc<dyn ConfigRepository>,
    workflows_dir: PathBuf,
    facets_base_dir: PathBuf,
}

#[cfg(test)]
pub(crate) struct NoopWorkflowExternalEditorGateway;

#[cfg(test)]
impl ExternalEditorGateway for NoopWorkflowExternalEditorGateway {
    fn open_workflow(&self, _name: &str) -> Result<(), WorkflowError> {
        Ok(())
    }

    fn open_facet(&self, _kind: &str, _key: &str) -> Result<(), WorkflowError> {
        Ok(())
    }
}

impl<R: tauri::Runtime> TauriWorkflowExternalEditorGateway<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>, config: Arc<dyn ConfigRepository>) -> Self {
        Self {
            app,
            config,
            workflows_dir: storage::workflows_dir(),
            facets_base_dir: facet::facets_base_dir(),
        }
    }
}

impl<R: tauri::Runtime + 'static> ExternalEditorGateway for TauriWorkflowExternalEditorGateway<R> {
    fn open_workflow(&self, name: &str) -> Result<(), WorkflowError> {
        let path = resolve_workflow_editor_path(&self.workflows_dir, name)?;
        let editor = self
            .config
            .load()
            .map_err(|e| WorkflowError::external(e.to_string()))?
            .app
            .external_editor;
        TauriEditorLauncherGateway::new(self.app.clone())
            .open_path(&path.to_string_lossy(), &editor, "ワークフロー")
            .map_err(WorkflowError::external)
    }

    fn open_facet(&self, kind: &str, key: &str) -> Result<(), WorkflowError> {
        let path = resolve_facet_editor_path(&self.facets_base_dir, kind, key)?;
        let editor = self
            .config
            .load()
            .map_err(|e| WorkflowError::external(e.to_string()))?
            .app
            .external_editor;
        TauriEditorLauncherGateway::new(self.app.clone())
            .open_path(&path.to_string_lossy(), &editor, "ファセット")
            .map_err(WorkflowError::external)
    }
}

fn resolve_workflow_editor_path(
    workflows_dir: &Path,
    name: &str,
) -> Result<PathBuf, WorkflowError> {
    if builtin::is_builtin_workflow(name) {
        return Err(WorkflowError::validation(
            "ビルトインワークフローは外部エディタで開けません",
        ));
    }
    storage::resolve_workflow_path(workflows_dir, name)
        .map_err(|e| WorkflowError::external(e.to_string()))
}

fn resolve_facet_editor_path(
    facets_base_dir: &Path,
    kind: &str,
    key: &str,
) -> Result<PathBuf, WorkflowError> {
    let kind = parse_editor_facet_kind(kind)?;
    if builtin::is_builtin_facet(kind, key) {
        return Err(WorkflowError::validation(
            "ビルトインファセットは外部エディタで開けません",
        ));
    }
    facet::resolve_facet_path(kind, key, facets_base_dir)
        .map_err(|e| WorkflowError::external(e.to_string()))
}

fn parse_editor_facet_kind(kind: &str) -> Result<facet::FacetKind, WorkflowError> {
    match kind {
        "policy" | "policies" => Ok(facet::FacetKind::Policy),
        "knowledge" => Ok(facet::FacetKind::Knowledge),
        "instruction" | "instructions" => Ok(facet::FacetKind::Instruction),
        _ => Err(WorkflowError::validation(format!(
            "Unknown facet kind: {kind}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, NodeDefinition, NodeKind, SessionSpec, Workflow,
    };
    use tempfile::TempDir;

    #[test]
    fn workflow_editor_path_rejects_builtin_and_resolves_custom_file() {
        let tmp = TempDir::new().unwrap();
        let workflow = Workflow {
            name: "custom".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "step".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    permission: Some("edit".to_string()),
                    facets: FacetRefs {
                        instruction: Some("implement".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..NodeDefinition::default()
            }],
        };
        storage::save_workflow(tmp.path(), &workflow).unwrap();

        let path = resolve_workflow_editor_path(tmp.path(), "custom").unwrap();

        assert_eq!(path.file_name().unwrap(), "custom.yml");
        if let Some(summary) = builtin::list_builtin_workflows().first() {
            assert!(resolve_workflow_editor_path(tmp.path(), &summary.name).is_err());
        }
    }

    #[test]
    fn facet_editor_path_rejects_builtin_and_resolves_custom_file() {
        let tmp = TempDir::new().unwrap();
        facet::save_facet(facet::FacetKind::Instruction, "custom", "body", tmp.path()).unwrap();

        let path = resolve_facet_editor_path(tmp.path(), "instructions", "custom").unwrap();

        assert_eq!(path.file_name().unwrap(), "custom.md");
        if let Some(key) = builtin::list_builtin_facet_keys(facet::FacetKind::Instruction).first() {
            assert!(resolve_facet_editor_path(tmp.path(), "instructions", key).is_err());
        }
        assert!(resolve_facet_editor_path(tmp.path(), "persona", "custom").is_err());
    }

    #[test]
    fn parse_editor_facet_kind_accepts_wire_and_directory_names() {
        assert_eq!(
            parse_editor_facet_kind("policy").unwrap(),
            facet::FacetKind::Policy
        );
        assert_eq!(
            parse_editor_facet_kind("policies").unwrap(),
            facet::FacetKind::Policy
        );
    }
}
