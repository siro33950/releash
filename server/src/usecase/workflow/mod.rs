//! Workflow command usecases.
//!
//! This layer orchestrates domain aggregates through repository/gateway ports.
//! Controllers, CLI adapters, and watchers should converge
//! here as the legacy `workflow` module is removed.

pub(crate) mod command;
pub(crate) mod control_plane;
mod definition;
pub(crate) mod delegate;
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
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support;
mod workspace_node_command;
pub(crate) mod workspace_tree;

pub mod diagnostic_dto;
pub mod summary_query;
pub use summary_query::{
    FacetQueryService, FacetSummaryDto, WorkflowDefinitionQueryService, WorkflowSummaryDto,
};

use crate::domain::external_editor::{EditorLauncherGateway, EditorSettingsGateway};
use crate::domain::workflow::{
    ExecutionStatusFilter, ExecutionTree, ExecutionTreeArchiveRepository, FacetKind,
    FacetRepository, ManagedWorktreeGateway, WorkflowDefinition, WorkflowDefinitionRepository,
    WorkflowDirectoryService, WorkflowError,
};
use crate::usecase::workflow::ports::{WorkflowDiagnosticsGateway, WorkflowDiagnosticsTarget};

use definition::WorkflowDefinitionUsecase;
use facet::WorkflowFacetUsecase;
pub(crate) use output::WorkflowOutputUsecase;
pub use query_service::WorkflowGetOutputResult;
use query_service::WorkflowQueryService;
pub use runtime_command::WorkflowRuntimeUsecase;
pub(crate) use workspace_node_command::{
    ApproveWorkspaceNodeCommand, RenameWorkspaceSessionNodeCommand,
    ResumeWorkspaceSessionNodeCommand, RetryWorkspaceNodeCommand, WorkspaceNodeActionResolver,
    WorkspaceNodeCommandUsecase, WorkspaceNodeWorkflowCommandExecutor,
};
pub(crate) use workspace_tree::{
    NodeWorktreeDto, WorkspaceCommandNodeContentDto, WorkspaceCommandResultDto,
    WorkspaceNodeAttemptDto, WorkspaceNodeCapabilitiesDto, WorkspaceNodeContentDto,
    WorkspaceNodeDetailDto, WorkspaceSessionNodeContentDto,
};

/// 定義と Facet の操作、および workflow の読み取りの入口。
#[derive(Clone)]
pub struct WorkflowUsecase {
    query: WorkflowQueryService,
    definition_commands: WorkflowDefinitionUsecase,
    facet_commands: WorkflowFacetUsecase,
    output: WorkflowOutputUsecase,
    worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
    execution_archives: std::sync::Arc<dyn ExecutionTreeArchiveRepository>,
    workspace_nodes: std::sync::Arc<dyn crate::domain::workspace_tree::WorkspaceTreeRepository>,
    workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    /// 一覧に出す worktree ごとの、最後に読めた実行木と直近の失敗。
    retained_trees: std::sync::Arc<
        parking_lot::Mutex<
            std::collections::HashMap<
                String,
                crate::usecase::fetched::Fetched<crate::domain::workspace_tree::WorkspaceTree>,
            >,
        >,
    >,
    diagnostics: std::sync::Arc<dyn WorkflowDiagnosticsGateway>,
    definition_query: std::sync::Arc<dyn WorkflowDefinitionQueryService>,
    facet_query: std::sync::Arc<dyn FacetQueryService>,
}

impl WorkflowUsecase {
    /// 操作と読み取りに使う依存先を受け取る。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        query: WorkflowQueryService,
        definition_query: std::sync::Arc<dyn WorkflowDefinitionQueryService>,
        facet_query: std::sync::Arc<dyn FacetQueryService>,
        definitions: std::sync::Arc<dyn WorkflowDefinitionRepository>,
        facets: std::sync::Arc<dyn FacetRepository>,
        worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
        editor_settings: std::sync::Arc<dyn EditorSettingsGateway>,
        editor_launcher: std::sync::Arc<dyn EditorLauncherGateway>,
        directory_service: std::sync::Arc<dyn WorkflowDirectoryService>,
        diagnostics: std::sync::Arc<dyn WorkflowDiagnosticsGateway>,
        execution_archives: std::sync::Arc<dyn ExecutionTreeArchiveRepository>,
        workspace_nodes: std::sync::Arc<dyn crate::domain::workspace_tree::WorkspaceTreeRepository>,
        workspace_query: std::sync::Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    ) -> Self {
        let definition_commands = WorkflowDefinitionUsecase::new(
            definitions.clone(),
            editor_settings.clone(),
            editor_launcher.clone(),
            directory_service,
        );
        let facet_commands =
            WorkflowFacetUsecase::new(facets.clone(), editor_settings, editor_launcher);
        let output = WorkflowOutputUsecase::new(query.clone());
        Self {
            query,
            definition_commands,
            facet_commands,
            output,
            worktrees,
            execution_archives,
            workspace_nodes,
            workspace_query,
            retained_trees: Default::default(),
            diagnostics,
            definition_query,
            facet_query,
        }
    }

    /// 実行状況を取得し、定義一覧の読み取りへ渡す。
    pub async fn list_workflow_summaries(&self) -> Result<Vec<WorkflowSummaryDto>, WorkflowError> {
        let running = self
            .workspace_query
            .execution_summaries(None, Some(ExecutionStatusFilter::Active))
            .await?;
        self.definition_query.list_workflows(&running)
    }

    /// 指定した実行木の状態を取得する。
    pub async fn get_execution_state(
        &self,
        execution_id: &str,
    ) -> Result<Option<ExecutionTree>, WorkflowError> {
        self.query.get_execution_state(execution_id).await
    }

    pub fn resolve_worktree_path(&self, worktree_path: &str) -> Result<String, WorkflowError> {
        self.worktrees.resolve(worktree_path)
    }

    /// 定義を取得し、存在しない場合は空を返す。
    pub fn get_workflow(
        &self,
        file_stem: &str,
    ) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        self.query.get_workflow(file_stem)
    }

    /// 指定した種類の Facet 一覧を取得する。
    pub fn list_facet_summaries(
        &self,
        kind: FacetKind,
    ) -> Result<Vec<FacetSummaryDto>, WorkflowError> {
        self.facet_query.list_facet_summaries(kind)
    }

    pub fn open_workflow_in_editor(&self, name: &str) -> Result<(), WorkflowError> {
        self.definition_commands.open_in_editor(name)
    }

    /// サーバの定義フォルダを外部エディタで開く。
    pub fn open_workflow_directory_in_editor(&self) -> Result<(), WorkflowError> {
        self.definition_commands.open_directory_in_editor()
    }

    pub fn open_facet_in_editor(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        self.facet_commands.open_in_editor(kind, key)
    }

    /// 指定した対象の診断を取得する。
    pub fn diagnose_all(
        &self,
        target: WorkflowDiagnosticsTarget,
    ) -> Result<diagnostic_dto::DiagnosticReport, WorkflowError> {
        self.diagnostics.diagnose_all(target)
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
}

#[cfg(test)]
pub(crate) use crate::usecase::workflow::test_helpers::NoopArchiveRepository;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_helpers;

#[cfg(test)]
#[path = "mod_test.rs"]
pub(crate) mod mod_tests;
