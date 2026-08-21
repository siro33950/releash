//! Workflow command usecases.
//!
//! This layer orchestrates domain aggregates through repository/gateway ports.
//! Controllers, CLI adapters, and watchers should converge
//! here as the legacy `workflow` module is removed.

pub(crate) mod command;
pub(crate) mod control_plane;
mod definition;
pub(crate) mod dto;
pub(crate) mod event_draft;
mod facet;
pub(crate) mod output;
pub(crate) mod output_submission;
pub(crate) mod ports;
pub(crate) mod query_service;
pub(crate) mod runtime_command;
pub(crate) mod runtime_driver;
pub(crate) mod runtime_error;
pub(crate) mod runtime_resolver;
pub(crate) mod runtime_snapshot;
pub(crate) mod runtime_start_guard;
#[cfg(test)]
pub(crate) mod test_support;
mod workspace_node_command;
mod workspace_tree;

use serde_json::Value;

use crate::domain::workflow::{
    ExecutionStatusFilter, FacetKind, FacetRepository, FacetSummary, ManagedWorktreeGateway,
    SecretSourceGateway, WorkflowDefinition, WorkflowDefinitionRepository, WorkflowError,
    WorkflowExecution, WorkflowExecutionArchiveRepository, WorkflowExecutionSummary,
    WorkflowPageRequest,
};
use crate::usecase::workflow::ports::{
    ExternalEditorGateway, WorkflowConfigPathGateway, WorkflowDefinitionSourceGateway,
    WorkflowDiagnosticsGateway, WorkflowSourceSaveError,
};

use definition::WorkflowDefinitionUsecase;
use facet::WorkflowFacetUsecase;
pub(crate) use output::WorkflowOutputUsecase;
pub use output::WorkflowValidateOutputResult;
use query_service::WorkflowQueryService;
pub use query_service::{WorkflowEventView, WorkflowGetOutputResult};
pub use runtime_command::WorkflowRuntimeUsecase;
pub(crate) use workspace_node_command::{
    ApproveWorkspaceNodeCommand, RetryWorkspaceNodeCommand, WorkspaceNodeActionResolver,
    WorkspaceNodeCommandUsecase, WorkspaceNodeWorkflowCommandExecutor,
};
pub(crate) use workspace_tree::{
    WorkspaceCommandNodeContentDto, WorkspaceCommandResultDto, WorkspaceFanoutDto,
    WorkspaceNodeCapabilitiesDto, WorkspaceNodeContentDto, WorkspaceNodeDetailDto,
    WorkspaceNodeDto, WorkspaceSessionNodeContentDto, WorkspaceTreeItemDto,
    WorkspaceTreeSelectionSnapshotDto, WorkspaceTreeSnapshotDto, WorkspaceWorkflowCapabilitiesDto,
    WorkspaceWorkflowDto, WorkspaceWorkflowHistoryItemDto,
};

#[derive(Clone)]
pub(crate) struct WorkflowReadUsecase {
    query: WorkflowQueryService,
    workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    output: WorkflowOutputUsecase,
    worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
}

impl WorkflowReadUsecase {
    pub(crate) fn new(
        query: WorkflowQueryService,
        worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
        secrets: std::sync::Arc<dyn SecretSourceGateway>,
        workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    ) -> Self {
        Self {
            output: WorkflowOutputUsecase::new(query.clone(), secrets),
            query,
            worktrees,
            workspace_query,
        }
    }

    pub(crate) fn list_workflow_summaries(
        &self,
    ) -> Result<Vec<dto::WorkflowSummaryDto>, WorkflowError> {
        let running_names = self
            .workspace_query
            .execution_summaries(None, Some(ExecutionStatusFilter::Active), None)?
            .into_iter()
            .map(|execution| execution.workflow_name)
            .collect::<Vec<_>>();
        self.query.list_workflows(&running_names).map(|summaries| {
            summaries
                .into_iter()
                .map(dto::workflow_summary_to_dto)
                .collect()
        })
    }

    pub(crate) fn list_executions_filtered(
        &self,
        status: Option<ExecutionStatusFilter>,
        worktree_path: Option<&str>,
        page: WorkflowPageRequest,
    ) -> Result<Vec<dto::WorkflowExecutionSummaryDto>, WorkflowError> {
        let worktree_path = worktree_path
            .filter(|worktree_path| !worktree_path.is_empty())
            .map(|worktree_path| self.worktrees.resolve(worktree_path))
            .transpose()?
            .map(crate::domain::workspace_tree::WorkspaceIdentity::new);
        self.workspace_query
            .execution_summaries(worktree_path.as_ref(), status, Some(page))
            .map(|executions| {
                executions
                    .into_iter()
                    .map(dto::workflow_execution_summary_to_dto)
                    .collect()
            })
    }

