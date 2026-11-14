use crate::domain::workflow::error::WorkflowError;
use crate::domain::workflow::gateway::ManagedWorktreeGateway;
use crate::domain::workflow::value_objects::definition::WorkflowDefinitionContent;
use crate::domain::workflow::value_objects::execution::ExecutionStatus;
use crate::domain::workflow::value_objects::execution::ExecutionTree;
use crate::domain::workflow::value_objects::execution_metadata::WorkflowExecutionSummary;
use crate::domain::workflow::value_objects::facet::FacetKind;

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
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Default)]
struct FakeEditorLauncher {
    opened: Mutex<Vec<(Vec<std::path::PathBuf>, String, String)>>,
}

impl FakeEditorLauncher {
    fn opened(&self) -> Vec<(Vec<std::path::PathBuf>, String, String)> {
        self.opened.lock().unwrap().clone()
    }
}

impl crate::domain::external_editor::EditorLauncherGateway for FakeEditorLauncher {
    fn open_paths(
        &self,
        paths: &[std::path::PathBuf],
        editor: &str,
        label: &str,
    ) -> Result<(), crate::domain::external_editor::EditorError> {
        self.opened
            .lock()
            .unwrap()
            .push((paths.into(), editor.into(), label.into()));
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

struct SelectedEditor;
impl crate::domain::external_editor::EditorSettingsGateway for SelectedEditor {
    fn selected_editor(&self) -> Result<String, crate::domain::external_editor::EditorError> {
        Ok("Code".into())
    }
}

struct Fixture {
    usecase: WorkflowUsecase,
    editors: Arc<FakeEditorLauncher>,
    facets: Arc<FakeFacetRepository>,
    definitions: Arc<FakeDefinitionRepository>,
    diagnostics: Arc<FakeDiagnosticsGateway>,
    summaries: Arc<FakeSummaryQuery>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_executions(Vec::new())
    }

    fn with_executions(executions: Vec<WorkflowExecutionSummary>) -> Self {
        let definitions = Arc::new(FakeDefinitionRepository::default());
        let facets = Arc::new(FakeFacetRepository::default());
        let events = Arc::new(FakeEventRepository::default());
        let editors = Arc::new(FakeEditorLauncher::default());
        let diagnostics = Arc::new(FakeDiagnosticsGateway::default());
        let workspace_nodes =
            crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new();
        let workspace_query =
            crate::usecase::workspace_tree::test_support::TestWorkspaceQueryService::new(
                executions,
            );
        let query = WorkflowQueryService::new(
            definitions.clone(),
            events.clone(),
            Arc::new(NoopExecutionProjectionRepository),
        );
        let summaries = Arc::new(FakeSummaryQuery::default());
        let usecase = WorkflowUsecase::new(
            query,
            summaries.clone(),
            summaries.clone(),
            definitions.clone(),
            facets.clone(),
            Arc::new(FakeManagedWorktreeGateway),
            Arc::new(SelectedEditor),
            editors.clone(),
            Arc::new(crate::usecase::workflow::test_support::NoopDirectoryService),
            diagnostics.clone(),
            Arc::new(NoopArchiveRepository),
            workspace_nodes.clone(),
            workspace_query,
        );
        Self {
            usecase,
            editors,
            facets,
            definitions,
            diagnostics,
            summaries,
        }
    }
}

fn workflow_definition(name: &str) -> crate::domain::workflow::WorkflowDefinition {
    crate::usecase::workflow::test_helpers::definition(WorkflowDefinitionContent {
        name: name.into(),
        entry: "main".into(),
        ..Default::default()
    })
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
    // Given
    let executions = vec![WorkflowExecutionSummary {
        execution_id: "00000000-0000-4000-8000-000000000001".into(),
        workflow_name: "wf".into(),
        status: ExecutionStatus::Running,
        worktree_path: "/repo".into(),
        current_node: None,
        started_at: 1.0,
        updated_at: 1.0,
        completed_at: None,
        error_reason: None,
        total_token_usage: Default::default(),
    }];
    let fixture = Fixture::with_executions(executions.clone());
    let expected = crate::usecase::workflow::WorkflowSummaryDto {
        name: "wf".into(),
        description: "desc".into(),
        builtin: false,
        is_running: true,
        source_format: crate::domain::workflow::WorkflowSourceFormat::Lua,
        failure: None,
    };
    fixture
        .summaries
        .workflows
        .lock()
        .unwrap()
        .push(expected.clone());
    // When
    let result = fixture.usecase.list_workflow_summaries().await.unwrap();
    // Then
    assert_eq!(result, vec![expected]);
    assert_eq!(
        fixture.summaries.running.lock().unwrap().as_ref(),
        Some(&executions)
    );
}

#[test]
pub fn test_workflow読取_定義不在を返す() {
    // Given
    let fixture = Fixture::new();
    // When
    let result = fixture.usecase.get_workflow("missing");
    // Then
    assert_eq!(result.unwrap(), None);
}

#[test]
pub fn test_workflow読取_定義読取失敗を不在と区別する() {
    // Given
    let fixture = Fixture::new();
    *fixture.definitions.read_error.lock().unwrap() = Some("definition unreadable".into());
    // When
    let result = fixture.usecase.get_workflow("missing");
    // Then
    assert!(
        matches!(result, Err(WorkflowError::External(message)) if message == "definition unreadable")
    );
}

#[test]
pub fn test_workflow読取_解釈失敗を既定内容に置き換えない() {
    // Given
    let fixture = Fixture::new();
    fixture.definitions.insert(
        "present",
        crate::domain::workflow::WorkflowDefinition::restore(
            "/workflows".into(),
            crate::domain::workflow::WorkflowSource::File {
                path: "/workflows/present.lua".into(),
            },
            crate::domain::workflow::WorkflowSourceFormat::Lua,
            Err(WorkflowError::external("parse failed")),
        ),
    );
    // When
    let definition = fixture.usecase.get_workflow("present").unwrap().unwrap();
    // Then
    assert_eq!(
        definition.format(),
        crate::domain::workflow::WorkflowSourceFormat::Lua
    );
    assert!(
        matches!(definition.content(), Err(WorkflowError::External(message)) if message == "parse failed")
    );
}

#[test]
pub fn editor_commands_delegate_to_external_editor_gateway() {
    // Given
    let fixture = Fixture::new();
    fixture
        .definitions
        .insert("custom-workflow", workflow_definition("custom-workflow"));
    fixture.facets.facets.lock().unwrap().insert(
        (FacetKind::Instruction, "implement".into()),
        "instruction".into(),
    );
    // When
    fixture
        .usecase
        .open_workflow_in_editor("custom-workflow")
        .unwrap();
    fixture
        .usecase
        .open_facet_in_editor(FacetKind::Instruction, "implement")
        .unwrap();
    fixture.usecase.open_workflow_directory_in_editor().unwrap();
    // Then
    use std::path::PathBuf;
    assert_eq!(
        fixture.editors.opened(),
        vec![
            (
                vec![
                    PathBuf::from("/workflows"),
                    PathBuf::from("/workflows/custom-workflow.yml")
                ],
                "Code".into(),
                "ワークフロー".into()
            ),
            (
                vec![
                    PathBuf::from("/workflows"),
                    PathBuf::from("/workflows/instructions/implement.md")
                ],
                "Code".into(),
                "ファセット".into()
            ),
            (
                vec![PathBuf::from("/workflows")],
                "Code".into(),
                "フォルダ".into()
            ),
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

#[test]
fn test_外部編集_builtinの定義とfacetを拒否しlauncherを呼ばない() {
    // Given
    use crate::domain::workflow::{WorkflowDefinition, WorkflowSource, WorkflowSourceFormat};
    let fixture = Fixture::new();
    fixture.definitions.insert(
        "builtin",
        WorkflowDefinition::restore(
            "/workflows".into(),
            WorkflowSource::Builtin,
            WorkflowSourceFormat::Yaml,
            Ok(WorkflowDefinitionContent {
                name: "builtin".into(),
                ..Default::default()
            }),
        ),
    );
    fixture
        .facets
        .facets
        .lock()
        .unwrap()
        .insert((FacetKind::Policy, "coding".into()), "# Coding".into());
    fixture
        .facets
        .builtin_keys
        .lock()
        .unwrap()
        .insert((FacetKind::Policy, "coding".into()));
    // When
    let definition_error = fixture
        .usecase
        .open_workflow_in_editor("builtin")
        .unwrap_err();
    let facet_error = fixture
        .usecase
        .open_facet_in_editor(FacetKind::Policy, "coding")
        .unwrap_err();
    // Then
    for error in [definition_error, facet_error] {
        assert!(error.to_string().contains("ビルトイン"));
        assert!(error.to_string().contains("外部エディタで開けません"));
    }
    assert!(fixture.editors.opened().is_empty());
}

#[derive(Default)]
struct FakeSummaryQuery {
    workflows: Mutex<Vec<crate::usecase::workflow::WorkflowSummaryDto>>,
    running: Mutex<Option<Vec<WorkflowExecutionSummary>>>,
}
impl crate::usecase::workflow::WorkflowDefinitionQueryService for FakeSummaryQuery {
    fn list_workflows(
        &self,
        running: &[WorkflowExecutionSummary],
    ) -> Result<Vec<crate::usecase::workflow::WorkflowSummaryDto>, WorkflowError> {
        *self.running.lock().unwrap() = Some(running.to_vec());
        Ok(self.workflows.lock().unwrap().clone())
    }
}
impl crate::usecase::workflow::FacetQueryService for FakeSummaryQuery {
    fn list_facet_summaries(
        &self,
        kind: FacetKind,
    ) -> Result<Vec<crate::usecase::workflow::FacetSummaryDto>, WorkflowError> {
        Ok(vec![crate::usecase::workflow::FacetSummaryDto {
            key: "coding".into(),
            kind: kind.canonical_name().into(),
            description: "desc".into(),
            builtin: true,
        }])
    }
}

#[test]
fn test_facet一覧_指定した種類でqueryのdtoを返す() {
    // Given
    let fixture = Fixture::new();
    // When
    let result = fixture
        .usecase
        .list_facet_summaries(FacetKind::Knowledge)
        .unwrap();
    // Then
    assert_eq!(
        result,
        vec![crate::usecase::workflow::FacetSummaryDto {
            key: "coding".into(),
            kind: "knowledge".into(),
            description: "desc".into(),
            builtin: true,
        }]
    );
}

pub(crate) fn empty_usecase() -> WorkflowUsecase {
    Fixture::new().usecase
}
