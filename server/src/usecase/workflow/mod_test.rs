use crate::domain::workflow::error::WorkflowError;
use crate::domain::workflow::value_objects::definition::WorkflowDefinitionContent;
use crate::domain::workflow::value_objects::execution_metadata::WorkflowExecutionSummary;
use crate::domain::workflow::value_objects::facet::FacetKind;
use crate::domain::workflow::WorkflowDefinitionSourceService;
use crate::usecase::workflow::ports::WorkflowDiagnosticsGateway;
use crate::usecase::workflow::ports::WorkflowDiagnosticsTarget;
use crate::usecase::workflow::test_helpers::FakeDefinitionRepository;
use crate::usecase::workflow::test_helpers::FakeFacetRepository;
use crate::usecase::workflow::WorkflowUsecase;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Default)]
struct FakeDefinitionSourceGateway {
    sources: Mutex<HashMap<String, String>>,
    save_definition: Mutex<Option<WorkflowDefinitionContent>>,
    save_error: Mutex<Option<String>>,
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
}

impl WorkflowDefinitionSourceService for FakeDefinitionSourceGateway {
    fn get_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
        Ok(self.sources.lock().unwrap().get(file_stem).cloned())
    }

    fn parse(
        &self,
        _: &str,
    ) -> Result<
        crate::domain::workflow::WorkflowDefinition,
        crate::domain::workflow::WorkflowSourceSaveError,
    > {
        if let Some(message) = self.save_error.lock().unwrap().as_ref() {
            return Err(crate::domain::workflow::WorkflowSourceSaveError::Workflow(
                WorkflowError::external(message),
            ));
        }
        Ok(crate::usecase::workflow::test_helpers::definition(
            self.save_definition
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| {
                    workflow_definition("saved-workflow")
                        .content()
                        .unwrap()
                        .clone()
                }),
        ))
    }
}

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
    fn open_path(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), crate::domain::external_editor::EditorError> {
        unreachable!()
    }
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

struct Fixture {
    usecase: WorkflowUsecase,
    editors: Arc<FakeEditorLauncher>,
    facets: Arc<FakeFacetRepository>,
    definitions: Arc<FakeDefinitionRepository>,
    definition_sources: Arc<FakeDefinitionSourceGateway>,
    diagnostics: Arc<FakeDiagnosticsGateway>,
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
        _executions: Vec<WorkflowExecutionSummary>,
        _definition_sources: Arc<FakeDefinitionSourceGateway>,
    ) -> Self {
        todo!()
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
    // Given
    let fixture = Fixture::new();
    // When
    let result = fixture.usecase.get_workflow("missing");
    // Then
    assert!(matches!(result, Err(WorkflowError::NotFound(_))));
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
    todo!()
}

#[test]
pub fn save_workflow_source_returns_saved_definition_and_surfaces_gateway_errors() {
    let fixture = Fixture::new();
    fixture
        .definition_sources
        .set_save_definition(workflow_definition("saved-wf").content().unwrap().clone());

    let saved = fixture
        .usecase
        .save_workflow_source("name: saved-wf\n", Some("old-wf"))
        .unwrap();

    assert_eq!(saved.file_stem(), "saved-wf");
    assert_eq!(fixture.definitions.get_saved("saved-wf"), Some(saved));

    assert_eq!(
        *fixture.definitions.saved_sources.lock().unwrap(),
        [("name: saved-wf\n".into(), Some("old-wf".into()))]
    );

    fixture.definition_sources.fail_saves("save failed");

    assert!(fixture
        .usecase
        .save_workflow_source("name: failed-wf\n", None)
        .is_err());
}

#[test]
pub fn editor_commands_delegate_to_external_editor_gateway() {
    // Given
    let fixture = Fixture::new();
    fixture
        .definitions
        .insert(workflow_definition("custom-workflow"));
    crate::domain::workflow::FacetRepository::save(
        &*fixture.facets,
        crate::usecase::workflow::test_helpers::facet(
            FacetKind::Instruction,
            "implement",
            "instruction".into(),
        ),
        true,
    )
    .unwrap();
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
    fixture.definitions.insert(WorkflowDefinition::restore(
        "builtin".into(),
        "/workflows".into(),
        WorkflowSource::Builtin,
        WorkflowSourceFormat::Yaml,
        Ok(WorkflowDefinitionContent {
            name: "builtin".into(),
            ..Default::default()
        }),
    ));
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
