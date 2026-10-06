use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adaptor::gateway::external_editor::NativeEditorLauncherGateway;
use crate::adaptor::gateway::workflow::{builtin, facet, storage};
use crate::domain::app_config::ConfigRepository;
use crate::domain::external_editor::EditorLauncherGateway as _;
use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::ExternalEditorGateway;

#[derive(Clone)]
pub struct WorkflowExternalEditorGateway {
    config: Arc<dyn ConfigRepository>,
    workflows_dir: PathBuf,
    facets_base_dir: PathBuf,
}

#[cfg(any(test, feature = "test-support"))]
pub struct NoopWorkflowExternalEditorGateway;

#[cfg(any(test, feature = "test-support"))]
impl ExternalEditorGateway for NoopWorkflowExternalEditorGateway {
    fn open_workflow(&self, _name: &str) -> Result<(), WorkflowError> {
        Ok(())
    }

    fn open_facet(&self, _kind: &str, _key: &str) -> Result<(), WorkflowError> {
        Ok(())
    }
}

impl WorkflowExternalEditorGateway {
    pub(crate) fn new(config: Arc<dyn ConfigRepository>) -> Self {
        Self {
            config,
            workflows_dir: storage::workflows_dir(),
            facets_base_dir: facet::facets_base_dir(),
        }
    }
}

impl ExternalEditorGateway for WorkflowExternalEditorGateway {
    fn open_workflow(&self, name: &str) -> Result<(), WorkflowError> {
        let path = resolve_workflow_editor_path(&self.workflows_dir, name)?;
        let editor = self
            .config
            .load()
            .map_err(|e| WorkflowError::external(e.to_string()))?
            .app
            .external_editor;
        NativeEditorLauncherGateway
            .open_path(&path.to_string_lossy(), &editor, "ワークフロー")
            .map_err(WorkflowError::Editor)
    }

    fn open_facet(&self, kind: &str, key: &str) -> Result<(), WorkflowError> {
        let path = resolve_facet_editor_path(&self.facets_base_dir, kind, key)?;
        let editor = self
            .config
            .load()
            .map_err(|e| WorkflowError::external(e.to_string()))?
            .app
            .external_editor;
        NativeEditorLauncherGateway
            .open_path(&path.to_string_lossy(), &editor, "ファセット")
            .map_err(WorkflowError::Editor)
    }
}

pub fn resolve_workflow_editor_path(
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

pub fn resolve_facet_editor_path(
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

pub fn parse_editor_facet_kind(kind: &str) -> Result<facet::FacetKind, WorkflowError> {
    match kind {
        "policy" | "policies" => Ok(facet::FacetKind::Policy),
        "knowledge" => Ok(facet::FacetKind::Knowledge),
        "instruction" | "instructions" => Ok(facet::FacetKind::Instruction),
        _ => Err(WorkflowError::validation(format!(
            "Unknown facet kind: {kind}"
        ))),
    }
}

#[cfg(feature = "test-support")]
impl WorkflowExternalEditorGateway {
    pub fn test_new(
        config: Arc<dyn ConfigRepository>,
        workflows_dir: PathBuf,
        facets_base_dir: PathBuf,
    ) -> Self {
        Self {
            config,
            workflows_dir,
            facets_base_dir,
        }
    }
}
