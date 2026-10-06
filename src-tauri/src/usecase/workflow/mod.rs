//! Workflow command usecases.
//!
//! This layer orchestrates domain aggregates through repository/gateway ports.
//! Controllers, CLI adapters, and watchers should converge
//! here as the legacy `workflow` module is removed.

pub(crate) mod command;
pub(crate) mod control_plane;
mod definition;
pub(crate) mod delegate;
pub(crate) mod dto;
pub(crate) mod event_draft;
pub(crate) mod execution_archive;
pub(crate) mod facet;
pub(crate) mod node_startup;
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
pub(crate) mod startup;
#[cfg(test)]
pub(crate) mod test_support;
mod workspace_node_command;
pub(crate) mod workspace_tree;

pub mod diagnostic_dto;

use serde_json::Value;

use crate::domain::workflow::{
    ExecutionStatusFilter, ExecutionTree, ExecutionTreeArchiveRepository, FacetKind,
    FacetRepository, FacetSummary, ManagedWorktreeGateway, SecretSourceGateway, WorkflowDefinition,
    WorkflowDefinitionRepository, WorkflowError, WorkflowExecutionSummary, WorkflowPageRequest,
};
use crate::usecase::workflow::ports::{
    ExternalEditorGateway, WorkflowDefinitionSourceGateway, WorkflowDiagnosticsGateway,
    WorkflowDiagnosticsTarget, WorkflowSourceSaveError,
};

use definition::WorkflowDefinitionUsecase;
use facet::WorkflowFacetUsecase;
pub(crate) use output::WorkflowOutputUsecase;
pub use output::WorkflowValidateOutputResult;
use query_service::WorkflowQueryService;
pub use query_service::{WorkflowEventView, WorkflowGetOutputResult};
pub use runtime_command::WorkflowRuntimeUsecase;
pub(crate) use workspace_node_command::{
    ApproveWorkspaceNodeCommand, RenameWorkspaceSessionNodeCommand,
    ResumeWorkspaceSessionNodeCommand, RetryWorkspaceNodeCommand, WorkspaceNodeActionResolver,
    WorkspaceNodeCommandUsecase, WorkspaceNodeWorkflowCommandExecutor,
};
pub(crate) use workspace_tree::{
    NodeWorktreeDto, WorkspaceCommandNodeContentDto, WorkspaceCommandResultDto,
    WorkspaceNodeCapabilitiesDto, WorkspaceNodeContentDto, WorkspaceNodeDetailDto,
    WorkspaceSessionNodeContentDto,
};

#[derive(Clone)]
pub struct WorkflowReadUsecase {
    query: WorkflowQueryService,
    workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    output: WorkflowOutputUsecase,
    worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
    diagnostics: std::sync::Arc<dyn WorkflowDiagnosticsGateway>,
}

impl WorkflowReadUsecase {
    pub(crate) fn new(
        query: WorkflowQueryService,
        worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
        secrets: std::sync::Arc<dyn SecretSourceGateway>,
        workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
        diagnostics: std::sync::Arc<dyn WorkflowDiagnosticsGateway>,
    ) -> Self {
        Self {
            output: WorkflowOutputUsecase::new(query.clone(), secrets),
            query,
            worktrees,
            workspace_query,
            diagnostics,
        }
    }

    pub fn diagnose_all(
        &self,
        target: WorkflowDiagnosticsTarget,
    ) -> Result<diagnostic_dto::DiagnosticReport, WorkflowError> {
        self.diagnostics.diagnose_all(target)
    }

    pub async fn list_workflow_summaries(
        &self,
    ) -> Result<Vec<dto::WorkflowSummaryDto>, WorkflowError> {
        let running_names = self
            .workspace_query
            .execution_summaries(None, Some(ExecutionStatusFilter::Active), None)
            .await?
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

    pub async fn list_executions_filtered(
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
            .await
            .map(|executions| {
                executions
                    .into_iter()
                    .map(dto::workflow_execution_summary_to_dto)
                    .collect()
            })
    }

    pub(crate) async fn get_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        self.workspace_query.execution_summary(execution_id).await
    }

    pub(crate) async fn get_execution_log_page(
        &self,
        execution_id: &str,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventView>, WorkflowError> {
        if self.get_execution(execution_id).await?.is_none() {
            return Err(WorkflowError::NotFound(format!(
                "Workflow execution not found: {execution_id}"
            )));
        }
        self.query.get_execution_log_page(execution_id, page).await
    }

    pub async fn get_execution_state(
        &self,
        execution_id: &str,
    ) -> Result<Option<ExecutionTree>, WorkflowError> {
        self.query.get_execution_state(execution_id).await
    }

    pub(crate) async fn validate_output_for_contract(
        &self,
        execution_id: &str,
        node_name: &str,
        contract: &str,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        self.output
            .validate_output_for_contract(execution_id, node_name, contract, structured_output)
            .await
    }

    pub async fn get_output(
        &self,
        execution_id: &str,
        node_name: &str,
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        self.output.get_output(execution_id, node_name).await
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
    execution_archives: std::sync::Arc<dyn ExecutionTreeArchiveRepository>,
    workspace_nodes: std::sync::Arc<dyn crate::domain::workspace_tree::WorkspaceTreeRepository>,
    workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    failures: std::sync::Arc<dyn crate::domain::failure::FailureRecordRepository>,
    /// 一覧に出す worktree ごとの、最後に読めた実行木と直近の失敗。
    retained_trees: std::sync::Arc<
        parking_lot::Mutex<
            std::collections::HashMap<
                String,
                crate::usecase::fetched::Fetched<crate::domain::workspace_tree::WorkspaceTree>,
            >,
        >,
    >,
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
        secrets: std::sync::Arc<dyn SecretSourceGateway>,
        execution_archives: std::sync::Arc<dyn ExecutionTreeArchiveRepository>,
        workspace_nodes: std::sync::Arc<dyn crate::domain::workspace_tree::WorkspaceTreeRepository>,
        workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
        failures: std::sync::Arc<dyn crate::domain::failure::FailureRecordRepository>,
    ) -> Self {
        let definition_commands = WorkflowDefinitionUsecase::new(definitions, definition_sources);
        let facet_commands = WorkflowFacetUsecase::new(facets.clone());
        let output = WorkflowOutputUsecase::new(query.clone(), secrets);
        let read = WorkflowReadUsecase {
            query: query.clone(),
            output: output.clone(),
            worktrees: worktrees.clone(),
            workspace_query: workspace_query.clone(),
            diagnostics,
        };
        Self {
            query,
            definition_commands,
            facet_commands,
            output,
            worktrees,
            editors,
            execution_archives,
            workspace_nodes,
            workspace_query,
            failures,
            retained_trees: Default::default(),
            read,
        }
    }

    pub fn read_usecase(&self) -> WorkflowReadUsecase {
        self.read.clone()
    }

    pub async fn get_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        self.workspace_query.execution_summary(execution_id).await
    }