    pub(crate) fn get_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        self.workspace_query.execution_summary(execution_id)
    }

    pub(crate) fn get_execution_log_page(
        &self,
        execution_id: &str,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventView>, WorkflowError> {
        if self.get_execution(execution_id)?.is_none() {
            return Err(WorkflowError::NotFound(format!(
                "Workflow execution not found: {execution_id}"
            )));
        }
        self.query.get_execution_log_page(execution_id, page)
    }

    pub(crate) fn get_execution_state(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecution>, WorkflowError> {
        self.query.get_execution_state(execution_id)
    }

    pub(crate) fn validate_output_for_contract(
        &self,
        execution_id: &str,
        node_name: &str,
        contract: &str,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        self.output.validate_output_for_contract(
            execution_id,
            node_name,
            contract,
            structured_output,
        )
    }

    pub(crate) fn get_output(
        &self,
        execution_id: &str,
        node_name: &str,
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        self.output.get_output(execution_id, node_name)
    }
}

#[derive(Clone)]
pub struct WorkflowUsecase {
    query: WorkflowQueryService,
    definition_commands: WorkflowDefinitionUsecase,
    facet_commands: WorkflowFacetUsecase,
    output: WorkflowOutputUsecase,
    worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
    editors: std::sync::Arc<dyn ExternalEditorGateway>,
    diagnostics: std::sync::Arc<dyn WorkflowDiagnosticsGateway>,
    config_paths: std::sync::Arc<dyn WorkflowConfigPathGateway>,
    execution_archives: std::sync::Arc<dyn WorkflowExecutionArchiveRepository>,
    workspace_nodes: std::sync::Arc<dyn crate::domain::workspace_tree::WorkspaceTreeRepository>,
    workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    read: WorkflowReadUsecase,
}

impl WorkflowUsecase {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        query: WorkflowQueryService,
        definitions: std::sync::Arc<dyn WorkflowDefinitionRepository>,
        definition_sources: std::sync::Arc<dyn WorkflowDefinitionSourceGateway>,
        facets: std::sync::Arc<dyn FacetRepository>,
        worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
        editors: std::sync::Arc<dyn ExternalEditorGateway>,
        diagnostics: std::sync::Arc<dyn WorkflowDiagnosticsGateway>,
        config_paths: std::sync::Arc<dyn WorkflowConfigPathGateway>,
        secrets: std::sync::Arc<dyn SecretSourceGateway>,
        execution_archives: std::sync::Arc<dyn WorkflowExecutionArchiveRepository>,
        workspace_nodes: std::sync::Arc<dyn crate::domain::workspace_tree::WorkspaceTreeRepository>,
        workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    ) -> Self {
        let definition_commands = WorkflowDefinitionUsecase::new(definitions, definition_sources);
        let facet_commands = WorkflowFacetUsecase::new(facets.clone());
        let output = WorkflowOutputUsecase::new(query.clone(), secrets);
        let read = WorkflowReadUsecase {
            query: query.clone(),
            output: output.clone(),
            worktrees: worktrees.clone(),
            workspace_query: workspace_query.clone(),
        };
        Self {
            query,
            definition_commands,
            facet_commands,
            output,
            worktrees,
            editors,
            diagnostics,
            config_paths,
            execution_archives,
            workspace_nodes,
            workspace_query,
            read,
        }
    }

    pub(crate) fn read_usecase(&self) -> WorkflowReadUsecase {
        self.read.clone()
    }

    pub fn list_executions_for_worktree(
        &self,
        status: Option<ExecutionStatusFilter>,
        worktree_path: &str,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        let worktree_path = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        self.workspace_query
            .execution_summaries(Some(&worktree_path), status, None)
    }

    pub fn get_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        self.workspace_query.execution_summary(execution_id)
    }

    pub fn authorize_execution_summary(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        let Some(summary) = self.get_execution(execution_id)? else {
            return Ok(None);
        };
        match self.resolve_worktree_path(&summary.worktree_path) {
            Ok(_) => Ok(Some(summary)),
            Err(_) => Ok(None),
        }
    }

    pub fn authorize_execution_summary_for_worktree(
        &self,
        execution_id: &str,
        worktree_path: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        let canonical = self.resolve_worktree_path(worktree_path)?;
        let Some(summary) = self.authorize_execution_summary(execution_id)? else {
            return Ok(None);
        };
        if summary.worktree_path == canonical {
            Ok(Some(summary))
        } else {
            Ok(None)
        }
    }

    pub fn authorize_execution_access_for_worktree(
        &self,
        execution_id: &str,
        worktree_path: &str,
    ) -> Result<(), WorkflowError> {
        let execution_id =
            crate::domain::workflow::WorkflowExecutionId::new(execution_id.to_string())?;
        if self
            .authorize_execution_summary_for_worktree(execution_id.as_str(), worktree_path)?
            .is_some()
        {
            Ok(())
        } else {
            Err(WorkflowError::external(format!(
                "Workflow execution not found: {execution_id}"
            )))
        }
    }

    pub fn authorize_node_execution_access_for_worktree(
        &self,
        node_execution_id: &str,
        worktree_path: &str,
    ) -> Result<(), WorkflowError> {
        if node_execution_id.trim().is_empty() {
            return Err(WorkflowError::validation(
                "node_execution_id must not be empty",
            ));
        }
        let node = self
            .workspace_nodes
            .load_node_by_node_execution_id(node_execution_id)
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::external(format!("Node execution not found: {node_execution_id}"))
            })?;
        let execution_id = node.execution_id.ok_or_else(|| {
            WorkflowError::external(format!("Node execution not found: {node_execution_id}"))
        })?;
        self.authorize_execution_access_for_worktree(&execution_id, worktree_path)
    }

    pub fn resolve_worktree_by_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<String>, WorkflowError> {
        Ok(self
            .workspace_query
            .execution_summary(execution_id)?
            .map(|execution| execution.worktree_path))
    }

    pub fn resolve_worktree_path(&self, worktree_path: &str) -> Result<String, WorkflowError> {
        self.worktrees.resolve(worktree_path)
    }

    pub fn get_workflow(
        &self,
        file_stem: &str,
    ) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        self.query.get_workflow(file_stem)
    }

    pub fn get_workflow_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
        self.query.get_workflow_source(file_stem)
    }

    pub fn get_workflow_source_format(
        &self,
        file_stem: &str,
    ) -> Result<crate::domain::workflow::WorkflowSourceFormat, WorkflowError> {
        self.query.get_workflow_source_format(file_stem)
    }

    pub fn get_execution_log(
        &self,
        execution_id: &str,
    ) -> Result<Vec<WorkflowEventView>, WorkflowError> {
        if self.get_execution(execution_id)?.is_none() {
            return Err(WorkflowError::NotFound(format!(
                "Workflow execution not found: {execution_id}"
            )));
        }
        self.query.get_execution_log(execution_id)
    }

    pub fn get_execution_state(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecution>, WorkflowError> {
        self.query.get_execution_state(execution_id)
    }

    pub fn get_node_detail(
        &self,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Result<Option<crate::domain::workflow::NodeExecution>, WorkflowError> {
        self.query.get_node_detail(execution_id, node_execution_id)
    }

    pub fn list_facets(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
        self.query.list_facets(kind)
    }

    pub fn get_facet(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError> {
        self.query.get_facet(kind, key)
    }

    pub fn list_facet_summaries(
        &self,
        kind: FacetKind,
    ) -> Result<Vec<FacetSummary>, WorkflowError> {
        self.query.list_facet_summaries(kind)
    }

    #[cfg(test)]
    pub fn save_workflow_source(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        self.definition_commands
            .save_workflow_source(source, original_name)
    }

    pub fn save_workflow_source_with_diagnostics(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowSourceSaveError> {
        self.definition_commands
            .save_workflow_source_with_diagnostics(source, original_name)
    }

    pub fn delete_workflow(&self, name: &str) -> Result<(), WorkflowError> {
        self.definition_commands.delete_workflow(name)
    }

    pub fn duplicate_workflow(
        &self,
        source_name: &str,
        new_name: &str,
    ) -> Result<(), WorkflowError> {
        self.definition_commands
            .duplicate_workflow(source_name, new_name)
    }

    pub fn save_facet(
        &self,
        kind: FacetKind,
        key: &str,
        content: &str,
        is_new: bool,
    ) -> Result<(), WorkflowError> {
        self.facet_commands.save_facet(kind, key, content, is_new)
    }

    pub fn delete_facet(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        self.facet_commands.delete_facet(kind, key)
    }

    pub fn duplicate_facet(
        &self,
        kind: FacetKind,
        source_key: &str,
        new_key: &str,
    ) -> Result<(), WorkflowError> {
        self.facet_commands
            .duplicate_facet(kind, source_key, new_key)
    }

    pub fn open_workflow_in_editor(&self, name: &str) -> Result<(), WorkflowError> {
        self.editors.open_workflow(name)
    }

    pub fn open_facet_in_editor(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        self.editors.open_facet(kind.dir_name(), key)
    }

    pub fn diagnose_all(&self) -> Result<serde_json::Value, WorkflowError> {
        self.diagnostics.diagnose_all()
    }

    pub fn automation_config_dir(&self) -> Result<String, WorkflowError> {
        self.config_paths.automation_config_dir()
    }

    pub fn render_facet_preview(
        &self,
        content: &str,
        sample_values: &std::collections::HashMap<String, String>,
    ) -> String {
        self.facet_commands
            .render_facet_preview(content, sample_values)
    }

    pub fn validate_output(
        &self,
        execution_id: &str,
        node_name: &str,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        self.output
            .validate_output(execution_id, node_name, structured_output)
    }

    pub fn get_output(
        &self,
        execution_id: &str,
        node_name: &str,
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        self.output.get_output(execution_id, node_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionStatus, ExecutionStatusFilter, WorkflowExecution,
        WorkflowExecutionId, WorkflowSummary,
    };
    use crate::usecase::workflow::ports::{
        ExternalEditorGateway, WorkflowConfigPathGateway, WorkflowDiagnosticsGateway,
        WorkflowEventDraft, WorkflowEventRepository, WorkflowExecutionProjectionRepository,
    };
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir)
            .unwrap_or_else(|err| panic!("read source dir {}: {err}", dir.display()))
        {
            let path = entry
                .unwrap_or_else(|err| panic!("read source entry in {}: {err}", dir.display()))
                .path();
            if path.is_dir() {
                collect_rs_files(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }

    fn assert_no_forbidden_crate_dependencies(relative_dir: &str, forbidden_modules: &[&str]) {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let layer_root = source_root.join(relative_dir);
        let mut files = Vec::new();
        collect_rs_files(&layer_root, &mut files);
        files.sort();

        assert!(
            !files.is_empty(),
            "no workflow source files found under {relative_dir}"
        );

        let forbidden_refs = forbidden_modules
            .iter()
            .map(|module| format!("crate::{module}"))
            .collect::<Vec<_>>();
        let mut violations = Vec::new();

        for file in files {
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|err| panic!("read source file {}: {err}", file.display()));
            for forbidden in &forbidden_refs {
                if content.contains(forbidden) {
                    let display_path = file.strip_prefix(&source_root).unwrap_or(&file);
                    violations.push(format!("{} -> {forbidden}", display_path.display()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "workflow layer dependency violations:\n{}",
            violations.join("\n")
        );
    }

    fn production_source(content: &str) -> String {
        let mut lines = Vec::new();
        for line in content.lines() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    fn production_source_excluding_cfg_test_items(content: &str) -> String {
        let lines = content.lines().collect::<Vec<_>>();
        let mut production = Vec::new();
        let mut index = 0;

        while index < lines.len() {
            let line = lines[index];
            if line.trim_start().starts_with("#[cfg(test)]") {
                index = skip_cfg_test_item(&lines, index + 1);
            } else {
                production.push(line);
                index += 1;
            }
        }

        production.join("\n")
    }

    fn skip_cfg_test_item(lines: &[&str], mut index: usize) -> usize {
        while index < lines.len() && lines[index].trim().is_empty() {
            index += 1;
        }
        while index < lines.len() && lines[index].trim_start().starts_with("#[") {
            index += 1;
            while index < lines.len() && lines[index].trim().is_empty() {
                index += 1;
            }
        }

        let mut depth = 0_i32;
        let mut saw_brace = false;
        while index < lines.len() {
            let line = lines[index];
            for ch in line.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        saw_brace = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }

            let trimmed = line.trim_end();
            index += 1;
            if saw_brace && depth <= 0 {
                break;
            }
            if !saw_brace && (trimmed.ends_with(';') || trimmed.ends_with(',')) {
                break;
            }
        }

        index
    }

    fn assert_no_forbidden_production_patterns(relative_dir: &str, forbidden_patterns: &[&str]) {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let layer_root = source_root.join(relative_dir);
        let mut files = Vec::new();
        collect_rs_files(&layer_root, &mut files);
        files.sort();

        assert!(
            !files.is_empty(),
            "no workflow source files found under {relative_dir}"
        );

        let mut violations = Vec::new();
        for file in files {
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|err| panic!("read source file {}: {err}", file.display()));
            let production = production_source(&content);
            for forbidden in forbidden_patterns {
                if production.contains(forbidden) {
                    let display_path = file.strip_prefix(&source_root).unwrap_or(&file);
                    violations.push(format!("{} -> {forbidden}", display_path.display()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "workflow production dependency violations:\n{}",
            violations.join("\n")
        );
    }

    fn assert_no_forbidden_production_patterns_in_file(
        relative_file: &str,
        forbidden_patterns: &[&str],
    ) {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let file = source_root.join(relative_file);
        let content = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("read source file {}: {err}", file.display()));
        let production = production_source_excluding_cfg_test_items(&content);
        let mut violations = Vec::new();

        for forbidden in forbidden_patterns {
            if production.contains(forbidden) {
                violations.push(format!("{relative_file} -> {forbidden}"));
            }
        }

        assert!(
            violations.is_empty(),
            "workflow production dependency violations:\n{}",
            violations.join("\n")
        );
    }

    fn assert_no_forbidden_production_patterns_in_all_sources(
        relative_dir: &str,
        forbidden_patterns: &[&str],
    ) {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let layer_root = source_root.join(relative_dir);
        let mut files = Vec::new();
        collect_rs_files(&layer_root, &mut files);
        files.sort();

        assert!(
            !files.is_empty(),
            "no source files found under {relative_dir}"
        );

        let mut violations = Vec::new();
        for file in files {
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|err| panic!("read source file {}: {err}", file.display()));
            let production = production_source_excluding_cfg_test_items(&content);
            for forbidden in forbidden_patterns {
                if production.contains(forbidden) {
                    let display_path = file.strip_prefix(&source_root).unwrap_or(&file);
                    violations.push(format!("{} -> {forbidden}", display_path.display()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "production dependency violations:\n{}",
            violations.join("\n")
        );
    }

    #[derive(Default)]
    struct FakeDefinitionRepository {
        definitions: Mutex<HashMap<String, WorkflowDefinition>>,
    }

    impl FakeDefinitionRepository {
        fn insert(&self, definition: WorkflowDefinition) {
            self.definitions
                .lock()
                .unwrap()
                .insert(definition.name.clone(), definition);
        }
    }

    impl WorkflowDefinitionRepository for FakeDefinitionRepository {
        fn list(&self, running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError> {
            let mut summaries = self
                .definitions
                .lock()
                .unwrap()
                .values()
                .map(|definition| WorkflowSummary {
                    name: definition.name.clone(),
                    description: definition.description.clone(),
                    builtin: definition.builtin,
                    is_running: running_names.contains(&definition.name),
                    source_format: crate::domain::workflow::WorkflowSourceFormat::Yaml,
                })
                .collect::<Vec<_>>();
            summaries.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(summaries)
        }

        fn get(&self, file_stem: &str) -> Result<Option<WorkflowDefinition>, WorkflowError> {
            Ok(self.definitions.lock().unwrap().get(file_stem).cloned())
        }

        fn save(
            &self,
            definition: WorkflowDefinition,
            _original_name: Option<&str>,
        ) -> Result<(), WorkflowError> {
            self.definitions
                .lock()
                .unwrap()
                .insert(definition.name.clone(), definition);
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<(), WorkflowError> {
            self.definitions.lock().unwrap().remove(name);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeDefinitionSourceGateway {
        sources: Mutex<HashMap<String, String>>,
        save_definition: Mutex<Option<WorkflowDefinition>>,
        save_error: Mutex<Option<String>>,
        saves: Mutex<Vec<(String, Option<String>)>>,
    }

    impl FakeDefinitionSourceGateway {
        fn insert_source(&self, file_stem: &str, source: &str) {
            self.sources
                .lock()
                .unwrap()
                .insert(file_stem.to_string(), source.to_string());
        }

        fn set_save_definition(&self, definition: WorkflowDefinition) {
            *self.save_definition.lock().unwrap() = Some(definition);
        }

        fn fail_saves(&self, message: &str) {
            *self.save_error.lock().unwrap() = Some(message.to_string());
        }

        fn saves(&self) -> Vec<(String, Option<String>)> {
            self.saves.lock().unwrap().clone()
        }
    }

    impl WorkflowDefinitionSourceGateway for FakeDefinitionSourceGateway {
        fn get_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
            Ok(self.sources.lock().unwrap().get(file_stem).cloned())
        }

        fn save_source(
            &self,
            source: &str,
            original_name: Option<&str>,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            self.saves
                .lock()
                .unwrap()
                .push((source.to_string(), original_name.map(str::to_string)));

            if let Some(message) = self.save_error.lock().unwrap().clone() {
                return Err(WorkflowError::external(message));
            }

            Ok(self
                .save_definition
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| workflow_definition("saved-workflow")))
        }
    }

    #[derive(Default)]
    struct FakeFacetRepository {
        facets: Mutex<HashMap<(FacetKind, String), String>>,
    }

    impl FacetRepository for FakeFacetRepository {
        fn list(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
            Ok(self
                .facets
                .lock()
                .unwrap()
                .keys()
                .filter(|(candidate, _)| *candidate == kind)
                .map(|(_, key)| key.clone())
                .collect())
        }

        fn get(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError> {
            self.facets
                .lock()
                .unwrap()
                .get(&(kind, key.to_string()))
                .cloned()
                .ok_or_else(|| WorkflowError::NotFound(key.to_string()))
        }

        fn save(
            &self,
            kind: FacetKind,
            key: &str,
            content: &str,
            _is_new: bool,
        ) -> Result<(), WorkflowError> {
            self.facets
                .lock()
                .unwrap()
                .insert((kind, key.to_string()), content.to_string());
            Ok(())
        }

        fn delete(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
            self.facets.lock().unwrap().remove(&(kind, key.to_string()));
            Ok(())
        }

        fn list_summaries(&self, _kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError> {
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct FakeEventRepository {
        events: Mutex<Vec<WorkflowEventDraft>>,
    }

    impl WorkflowEventRepository for FakeEventRepository {
        fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }

        fn read(
            &self,
            _execution_id: &WorkflowExecutionId,
        ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    struct NoopExecutionProjectionRepository;

    impl WorkflowExecutionProjectionRepository for NoopExecutionProjectionRepository {
        fn get_execution(
            &self,
            _execution_id: &WorkflowExecutionId,
        ) -> Result<Option<WorkflowExecution>, WorkflowError> {
            Ok(None)
        }
    }

    struct NoopArchiveRepository;

    impl WorkflowExecutionArchiveRepository for NoopArchiveRepository {
        fn archive_manual(
            &self,
            _execution_id: &WorkflowExecutionId,
            _archived_at: f64,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn restore_manual(
            &self,
            _execution_id: &WorkflowExecutionId,
            _restored_at: f64,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn manual_archive_snapshot_for(
            &self,
            _execution_ids: &[String],
        ) -> Result<crate::domain::workflow::WorkflowExecutionArchiveSnapshot, WorkflowError>
        {
            Ok(crate::domain::workflow::WorkflowExecutionArchiveSnapshot {
                records: Vec::new(),
            })
        }
    }

    struct FakeManagedWorktreeGateway;

    impl ManagedWorktreeGateway for FakeManagedWorktreeGateway {
        fn resolve(&self, worktree_path: &str) -> Result<String, WorkflowError> {
            if worktree_path == "reject" {
                return Err(WorkflowError::external("not managed"));
            }
            Ok(format!("/canonical/{worktree_path}"))
        }
    }

    #[derive(Default)]
    struct FakeExternalEditorGateway {
        opened: Mutex<Vec<String>>,
    }

    impl FakeExternalEditorGateway {
        fn opened(&self) -> Vec<String> {
            self.opened.lock().unwrap().clone()
        }
    }

    impl ExternalEditorGateway for FakeExternalEditorGateway {
        fn open_workflow(&self, name: &str) -> Result<(), WorkflowError> {
            self.opened.lock().unwrap().push(format!("workflow:{name}"));
            Ok(())
        }

        fn open_facet(&self, kind: &str, key: &str) -> Result<(), WorkflowError> {
            self.opened
                .lock()
                .unwrap()
                .push(format!("facet:{kind}:{key}"));
            Ok(())
        }
    }

    struct FakeDiagnosticsGateway;

    impl WorkflowDiagnosticsGateway for FakeDiagnosticsGateway {
        fn diagnose_all(&self) -> Result<serde_json::Value, WorkflowError> {
            Ok(serde_json::json!({"items": [], "workflowSummaries": {}}))
        }
    }

    struct FakeConfigPathGateway;

    impl WorkflowConfigPathGateway for FakeConfigPathGateway {
        fn automation_config_dir(&self) -> Result<String, WorkflowError> {
            Ok("/automation".to_string())
        }
    }

    struct FakeSecretSourceGateway;

    impl SecretSourceGateway for FakeSecretSourceGateway {
        fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError> {
            Ok(vec!["token-123".to_string()])
        }
    }

    struct Fixture {
        usecase: WorkflowUsecase,
        editors: Arc<FakeExternalEditorGateway>,
        definitions: Arc<FakeDefinitionRepository>,
        definition_sources: Arc<FakeDefinitionSourceGateway>,
        workspace_nodes: Arc<FakeWorkspaceTreeRepository>,
        _workspace_root: tempfile::TempDir,
    }

    #[derive(Default)]
    struct FakeWorkspaceTreeRepository {
        nodes: Mutex<HashMap<String, crate::domain::workspace_tree::WorkspaceTreeNode>>,
    }

    impl FakeWorkspaceTreeRepository {
        fn insert(
            &self,
            node_execution_id: &str,
            node: crate::domain::workspace_tree::WorkspaceTreeNode,
        ) {
            self.nodes
                .lock()
                .unwrap()
                .insert(node_execution_id.to_string(), node);
        }
    }

    impl crate::domain::workspace_tree::WorkspaceTreeRepository for FakeWorkspaceTreeRepository {
        fn load_node(
            &self,
            _workspace_identity: &crate::domain::workspace_tree::WorkspaceIdentity,
            _node_id: &str,
        ) -> Result<
            Option<crate::domain::workspace_tree::WorkspaceTreeNode>,
            crate::domain::local_event::LocalEventQueryError,
        > {
            Ok(None)
        }

        fn load_node_by_node_execution_id(
            &self,
            node_execution_id: &str,
        ) -> Result<
            Option<crate::domain::workspace_tree::WorkspaceTreeNode>,
            crate::domain::local_event::LocalEventQueryError,
        > {
            Ok(self.nodes.lock().unwrap().get(node_execution_id).cloned())
        }

        fn node_id_for_session(
            &self,
            _workspace_identity: &crate::domain::workspace_tree::WorkspaceIdentity,
            _session_id: &str,
        ) -> Result<Option<String>, crate::domain::local_event::LocalEventQueryError> {
            Ok(None)
        }
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_executions(Vec::new())
        }

        fn with_executions(executions: Vec<WorkflowExecutionSummary>) -> Self {
            Self::with_executions_and_definition_sources(
                executions,
                Arc::new(FakeDefinitionSourceGateway::default()),
            )
        }

        fn with_definition_sources(definition_sources: Arc<FakeDefinitionSourceGateway>) -> Self {
            Self::with_executions_and_definition_sources(Vec::new(), definition_sources)
        }

        fn with_executions_and_definition_sources(
            executions: Vec<WorkflowExecutionSummary>,
            definition_sources: Arc<FakeDefinitionSourceGateway>,
        ) -> Self {
            let definitions = Arc::new(FakeDefinitionRepository::default());
            let facets = Arc::new(FakeFacetRepository::default());
            let events = Arc::new(FakeEventRepository::default());
            let editors = Arc::new(FakeExternalEditorGateway::default());
            let workspace_nodes = Arc::new(FakeWorkspaceTreeRepository::default());
            let workspace_root = tempfile::tempdir().unwrap();
            let workspace_query =
                crate::usecase::workspace_tree::TestWorkspaceQueryService::new(executions);
            let query = WorkflowQueryService::new(
                definitions.clone(),
                definition_sources.clone(),
                facets.clone(),
                events.clone(),
                Arc::new(NoopExecutionProjectionRepository),
            );
            let usecase = WorkflowUsecase::new(
                query,
                definitions.clone(),
                definition_sources.clone(),
                facets.clone(),
                Arc::new(FakeManagedWorktreeGateway),
                editors.clone(),
                Arc::new(FakeDiagnosticsGateway),
                Arc::new(FakeConfigPathGateway),
                Arc::new(FakeSecretSourceGateway),
                Arc::new(NoopArchiveRepository),
                workspace_nodes.clone(),
                workspace_query,
            );
            Self {
                usecase,
                editors,
                definitions,
                definition_sources,
                workspace_nodes,
                _workspace_root: workspace_root,
            }
        }
    }

    fn workspace_node(
        node_execution_id: &str,
        execution_id: Option<&str>,
    ) -> crate::domain::workspace_tree::WorkspaceTreeNode {
        crate::domain::workspace_tree::WorkspaceTreeNode {
            id: format!("node:{node_execution_id}"),
            parent_id: execution_id.map(str::to_string),
            sibling_order: 0,
            kind: crate::domain::workspace_tree::WorkspaceNodeKind::WorkflowSession,
            title: "node".to_string(),
            status: crate::domain::workspace_tree::WorkspaceNodeStatus::Running,
            error_reason: None,
            updated_at_bits: 1.0_f64.to_bits(),
            execution_id: execution_id.map(str::to_string),
            node_execution_id: Some(node_execution_id.to_string()),
            node_name: Some("node".to_string()),
            attempt: Some(1),
            completion_signals: Default::default(),
            has_artifact: false,
            session_id: None,
            can_approve: false,
            can_retry: false,
            can_close: false,
            can_stop: false,
            can_resume: false,
            recovery_owner_reason: None,
            resume_unavailable_reason: None,
            can_abort: false,
            can_archive: false,
            display_command: None,
            command_result: None,
            dynamic_fanout: false,
        }
    }

    fn workflow_definition(name: &str) -> WorkflowDefinition {
        WorkflowDefinition {
            name: name.to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: Vec::new(),
            entry: "main".to_string(),
        }
    }

    fn execution_summary(
        execution_id: &str,
        worktree_path: &str,
        status: ExecutionStatus,
    ) -> WorkflowExecutionSummary {
        WorkflowExecutionSummary {
            execution_id: execution_id.to_string(),
            workflow_name: "wf".to_string(),
            status,
            worktree_path: worktree_path.to_string(),
            current_node: Some("node".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: Default::default(),
        }
    }

    #[test]
    fn resolve_worktree_path_delegates_to_managed_worktree_gateway() {
        let fixture = Fixture::new();

        assert_eq!(
            fixture.usecase.resolve_worktree_path("repo").unwrap(),
            "/canonical/repo"
        );
        assert!(fixture.usecase.resolve_worktree_path("reject").is_err());
    }

    #[test]
    fn list_executions_for_worktree_canonicalizes_path_before_querying_executions() {
        let executions = vec![execution_summary(
            "00000000-0000-0000-0000-000000000001",
            "/canonical/repo",
            ExecutionStatus::Running,
        )];
        let fixture = Fixture::with_executions(executions);

        let listed = fixture
            .usecase
            .list_executions_for_worktree(Some(ExecutionStatusFilter::Active), "repo")
            .unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].execution_id,
            "00000000-0000-0000-0000-000000000001"
        );
        assert!(fixture
            .usecase
            .list_executions_for_worktree(None, "reject")
            .is_err());
    }

    #[test]
    fn workflow_read_facade_owns_active_aggregation_filtering_and_dto_projection() {
        let executions = vec![execution_summary(
            "00000000-0000-0000-0000-000000000001",
            "/canonical/repo",
            ExecutionStatus::Running,
        )];
        let fixture = Fixture::with_executions(executions);
        fixture.definitions.insert(workflow_definition("idle"));
        fixture.definitions.insert(workflow_definition("wf"));
        let read = fixture.usecase.read_usecase();

        let workflows = read.list_workflow_summaries().unwrap();
        assert_eq!(workflows.len(), 2);
        assert_eq!(workflows[0].name, "idle");
        assert!(!workflows[0].is_running);
        assert_eq!(workflows[1].name, "wf");
        assert!(workflows[1].is_running);

        let active = read
            .list_executions_filtered(
                Some(ExecutionStatusFilter::Active),
                Some("repo"),
                WorkflowPageRequest::new(0, 10),
            )
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(
            active[0].execution_id,
            "00000000-0000-0000-0000-000000000001"
        );
        assert_eq!(active[0].worktree_path, "/canonical/repo");
    }

    #[test]
    fn get_workflow_source_returns_some_and_none_from_gateway() {
        let definition_sources = Arc::new(FakeDefinitionSourceGateway::default());
        definition_sources.insert_source("wf", "name: wf\n");
        let fixture = Fixture::with_definition_sources(definition_sources);

        assert_eq!(
            fixture.usecase.get_workflow_source("wf").unwrap(),
            Some("name: wf\n".to_string())
        );
        assert_eq!(
            fixture.usecase.get_workflow_source("missing").unwrap(),
            None
        );
    }

    #[test]
    fn save_workflow_source_returns_saved_definition_and_surfaces_gateway_errors() {
        let fixture = Fixture::new();
        fixture
            .definition_sources
            .set_save_definition(workflow_definition("saved-wf"));

        let saved = fixture
            .usecase
            .save_workflow_source("name: saved-wf\n", Some("old-wf"))
            .unwrap();

        assert_eq!(saved.name, "saved-wf");
        assert_eq!(
            fixture.definition_sources.saves(),
            vec![("name: saved-wf\n".to_string(), Some("old-wf".to_string()))]
        );

        fixture.definition_sources.fail_saves("save failed");

        assert!(fixture
            .usecase
            .save_workflow_source("name: failed-wf\n", None)
            .is_err());
    }

    #[test]
    fn authorize_execution_summary_for_worktree_hides_unmanaged_or_mismatched_runs() {
        let executions = vec![
            execution_summary(
                "00000000-0000-0000-0000-000000000011",
                "/canonical/repo",
                ExecutionStatus::Running,
            ),
            execution_summary(
                "00000000-0000-0000-0000-000000000012",
                "reject",
                ExecutionStatus::Running,
            ),
        ];
        let fixture = Fixture::with_executions(executions);

        let authorized = fixture
            .usecase
            .authorize_execution_summary_for_worktree(
                "00000000-0000-0000-0000-000000000011",
                "repo",
            )
            .unwrap();
        assert!(authorized.is_some());

        let mismatched = fixture
            .usecase
            .authorize_execution_summary_for_worktree(
                "00000000-0000-0000-0000-000000000011",
                "other",
            )
            .unwrap();
        assert!(mismatched.is_none());

        let unmanaged = fixture
            .usecase
            .authorize_execution_summary("00000000-0000-0000-0000-000000000012")
            .unwrap();
        assert!(unmanaged.is_none());

        fixture
            .usecase
            .authorize_execution_access_for_worktree("00000000-0000-0000-0000-000000000011", "repo")
            .unwrap();
        assert_eq!(
            fixture
                .usecase
                .authorize_execution_access_for_worktree(
                    "00000000-0000-0000-0000-000000000011",
                    "other",
                )
                .unwrap_err(),
            WorkflowError::external(
                "Workflow execution not found: 00000000-0000-0000-0000-000000000011"
            )
        );
        assert!(matches!(
            fixture
                .usecase
                .authorize_execution_access_for_worktree("invalid", "repo"),
            Err(WorkflowError::Validation(_))
        ));
    }

    #[test]
    fn authorize_node_execution_access_for_worktree_checks_identity_and_execution_ownership() {
        let execution_id = "00000000-0000-0000-0000-000000000011";
        let fixture = Fixture::with_executions(vec![execution_summary(
            execution_id,
            "/canonical/repo",
            ExecutionStatus::Running,
        )]);
        fixture.workspace_nodes.insert(
            "node-execution-1",
            workspace_node("node-execution-1", Some(execution_id)),
        );
        fixture.workspace_nodes.insert(
            "node-execution-without-owner",
            workspace_node("node-execution-without-owner", None),
        );

        fixture
            .usecase
            .authorize_node_execution_access_for_worktree("node-execution-1", "repo")
            .unwrap();
        assert!(matches!(
            fixture
                .usecase
                .authorize_node_execution_access_for_worktree("   ", "repo"),
            Err(WorkflowError::Validation(_))
        ));
        for node_execution_id in ["missing", "node-execution-without-owner"] {
            assert_eq!(
                fixture
                    .usecase
                    .authorize_node_execution_access_for_worktree(node_execution_id, "repo")
                    .unwrap_err(),
                WorkflowError::external(format!("Node execution not found: {node_execution_id}"))
            );
        }
        assert_eq!(
            fixture
                .usecase
                .authorize_node_execution_access_for_worktree("node-execution-1", "other")
                .unwrap_err(),
            WorkflowError::external(format!("Workflow execution not found: {execution_id}"))
        );
    }

    #[test]
    fn editor_commands_delegate_to_external_editor_gateway() {
        let fixture = Fixture::new();

        fixture
            .usecase
            .open_workflow_in_editor("custom-workflow")
            .unwrap();
        fixture
            .usecase
            .open_facet_in_editor(FacetKind::Instruction, "implement")
            .unwrap();

        assert_eq!(
            fixture.editors.opened(),
            vec![
                "workflow:custom-workflow".to_string(),
                "facet:instructions:implement".to_string(),
            ]
        );
    }

    #[test]
    fn diagnose_and_config_path_delegate_to_gateways() {
        let fixture = Fixture::new();

        let report = fixture.usecase.diagnose_all().unwrap();
        assert_eq!(report["items"].as_array().unwrap().len(), 0);
        assert_eq!(
            fixture.usecase.automation_config_dir().unwrap(),
            "/automation"
        );
    }

    #[test]
    fn workflow_domain_and_usecase_keep_dependencies_inward() {
        let outer_modules = [
            "adaptor",
            "agent_message_dispatcher",
            "app_data_dir",
            "cli",
            "cli_install",
            "config",
            "external_editor",
            "focus_tracker",
            "git",
            "git_host",
            "infrastructure",
            "mcp",
            "menu",
            "native_drop",
            "notion",
            "other",
            "path_aliases",
            "permission",
            "protocol",
            "pty",
            "qr_code",
            "review_comments",
            concat!("session", "_commands"),
            "shell_integration",
            "tls",
            "tray",
            "vpn_detect",
            "watcher",
            "webhook",
            "workflow",
            "workspace_state_store",
        ];
        let mut domain_forbidden = outer_modules.to_vec();
        domain_forbidden.push("usecase");

        assert_no_forbidden_crate_dependencies("domain/workflow", &domain_forbidden);
        assert_no_forbidden_crate_dependencies("usecase/workflow", &outer_modules);

        let external_dependency_patterns = [
            "tauri::",
            "git2::",
            "tokio::",
            "reqwest::",
            "sqlx::",
            "std::env::",
            "std::fs::",
            "std::net::",
            "std::process::",
            "use std::fs",
            "canonicalize(",
        ];
        assert_no_forbidden_production_patterns("domain/workflow", &external_dependency_patterns);
        assert_no_forbidden_production_patterns("usecase/workflow", &external_dependency_patterns);
        assert_no_forbidden_production_patterns(
            "usecase/repository_state",
            &external_dependency_patterns,
        );
    }

    #[test]
    fn workflow_execution_runtime_does_not_depend_on_lua_evaluation() {
        let forbidden_patterns = [
            concat!("infrastructure", "::", "lua"),
            "mlua::",
            "load_lua_workflow",
        ];

        assert_no_forbidden_production_patterns("domain/workflow", &forbidden_patterns);
        assert_no_forbidden_production_patterns("usecase/workflow", &forbidden_patterns);
        assert_no_forbidden_production_patterns(
            "adaptor/gateway/workflow/workflow_host",
            &forbidden_patterns,
        );
        assert_no_forbidden_production_patterns_in_file(
            "adaptor/gateway/workflow/workflow_host.rs",
            &forbidden_patterns,
        );
    }

    #[test]
    fn domain_and_usecase_production_do_not_depend_on_protocol_dto() {
        let forbidden_patterns = [
            concat!("crate", "::", "adaptor", "::", "protocol"),
            concat!("adaptor", "::", "protocol"),
        ];

        assert_no_forbidden_production_patterns_in_all_sources("domain", &forbidden_patterns);
        assert_no_forbidden_production_patterns_in_all_sources("usecase", &forbidden_patterns);
    }

    #[test]
    fn workflow_controller_production_uses_usecase_boundary_only() {
        let forbidden_patterns = [
            concat!("crate", "::", "adaptor", "::gateway", "::workflow"),
            concat!("adaptor", "::gateway", "::workflow"),
            concat!("crate", "::", "infrastructure", "::workflow"),
            concat!("infrastructure", "::workflow"),
        ];

        assert_no_forbidden_production_patterns(
            "adaptor/controller/command/workflow",
            &forbidden_patterns,
        );
        assert_no_forbidden_production_patterns(
            "adaptor/controller/command/agent_session",
            &[concat!(
                "crate",
                "::",
                "adaptor",
                "::",
                "controller",
                "::",
                "handler"
            )],
        );
    }

    #[test]
    fn agent_session_runtime_status_notification_uses_usecase_port() {
        assert_no_forbidden_production_patterns_in_file(
            "usecase/agent_session/agent_session_activity.rs",
            &[
                concat!("crate", "::", "adaptor", "::", "presenter"),
                concat!("crate", "::", "adaptor", "::", "gateway"),
            ],
        );
    }

    #[test]
    fn workflow_legacy_entrypoints_are_removed() {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for relative_path in [
            "workflow",
            concat!("infrastructure", "/workflow"),
            concat!("workflow_state", "_events.rs"),
            concat!("workflow_state", "_presenter.rs"),
            concat!("workflow_step", "_lifecycle.rs"),
            concat!("workflow_step", "_lifecycle_adapters.rs"),
            concat!("session", "_commands.rs"),
            concat!("protocol", "/workflow.rs"),
        ] {
            assert!(
                !source_root.join(relative_path).exists(),
                "{relative_path} must not remain as a workflow compatibility shim"
            );
        }
    }

    #[test]
    fn transport_legacy_entrypoints_are_removed() {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for relative_path in [
            "ws_server",
            concat!("ws_server", ".rs"),
            concat!("ws_bridge", ".rs"),
            "adaptor/controller/handler",
            concat!("agent_status", "_events.rs"),
        ] {
            assert!(
                !source_root.join(relative_path).exists(),
                "{relative_path} must not remain as a transport compatibility path"
            );
        }
    }
}
