use crate::domain::workflow::error::WorkflowError;
use crate::domain::workflow::gateway::ManagedWorktreeGateway;
use crate::domain::workflow::value_objects::definition::WorkflowDefinition;
use crate::domain::workflow::value_objects::execution::ExecutionOrigin;
use crate::domain::workflow::value_objects::execution::ExecutionStatus;
use crate::domain::workflow::value_objects::execution::ExecutionTree;
use crate::domain::workflow::value_objects::execution_metadata::ExecutionStatusFilter;
use crate::domain::workflow::value_objects::execution_metadata::WorkflowExecutionSummary;
use crate::domain::workflow::value_objects::execution_metadata::WorkflowPageRequest;
use crate::domain::workflow::value_objects::facet::FacetKind;
use crate::usecase::workflow::ports::ExternalEditorGateway;
use crate::usecase::workflow::ports::WorkflowDefinitionSourceGateway;
use crate::usecase::workflow::ports::WorkflowDiagnosticsGateway;
use crate::usecase::workflow::ports::WorkflowDiagnosticsTarget;
use crate::usecase::workflow::query_service::WorkflowQueryService;
use crate::usecase::workflow::test_helpers::FakeDefinitionRepository;
use crate::usecase::workflow::test_helpers::NoopArchiveRepository;
use crate::usecase::workflow::test_helpers::{
    FakeEventRepository, FakeFacetRepository, FakeSecretSourceGateway,
};
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
    save_definition: Mutex<Option<WorkflowDefinition>>,
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
    fn source_format(
        &self,
        _: &str,
    ) -> Result<
        crate::domain::workflow::value_objects::definition::WorkflowSourceFormat,
        WorkflowError,
    > {
        if let Some(message) = self.format_error.lock().unwrap().as_ref() {
            return Err(WorkflowError::external(message));
        }
        Ok(crate::domain::workflow::value_objects::definition::WorkflowSourceFormat::Yaml)
    }

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
    workspace_nodes: Arc<FakeWorkspaceTreeRepository>,
}

#[derive(Default)]
struct FakeWorkspaceTreeRepository {
    nodes: Mutex<HashMap<String, crate::domain::workspace_tree::value_objects::WorkspaceTreeNode>>,
}

impl FakeWorkspaceTreeRepository {
    fn insert(
        &self,
        node_execution_id: &str,
        node: crate::domain::workspace_tree::value_objects::WorkspaceTreeNode,
    ) {
        self.nodes
            .lock()
            .unwrap()
            .insert(node_execution_id.to_string(), node);
    }
}

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

    async fn load_node_by_node_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::value_objects::WorkspaceTreeNode>,
        crate::domain::local_event::query::LocalEventQueryError,
    > {
        Ok(self.nodes.lock().unwrap().get(node_execution_id).cloned())
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
        let diagnostics = Arc::new(FakeDiagnosticsGateway::default());
        let workspace_nodes = Arc::new(FakeWorkspaceTreeRepository::default());
        let workspace_query =
            crate::usecase::workspace_tree::test_support::TestWorkspaceQueryService::new(
                executions,
            );
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
            diagnostics.clone(),
            Arc::new(FakeSecretSourceGateway),
            Arc::new(NoopArchiveRepository),
            workspace_nodes.clone(),
            workspace_query,
            Arc::new(UnexpectedFailures),
        );
        Self {
            usecase,
            editors,
            definitions,
            definition_sources,
            diagnostics,
            workspace_nodes,
        }
    }
}