    pub async fn authorize_execution_summary(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        let Some(summary) = self.get_execution(execution_id).await? else {
            return Ok(None);
        };
        match self.resolve_worktree_path(&summary.worktree_path) {
            Ok(_) => Ok(Some(summary)),
            Err(_) => Ok(None),
        }
    }

    pub async fn authorize_execution_summary_for_worktree(
        &self,
        execution_id: &str,
        worktree_path: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        let canonical = self.resolve_worktree_path(worktree_path)?;
        let Some(summary) = self.authorize_execution_summary(execution_id).await? else {
            return Ok(None);
        };
        if summary.worktree_path == canonical {
            Ok(Some(summary))
        } else {
            Ok(None)
        }
    }

    pub async fn authorize_execution_access_for_worktree(
        &self,
        execution_id: &str,
        worktree_path: &str,
    ) -> Result<(), WorkflowError> {
        let execution_id = crate::domain::workflow::ExecutionTreeId::new(execution_id.to_string())?;
        if self
            .authorize_execution_summary_for_worktree(execution_id.as_str(), worktree_path)
            .await?
            .is_some()
        {
            Ok(())
        } else {
            Err(WorkflowError::external(format!(
                "Workflow execution not found: {execution_id}"
            )))
        }
    }

    pub async fn authorize_node_execution_access_for_worktree(
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
            .await
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::external(format!("Node execution not found: {node_execution_id}"))
            })?;
        let execution_id = node.execution_id.ok_or_else(|| {
            WorkflowError::external(format!("Node execution not found: {node_execution_id}"))
        })?;
        self.authorize_execution_access_for_worktree(&execution_id, worktree_path)
            .await
    }

    pub fn resolve_worktree_path(&self, worktree_path: &str) -> Result<String, WorkflowError> {
        self.worktrees.resolve(worktree_path)
    }

    pub fn get_workflow_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
        self.query.get_workflow_source(file_stem)
    }

    pub fn get_workflow_dto(
        &self,
        file_stem: &str,
    ) -> Result<Option<dto::WorkflowDto>, WorkflowError> {
        let Some(workflow) = self.query.get_workflow(file_stem)? else {
            return Ok(None);
        };
        let format = self.query.get_workflow_source_format(file_stem)?;
        Ok(Some(dto::workflow_to_dto_with_source_format(
            &workflow, format,
        )))
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

    #[cfg(any(test, feature = "test-support"))]
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

    pub fn diagnose_all(
        &self,
        target: WorkflowDiagnosticsTarget,
    ) -> Result<diagnostic_dto::DiagnosticReport, WorkflowError> {
        self.read.diagnose_all(target)
    }

    pub fn render_facet_preview(
        &self,
        content: &str,
        sample_values: &std::collections::HashMap<String, String>,
    ) -> String {
        self.facet_commands
            .render_facet_preview(content, sample_values)
    }

    pub async fn validate_output(
        &self,
        execution_id: &str,
        node_name: &str,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        self.output
            .validate_output(execution_id, node_name, structured_output)
            .await
    }

    pub async fn get_output(
        &self,
        execution_id: &str,
        node_name: &str,
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        self.output.get_output(execution_id, node_name).await
    }
}

#[cfg(feature = "test-support")]
pub(crate) use workspace_node_command::{
    WorkspaceNodeApprovalTarget, WorkspaceNodeRetryTarget, WorkspaceSessionNodeRenameTarget,
};

#[cfg(feature = "test-support")]
impl WorkflowUsecase {
    pub fn test_replace_managed_worktree_gateway(
        &mut self,
        worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
    ) {
        self.worktrees = worktrees;
    }

    pub fn test_replace_workspace_tree_repository(
        &mut self,
        workspace_nodes: std::sync::Arc<dyn crate::domain::workspace_tree::WorkspaceTreeRepository>,
    ) {
        self.workspace_nodes = workspace_nodes;
    }

    pub fn test_replace_failure_repository(
        &mut self,
        failures: std::sync::Arc<dyn crate::domain::failure::FailureRecordRepository>,
    ) {
        self.failures = failures;
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_helpers_archive;
#[cfg(test)]
pub(crate) use test_helpers_archive::NoopArchiveRepository;

#[cfg(any(test, feature = "test-support"))]
#[path = "test_helpers_shared_fixtures.rs"]
pub(crate) mod shared_test_helpers;
