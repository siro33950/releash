use crate::domain::workflow::WorkflowDefinitionContent;
use crate::usecase::workflow::test_helpers::FakeEventRepository;
pub(crate) mod tests {
    use super::super::*;
    use super::*;
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionStatus, FacetRefs, NodeCompletion, NodeDefinition, NodeExecution,
        NodeExecutionStatus, NodeKind, NodeKindName, SessionSpec, TokenUsage,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FakeDefinitionRepository {
        workflow: WorkflowDefinitionContent,
        format: crate::domain::workflow::WorkflowSourceFormat,
        read_error: Option<WorkflowError>,
    }

    impl crate::domain::workflow::WorkflowDefinitionRepository for FakeDefinitionRepository {
        fn list_names(&self) -> Result<Vec<String>, WorkflowError> {
            Ok(vec![self.workflow.name.clone()])
        }
        fn get(
            &self,
            name: &str,
        ) -> Result<crate::domain::workflow::WorkflowDefinition, WorkflowError> {
            if let Some(error) = &self.read_error {
                return Err(error.clone());
            }
            if name != self.workflow.name {
                return Err(WorkflowError::NotFound(name.into()));
            }
            Ok(crate::domain::workflow::WorkflowDefinition::restore(
                name.into(),
                "/workflows".into(),
                crate::domain::workflow::WorkflowSource::File {
                    path: "/workflows/wf.lua".into(),
                },
                self.format,
                Ok(self.workflow.clone()),
            ))
        }
        fn save(
            &self,
            _: crate::domain::workflow::WorkflowDefinition,
            _: Option<&str>,
        ) -> Result<(), WorkflowError> {
            unreachable!()
        }
        fn delete(
            &self,
            _: &crate::domain::workflow::WorkflowDefinition,
        ) -> Result<(), WorkflowError> {
            unreachable!()
        }
        fn find(
            &self,
            name: Option<&str>,
        ) -> Result<Option<crate::domain::workflow::WorkflowDefinition>, WorkflowError> {
            match name.map(|name| self.get(name)) {
                None | Some(Err(WorkflowError::NotFound(_))) => Ok(None),
                Some(result) => result.map(Some),
            }
        }
        fn save_source(
            &self,
            _: &crate::domain::workflow::WorkflowDefinition,
            _: &str,
            _: Option<&str>,
        ) -> Result<(), WorkflowError> {
            unreachable!()
        }
    }

    struct FakeDefinitionSourceGateway {
        workflow_source: Option<String>,
    }

    impl WorkflowDefinitionSourceService for FakeDefinitionSourceGateway {
        fn get_source(&self, _file_stem: &str) -> Result<Option<String>, WorkflowError> {
            Ok(self.workflow_source.clone())
        }

        fn parse(
            &self,
            _source: &str,
        ) -> Result<
            crate::domain::workflow::WorkflowDefinition,
            crate::domain::workflow::WorkflowSourceSaveError,
        > {
            unreachable!()
        }
    }

    use crate::usecase::workflow::test_helpers::FakeFacetRepository;

    #[derive(Default)]
    struct FakeExecutionProjectionRepository {
        executions: Mutex<HashMap<String, ExecutionTree>>,
    }

    impl FakeExecutionProjectionRepository {
        fn seed(&self, execution: ExecutionTree) {
            self.executions
                .lock()
                .unwrap()
                .insert(execution.id.clone(), execution);
        }
    }

    #[async_trait::async_trait]
    impl WorkflowExecutionProjectionRepository for FakeExecutionProjectionRepository {
        fn get_node_artifact_from_events(
            &self,
            execution_id: &ExecutionTreeId,
            node_name: &str,
            _events: &[WorkflowEventDraft],
        ) -> Result<Option<crate::domain::workflow::Artifact>, WorkflowError> {
            Ok(self
                .executions
                .lock()
                .unwrap()
                .get(execution_id.as_str())
                .cloned()
                .and_then(|execution| {
                    execution
                        .node_executions
                        .into_iter()
                        .rev()
                        .find(|node| node.node_name == node_name)
                        .and_then(|node| node.artifact)
                }))
        }

        async fn get_execution(
            &self,
            execution_id: &ExecutionTreeId,
        ) -> Result<Option<ExecutionTree>, WorkflowError> {
            Ok(self
                .executions
                .lock()
                .unwrap()
                .get(execution_id.as_str())
                .cloned())
        }
    }

    fn service_with_metadata(
        format: crate::domain::workflow::WorkflowSourceFormat,
        read_error: Option<WorkflowError>,
    ) -> WorkflowQueryService {
        WorkflowQueryService::new(
            Arc::new(FakeDefinitionRepository {
                workflow: workflow(),
                format,
                read_error,
            }),
            Arc::new(FakeDefinitionSourceGateway {
                workflow_source: None,
            }),
            Arc::new(FakeFacetRepository::default()),
            Arc::new(FakeEventRepository::default()),
            Arc::new(FakeExecutionProjectionRepository::default()),
        )
    }

