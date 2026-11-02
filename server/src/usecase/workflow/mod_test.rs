use crate::domain::workflow::error::WorkflowError;
use crate::domain::workflow::gateway::ManagedWorktreeGateway;
use crate::domain::workflow::value_objects::definition::WorkflowDefinitionContent;
use crate::domain::workflow::value_objects::execution::ExecutionOrigin;
use crate::domain::workflow::value_objects::execution::ExecutionStatus;
use crate::domain::workflow::value_objects::execution::ExecutionTree;
use crate::domain::workflow::value_objects::execution_metadata::WorkflowExecutionSummary;
use crate::domain::workflow::value_objects::facet::FacetKind;
use crate::domain::workflow::WorkflowDefinitionSourceService;
use crate::usecase::workflow::ports::WorkflowDiagnosticsGateway;
use crate::usecase::workflow::ports::WorkflowDiagnosticsTarget;
use crate::usecase::workflow::query_service::WorkflowQueryService;
use crate::usecase::workflow::test_helpers::FakeDefinitionRepository;
use crate::usecase::workflow::test_helpers::NoopArchiveRepository;
use crate::usecase::workflow::test_helpers::{FakeEventRepository, FakeFacetRepository};
use crate::usecase::workflow::WorkflowUsecase;

use crate::domain::workflow::value_objects::ids::ExecutionTreeId;

use crate::usecase::workflow::ports::WorkflowEventDraft;
use crate::usecase::workflow::ports::WorkflowExecutionProjectionRepository;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Default)]
struct FakeDefinitionSourceGateway {
    sources: Mutex<HashMap<String, String>>,
    save_definition: Mutex<Option<WorkflowDefinitionContent>>,
    save_error: Mutex<Option<String>>,
    format_error: Mutex<Option<String>>,
    saves: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeDefinitionSourceGateway {
    fn insert_source(&self, file_stem: &str, source: &str) {
        self.sources
            .lock()
            .unwrap()
            .insert(file_stem.to_string(), source.to_string());
    }

    fn set_save_definition(&self, definition: WorkflowDefinitionContent) {
        *self.save_definition.lock().unwrap() = Some(definition);
    }

    fn fail_saves(&self, message: &str) {
        *self.save_error.lock().unwrap() = Some(message.to_string());
    }

    fn saves(&self) -> Vec<(String, Option<String>)> {
        self.saves.lock().unwrap().clone()
    }
}

impl WorkflowDefinitionSourceService for FakeDefinitionSourceGateway {
    fn get_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
        Ok(self.sources.lock().unwrap().get(file_stem).cloned())
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

struct NoopExecutionProjectionRepository;

#[async_trait::async_trait]
impl WorkflowExecutionProjectionRepository for NoopExecutionProjectionRepository {
    fn get_node_artifact_from_events(
        &self,
        _execution_id: &ExecutionTreeId,
        _node_name: &str,
        _events: &[WorkflowEventDraft],
    ) -> Result<Option<crate::domain::workflow::value_objects::execution::Artifact>, WorkflowError>
    {
        Ok(None)
    }