fn workspace_node(
    node_execution_id: &str,
    execution_id: Option<&str>,
) -> crate::domain::workspace_tree::value_objects::WorkspaceTreeNode {
    crate::domain::workspace_tree::value_objects::WorkspaceTreeNode {
        process_presence: Default::default(),
        can_resume_session: false,
        worktree: None,
        id: format!("node:{node_execution_id}"),
        parent_id: execution_id.map(str::to_string),
        sibling_order: 0,
        kind: crate::domain::workspace_tree::value_objects::WorkspaceNodeKind::WorkflowSession,
        title: "node".to_string(),
        status: crate::domain::workspace_tree::value_objects::WorkspaceNodeStatus::Running,
        status_classification:
            crate::domain::workspace_tree::value_objects::WorkspaceNodeStatusClassification::Active,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: Some(crate::domain::workflow::value_objects::node_fact::AgentSessionActivity::AwaitingInstruction),
        error_reason: None,
        updated_at_bits: 1.0_f64.to_bits(),
        execution_id: execution_id.map(str::to_string),
        node_execution_id: Some(node_execution_id.to_string()),
        node_name: Some("node".to_string()),
        attempt: Some(1),
        retry_predecessor_id: None,
        past_attempt_ids: Vec::new(),
        is_retry_history: false,
        completion_signals: Default::default(),
        has_artifact: false,
        session_id: None,
        can_rename: false,
        can_approve: false,
        can_retry: false,
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
    let executions = vec![
        execution_summary(
            "00000000-0000-0000-0000-000000000001",
            "/canonical/repo",
            ExecutionStatus::Running,
        )
        .await,
    ];
    let fixture = Fixture::with_executions(executions);
    fixture.definitions.insert(workflow_definition("idle"));
    fixture.definitions.insert(workflow_definition("wf"));
    let read = fixture.usecase.read_usecase();

    let workflows = read.list_workflow_summaries().await.unwrap();
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
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(
        active[0].execution_id,
        "00000000-0000-0000-0000-000000000001"
    );
    assert_eq!(active[0].worktree_path, "/canonical/repo");
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
    // Given
    let fixture = Fixture::new();
    // When
    let result = fixture.usecase.get_workflow_dto("missing");
    // Then
    assert!(result.unwrap().is_none());
}

#[test]
pub fn test_workflow読取_定義読取失敗を不在と区別する() {
    // Given
    let fixture = Fixture::new();
    *fixture.definitions.read_error.lock().unwrap() = Some("definition unreadable".into());
    // When
    let result = fixture.usecase.get_workflow_dto("missing");
    // Then
    assert!(
        matches!(result, Err(WorkflowError::External(message)) if message == "definition unreadable")
    );
}

#[test]
pub fn test_workflow読取_形式読取失敗を既定形式と区別する() {
    // Given
    let fixture = Fixture::new();
    fixture.definitions.insert(workflow_definition("present"));
    *fixture.definition_sources.format_error.lock().unwrap() = Some("format unreadable".into());
    // When
    let result = fixture.usecase.get_workflow_dto("present");
    // Then
    assert!(
        matches!(result, Err(WorkflowError::External(message)) if message == "format unreadable")
    );
}

#[test]
pub fn save_workflow_source_returns_saved_definition_and_surfaces_gateway_errors() {
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

#[tokio::test]
pub async fn authorize_execution_summary_for_worktree_hides_unmanaged_or_mismatched_runs() {
    let executions = vec![
        execution_summary(
            "00000000-0000-0000-0000-000000000011",
            "/canonical/repo",
            ExecutionStatus::Running,
        )
        .await,
        execution_summary(
            "00000000-0000-0000-0000-000000000012",
            "reject",
            ExecutionStatus::Running,
        )
        .await,
    ];
    let fixture = Fixture::with_executions(executions);

    let authorized = fixture
        .usecase
        .authorize_execution_summary_for_worktree("00000000-0000-0000-0000-000000000011", "repo")
        .await
        .unwrap();
    assert!(authorized.is_some());

    let mismatched = fixture
        .usecase
        .authorize_execution_summary_for_worktree("00000000-0000-0000-0000-000000000011", "other")
        .await
        .unwrap();
    assert!(mismatched.is_none());

    let unmanaged = fixture
        .usecase
        .authorize_execution_summary("00000000-0000-0000-0000-000000000012")
        .await
        .unwrap();
    assert!(unmanaged.is_none());

    fixture
        .usecase
        .authorize_execution_access_for_worktree("00000000-0000-0000-0000-000000000011", "repo")
        .await
        .unwrap();
    assert_eq!(
        fixture
            .usecase
            .authorize_execution_access_for_worktree(
                "00000000-0000-0000-0000-000000000011",
                "other",
            )
            .await
            .unwrap_err(),
        WorkflowError::external(
            "Workflow execution not found: 00000000-0000-0000-0000-000000000011"
        )
    );
    assert!(matches!(
        fixture
            .usecase
            .authorize_execution_access_for_worktree("invalid", "repo")
            .await,
        Err(WorkflowError::Validation(_))
    ));
}

#[tokio::test]
pub async fn authorize_node_execution_access_for_worktree_checks_identity_and_execution_ownership()
{
    let execution_id = "00000000-0000-0000-0000-000000000011";
    let fixture = Fixture::with_executions(vec![
        execution_summary(execution_id, "/canonical/repo", ExecutionStatus::Running).await,
    ]);
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
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .usecase
            .authorize_node_execution_access_for_worktree("   ", "repo")
            .await,
        Err(WorkflowError::Validation(_))
    ));
    for node_execution_id in ["missing", "node-execution-without-owner"] {
        assert_eq!(
            fixture
                .usecase
                .authorize_node_execution_access_for_worktree(node_execution_id, "repo")
                .await
                .unwrap_err(),
            WorkflowError::external(format!("Node execution not found: {node_execution_id}"))
        );
    }
    assert_eq!(
        fixture
            .usecase
            .authorize_node_execution_access_for_worktree("node-execution-1", "other")
            .await
            .unwrap_err(),
        WorkflowError::external(format!("Workflow execution not found: {execution_id}"))
    );
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