    #[test]
    fn test_workflow取得_entityのlua形式を保持する() {
        // Given
        let service =
            service_with_metadata(crate::domain::workflow::WorkflowSourceFormat::Lua, None);
        // When
        let definition = service.get_workflow("wf").unwrap().unwrap();
        // Then
        assert_eq!(
            definition.format(),
            crate::domain::workflow::WorkflowSourceFormat::Lua
        );
        assert_eq!(definition.content(), Ok(&workflow()));
    }
    #[test]
    fn test_workflow取得_repositoryの失敗を伝える() {
        // Given
        let service = service_with_metadata(
            crate::domain::workflow::WorkflowSourceFormat::Yaml,
            Some(WorkflowError::external("read failed")),
        );
        // When / Then
        assert!(
            matches!(service.get_workflow("wf"), Err(WorkflowError::External(message)) if message == "read failed")
        );
    }
    #[test]
    fn test_workflow取得_不正なfile_stemをrepositoryより先に拒否する() {
        // Given
        let service = service_with_metadata(
            crate::domain::workflow::WorkflowSourceFormat::Yaml,
            Some(WorkflowError::external("must not be called")),
        );
        // When / Then
        assert!(matches!(
            service.get_workflow("../escape"),
            Err(WorkflowError::Validation(_))
        ));
    }

    struct Fixture {
        service: WorkflowQueryService,
        events: Arc<FakeEventRepository>,
        projections: Arc<FakeExecutionProjectionRepository>,
        facets: Arc<FakeFacetRepository>,
    }

    impl Fixture {
        fn new() -> Self {
            let definitions = Arc::new(FakeDefinitionRepository {
                workflow: workflow(),
                format: crate::domain::workflow::WorkflowSourceFormat::Yaml,
                read_error: None,
            });
            let definition_sources = Arc::new(FakeDefinitionSourceGateway {
                workflow_source: None,
            });
            let facets = Arc::new(FakeFacetRepository::default());
            let events = Arc::new(FakeEventRepository::default());
            let projections = Arc::new(FakeExecutionProjectionRepository::default());
            let service = WorkflowQueryService::new(
                definitions,
                definition_sources,
                facets.clone(),
                events.clone(),
                projections.clone(),
            );
            Self {
                service,
                events,
                projections,
                facets,
            }
        }
    }