    async fn get_execution(
        &self,
        _execution_id: &ExecutionTreeId,
    ) -> Result<Option<ExecutionTree>, WorkflowError> {
        Ok(None)
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

#[derive(Default)]
struct FakeDiagnosticsGateway {
    targets: Mutex<Vec<WorkflowDiagnosticsTarget>>,
}

impl FakeDiagnosticsGateway {
    fn targets(&self) -> Vec<WorkflowDiagnosticsTarget> {
        self.targets.lock().unwrap().clone()
    }
}

impl WorkflowDiagnosticsGateway for FakeDiagnosticsGateway {
    fn diagnose_all(
        &self,
        target: WorkflowDiagnosticsTarget,
    ) -> Result<crate::usecase::workflow::diagnostic_dto::DiagnosticReport, WorkflowError> {
        self.targets.lock().unwrap().push(target);
        Ok(crate::usecase::workflow::diagnostic_dto::DiagnosticReport {
            items: vec![],
            workflow_summaries: Default::default(),
            facet_summaries: Default::default(),
            facet_usage: Default::default(),
        })
    }
}

struct Fixture {
    usecase: WorkflowUsecase,
    editors: Arc<FakeExternalEditorGateway>,
    definitions: Arc<FakeDefinitionRepository>,
    definition_sources: Arc<FakeDefinitionSourceGateway>,
    diagnostics: Arc<FakeDiagnosticsGateway>,
}

#[derive(Default)]
struct FakeWorkspaceTreeRepository;

#[async_trait::async_trait]
impl crate::domain::workspace_tree::repository::WorkspaceTreeRepository
    for FakeWorkspaceTreeRepository
{
    async fn load_trees(
        &self,
        workspace_identities: &[crate::domain::workspace_tree::value_objects::WorkspaceIdentity],
    ) -> Vec<Result<crate::domain::workspace_tree::entities::WorkspaceTree, WorkflowError>> {
        workspace_identities
            .iter()
            .map(|identity| {
                Ok(
                    crate::domain::workspace_tree::entities::WorkspaceTree::empty(
                        identity.as_str(),
                    ),
                )
            })
            .collect()
    }

    async fn load_execution_tree(
        &self,
        _: &crate::domain::workspace_tree::WorkspaceIdentity,
        _: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::WorkspaceTree>,
        crate::domain::workflow::WorkflowError,
    > {
        Ok(None)
    }

    async fn load_node_by_session_id(
        &self,
        _: &crate::domain::workspace_tree::value_objects::WorkspaceIdentity,
        _: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::value_objects::WorkspaceTreeNode>,
        crate::domain::local_event::query::LocalEventQueryError,
    > {
        Ok(None)
    }

    async fn load_node(
        &self,
        _workspace_identity: &crate::domain::workspace_tree::value_objects::WorkspaceIdentity,
        _node_id: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::value_objects::WorkspaceTreeNode>,
        crate::domain::local_event::query::LocalEventQueryError,
    > {
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
        todo!()
    }
}

fn workflow_definition(name: &str) -> WorkflowDefinitionContent {
    WorkflowDefinitionContent {
        name: name.to_string(),
        description: String::new(),
        schemas: Default::default(),
        nodes: Vec::new(),
        entry: "main".to_string(),
    }
}

async fn execution_summary(
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
        total_token_usage: Default::default(),
    }
}

#[test]
pub fn resolve_worktree_path_delegates_to_managed_worktree_gateway() {
    let fixture = Fixture::new();

    assert_eq!(
        fixture.usecase.resolve_worktree_path("repo").unwrap(),
        "/canonical/repo"
    );
    assert!(fixture.usecase.resolve_worktree_path("reject").is_err());
}

#[tokio::test]
pub async fn workflow_read_facade_owns_active_aggregation_filtering_and_dto_projection() {
    todo!()
}

#[test]
pub fn get_workflow_source_returns_some_and_none_from_gateway() {
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
pub fn test_workflow読取_定義不在を返す() {
    todo!()
}

#[test]
pub fn test_workflow読取_定義読取失敗を不在と区別する() {
    todo!()
}

#[test]
pub fn test_workflow読取_形式読取失敗を既定形式と区別する() {
    todo!()
}

#[test]
pub fn save_workflow_source_returns_saved_definition_and_surfaces_gateway_errors() {
    todo!()
}

#[test]
pub fn editor_commands_delegate_to_external_editor_gateway() {
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
pub fn test_診断usecase_指定directoryをgatewayへ渡す() {
    // Given
    let fixture = Fixture::new();
    let path = std::path::PathBuf::from("/tmp/custom-workflows");

    // When
    let report = fixture
        .usecase
        .diagnose_all(WorkflowDiagnosticsTarget::Directory(path.clone()))
        .unwrap();

    // Then
    assert!(report.items.is_empty());
    assert_eq!(
        fixture.diagnostics.targets(),
        vec![WorkflowDiagnosticsTarget::Directory(path)]
    );
}

#[test]
pub fn test_診断usecase_適用済みdirectoryをgatewayへ渡す() {
    // Given
    let fixture = Fixture::new();

    // When
    let report = fixture
        .usecase
        .diagnose_all(WorkflowDiagnosticsTarget::AppliedConfigDirectory)
        .unwrap();

    // Then
    assert!(report.items.is_empty());
    assert_eq!(
        fixture.diagnostics.targets(),
        vec![WorkflowDiagnosticsTarget::AppliedConfigDirectory]
    );
}

struct UnexpectedFailures;
impl crate::domain::failure::FailureRecordRepository for UnexpectedFailures {
    fn record_observed(
        &self,
        _: &crate::domain::failure::FailureKey,
        _: crate::domain::failure::WorkFailure,
        _: bool,
    ) -> bool {
        panic!("unexpected failure observation")
    }
    fn record_resolved(&self, _: &crate::domain::failure::FailureKey) -> bool {
        panic!("unexpected failure resolution")
    }
    fn attention_messages(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}
