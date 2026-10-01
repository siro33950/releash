use super::*;
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionStatus, ExecutionStatusFilter, ExecutionTree, ExecutionTreeId,
    WorkflowSummary,
};
use crate::usecase::workflow::ports::{
    ExternalEditorGateway, WorkflowDiagnosticsGateway, WorkflowDiagnosticsTarget,
    WorkflowEventDraft, WorkflowEventRepository, WorkflowExecutionProjectionRepository,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct FakeDefinitionRepository {
    definitions: Mutex<HashMap<String, WorkflowDefinition>>,
    read_error: Mutex<Option<String>>,
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
                failure: None,
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
        if let Some(message) = self.read_error.lock().unwrap().as_ref() {
            return Err(WorkflowError::external(message));
        }
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
    ) -> Result<crate::domain::workflow::WorkflowSourceFormat, WorkflowError> {
        if let Some(message) = self.format_error.lock().unwrap().as_ref() {
            return Err(WorkflowError::external(message));
        }
        Ok(crate::domain::workflow::WorkflowSourceFormat::Yaml)
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

#[async_trait::async_trait]
impl WorkflowEventRepository for FakeEventRepository {
    fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }

    async fn read(
        &self,
        _execution_id: &ExecutionTreeId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        Ok(self.events.lock().unwrap().clone())
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
    ) -> Result<Option<crate::domain::workflow::Artifact>, WorkflowError> {
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
    ) -> Result<diagnostic_dto::DiagnosticReport, WorkflowError> {
        self.targets.lock().unwrap().push(target);
        Ok(diagnostic_dto::DiagnosticReport {
            items: vec![],
            workflow_summaries: Default::default(),
            facet_summaries: Default::default(),
            facet_usage: Default::default(),
        })
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
    diagnostics: Arc<FakeDiagnosticsGateway>,
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

#[async_trait::async_trait]
impl crate::domain::workspace_tree::WorkspaceTreeRepository for FakeWorkspaceTreeRepository {
    async fn load_trees(
        &self,
        workspace_identities: &[crate::domain::workspace_tree::WorkspaceIdentity],
    ) -> Vec<Result<crate::domain::workspace_tree::WorkspaceTree, WorkflowError>> {
        workspace_identities
            .iter()
            .map(|identity| {
                Ok(crate::domain::workspace_tree::WorkspaceTree::empty(
                    identity.as_str(),
                ))
            })
            .collect()
    }

    async fn load_node(
        &self,
        _workspace_identity: &crate::domain::workspace_tree::WorkspaceIdentity,
        _node_id: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::WorkspaceTreeNode>,
        crate::domain::local_event::LocalEventQueryError,
    > {
        Ok(None)
    }

    async fn load_node_by_node_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::WorkspaceTreeNode>,
        crate::domain::local_event::LocalEventQueryError,
    > {
        Ok(self.nodes.lock().unwrap().get(node_execution_id).cloned())
    }

    async fn node_id_for_session(
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
        let diagnostics = Arc::new(FakeDiagnosticsGateway::default());
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
            diagnostics.clone(),
            Arc::new(FakeSecretSourceGateway),
            Arc::new(NoopArchiveRepository),
            workspace_nodes.clone(),
            workspace_query,
            Arc::new(crate::adaptor::gateway::failure_records::FailureRecordStore::default()),
        );
        Self {
            usecase,
            editors,
            definitions,
            definition_sources,
            diagnostics,
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
        process_presence: Default::default(),
        can_resume_session: false,
        worktree: None,
        id: format!("node:{node_execution_id}"),
        parent_id: execution_id.map(str::to_string),
        sibling_order: 0,
        kind: crate::domain::workspace_tree::WorkspaceNodeKind::WorkflowSession,
        title: "node".to_string(),
        status: crate::domain::workspace_tree::WorkspaceNodeStatus::Running,
        status_classification:
            crate::domain::workspace_tree::WorkspaceNodeStatusClassification::Active,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: Some(crate::domain::workflow::AgentSessionActivity::AwaitingInstruction),
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
fn resolve_worktree_path_delegates_to_managed_worktree_gateway() {
    let fixture = Fixture::new();

    assert_eq!(
        fixture.usecase.resolve_worktree_path("repo").unwrap(),
        "/canonical/repo"
    );
    assert!(fixture.usecase.resolve_worktree_path("reject").is_err());
}

#[tokio::test]
async fn workflow_read_facade_owns_active_aggregation_filtering_and_dto_projection() {
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
fn test_workflow読取_定義不在を返す() {
    // Given
    let fixture = Fixture::new();
    // When
    let result = fixture.usecase.get_workflow_dto("missing");
    // Then
    assert!(result.unwrap().is_none());
}
#[test]
fn test_workflow読取_定義読取失敗を不在と区別する() {
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
fn test_workflow読取_形式読取失敗を既定形式と区別する() {
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

#[tokio::test]
async fn authorize_execution_summary_for_worktree_hides_unmanaged_or_mismatched_runs() {
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
async fn authorize_node_execution_access_for_worktree_checks_identity_and_execution_ownership() {
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
fn test_診断usecase_指定directoryをgatewayへ渡す() {
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
fn test_診断usecase_適用済みdirectoryをgatewayへ渡す() {
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