    fn workflow() -> WorkflowDefinitionContent {
        WorkflowDefinitionContent {
            name: "wf".to_string(),
            description: "desc".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("implement".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                completion: NodeCompletion::require_approval(),
                ..Default::default()
            }],
            entry: "review".to_string(),
        }
    }

    fn execution_projection(execution_id: &str) -> ExecutionTree {
        ExecutionTree {
            id: execution_id.to_string(),
            workflow_name: "wf".to_string(),
            status: ExecutionStatus::Running,
            current_node: Some("review".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            worktree_path: "/repo".to_string(),
            started_at: 1.0,
            updated_at: 1.0,
            completed_at: None,
            error_reason: None,
            total_token_usage: TokenUsage::default(),
            node_executions: vec![NodeExecution {
                process_presence: Default::default(),
                worktree: None,
                id: "ne-review-1".to_string(),
                execution_id: execution_id.to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                status: NodeExecutionStatus::Running,
                session_id: None,
                display_command: None,
                shell: None,
                cwd: None,
                result_summary: None,
                artifact: None,
                token_usage: None,

                parent: None,
                completion_signals: Default::default(),
                started_at: 1.0,
                completed_at: None,
            }],
            artifacts: Vec::new(),
            fanouts: Vec::new(),
            approval_target: None,
        }
    }

    fn test_execution_id() -> &'static str {
        "00000000-0000-4000-8000-000000000101"
    }

    fn artifact_produced(
        execution_id: &str,
        node_name: &str,
        contract: &str,
        structured_output: serde_json::Value,
        timestamp: f64,
        request_id: &str,
    ) -> WorkflowEventDraft {
        WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp,
            payload: serde_json::json!({
                "nodeExecutionId": format!("{execution_id}:{node_name}:1"),
                "nodeName": node_name,
                "kind": "session",
                "attempt": 1,
                "contract": contract,
                "value": structured_output,
                "requestId": request_id,
            }),
        }
    }

    #[test]
    fn workflow_queries_delegate_to_definition_repository() {
        // Given
        let fixture = Fixture::new();
        // When
        let result = fixture.service.get_workflow("wf").unwrap().unwrap();
        // Then
        assert_eq!(result.content(), Ok(&workflow()));
        assert_eq!(fixture.service.get_workflow("missing").unwrap(), None);
        assert!(fixture.service.get_workflow("bad name!").is_err());
        assert!(fixture.service.get_workflow_source("bad name!").is_err());
    }

    #[tokio::test]
    async fn event_and_facet_queries_validate_execution_ids_and_delegate() {
        // Given
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                execution_id: test_execution_id().to_string(),
                event_kind: "execution_started".to_string(),
                timestamp: 1.0,
                payload: serde_json::json!({}),
            })
            .unwrap();
        fixture.facets.facets.lock().unwrap().insert(
            (FacetKind::Instruction, "implement".to_string()),
            "instruction body".to_string(),
        );

        // When
        let events = fixture
            .service
            .read_events(test_execution_id())
            .await
            .unwrap();
        let invalid_execution = fixture.service.read_events("not-a-uuid").await.unwrap_err();
        let facet = fixture
            .service
            .get_facet(FacetKind::Instruction, "implement")
            .unwrap();

        // Then
        assert_eq!(events.len(), 1);
        assert!(matches!(invalid_execution, WorkflowError::Validation(_)));
        assert_eq!(facet.content(), "instruction body");
    }

    #[tokio::test]
    async fn get_output_returns_latest_submitted_snapshot_for_node() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&artifact_produced(
                test_execution_id(),
                "review",
                "review-result",
                serde_json::json!({"status":"old"}),
                2.0,
                "req-old",
            ))
            .unwrap();
        fixture
            .events
            .append(&artifact_produced(
                test_execution_id(),
                "review",
                "review-result",
                serde_json::json!({"status":"new"}),
                3.0,
                "req-new",
            ))
            .unwrap();

        let events = fixture
            .service
            .read_events(test_execution_id())
            .await
            .unwrap();
        let result = WorkflowQueryService::get_output_from_events(&events, "review");

        assert_eq!(
            result,
            WorkflowGetOutputResult::Submitted {
                contract: Some("review-result".to_string()),
                structured_output: serde_json::json!({"status":"new"}),
                submitted_at: Some(3.0),
                request_id: Some("req-new".to_string()),
                timestamp: 3.0,
            }
        );
        assert_eq!(
            WorkflowQueryService::get_output_from_events(&events, "missing"),
            WorkflowGetOutputResult::NotSubmitted
        );
    }

    #[tokio::test]
    async fn get_output_returns_contractless_standard_artifact_for_node() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                execution_id: test_execution_id().to_string(),
                event_kind: "artifact_produced".to_string(),
                timestamp: 4.0,
                payload: serde_json::json!({
                    "nodeExecutionId": format!("{}:review:1", test_execution_id()),
                    "nodeName": "review",
                    "kind": "command",
                    "attempt": 1,
                    "contract": null,
                    "value": {
                        "ok": false,
                        "exit_code": 7,
                        "stdout": "out",
                        "stderr": "err",
                        "duration": 10
                    }
                }),
            })
            .unwrap();

        let events = fixture
            .service
            .read_events(test_execution_id())
            .await
            .unwrap();
        let result = WorkflowQueryService::get_output_from_events(&events, "review");

        assert_eq!(
            result,
            WorkflowGetOutputResult::Submitted {
                contract: None,
                structured_output: serde_json::json!({
                    "ok": false,
                    "exit_code": 7,
                    "stdout": "out",
                    "stderr": "err",
                    "duration": 10
                }),
                submitted_at: Some(4.0),
                request_id: None,
                timestamp: 4.0,
            }
        );
    }

    #[tokio::test]
    async fn get_execution_state_delegates_to_execution_projection_port() {
        let fixture = Fixture::new();
        fixture
            .projections
            .seed(execution_projection(test_execution_id()));

        let state = fixture
            .service
            .get_execution_state(test_execution_id())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(state.id, test_execution_id());
        assert_eq!(state.workflow_name, "wf");
        assert_eq!(state.node_executions[0].id, "ne-review-1");
        assert_eq!(state.node_executions[0].node_name, "review");
        assert!(fixture
            .service
            .get_execution_state("not-a-uuid")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_実行木query_単独sessionの状態と事実を共通idで読める() {
        // Given
        let fixture = Fixture::new();
        let id = "agent-session-00000000000040008000000000000001";
        let expected = execution_projection(id);
        fixture.projections.seed(expected.clone());
        fixture
            .events
            .append(&artifact_produced(
                id,
                "review",
                "review-result",
                serde_json::json!({"ok": true}),
                2.0,
                "request",
            ))
            .unwrap();
        // When / Then
        assert_eq!(
            fixture.service.get_execution_state(id).await.unwrap(),
            Some(expected)
        );
        assert_eq!(fixture.service.read_events(id).await.unwrap().len(), 1);
        assert!(fixture
            .service
            .get_execution_state("agent-session-invalid")
            .await
            .is_err());
    }
}
