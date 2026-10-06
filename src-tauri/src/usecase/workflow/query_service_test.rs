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
        workflow: WorkflowDefinition,
    }

    impl crate::domain::workflow::WorkflowDefinitionRepository for FakeDefinitionRepository {
        fn list(&self, running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError> {
            Ok(vec![WorkflowSummary {
                failure: None,
                name: self.workflow.name.clone(),
                description: self.workflow.description.clone(),
                builtin: self.workflow.builtin,
                is_running: running_names.contains(&self.workflow.name),
                source_format: crate::domain::workflow::WorkflowSourceFormat::Yaml,
            }])
        }

        fn get(&self, file_stem: &str) -> Result<Option<WorkflowDefinition>, WorkflowError> {
            Ok((file_stem == self.workflow.name).then(|| self.workflow.clone()))
        }

        fn save(
            &self,
            _definition: WorkflowDefinition,
            _original_name: Option<&str>,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn delete(&self, _name: &str) -> Result<(), WorkflowError> {
            Ok(())
        }
    }

    struct FakeDefinitionSourceGateway {
        workflow_source: Option<String>,
    }

    impl WorkflowDefinitionSourceGateway for FakeDefinitionSourceGateway {
        fn get_source(&self, _file_stem: &str) -> Result<Option<String>, WorkflowError> {
            Ok(self.workflow_source.clone())
        }

        fn save_source(
            &self,
            _source: &str,
            _original_name: Option<&str>,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            Err(WorkflowError::external("not used"))
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

    /// `source_format` の戻り値だけを制御する gateway。
    struct FakeSourceFormatGateway {
        format: Option<crate::domain::workflow::WorkflowSourceFormat>,
    }

    impl WorkflowDefinitionSourceGateway for FakeSourceFormatGateway {
        fn get_source(&self, _file_stem: &str) -> Result<Option<String>, WorkflowError> {
            Ok(None)
        }

        fn source_format(
            &self,
            _file_stem: &str,
        ) -> Result<crate::domain::workflow::WorkflowSourceFormat, WorkflowError> {
            self.format
                .ok_or_else(|| WorkflowError::external("source format unavailable"))
        }

        fn save_source(
            &self,
            _source: &str,
            _original_name: Option<&str>,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            Err(WorkflowError::external("not used"))
        }
    }

    fn service_with_source_format(
        format: Option<crate::domain::workflow::WorkflowSourceFormat>,
    ) -> WorkflowQueryService {
        WorkflowQueryService::new(
            Arc::new(FakeDefinitionRepository {
                workflow: workflow(),
            }),
            Arc::new(FakeSourceFormatGateway { format }),
            Arc::new(FakeFacetRepository::default()),
            Arc::new(FakeEventRepository::default()),
            Arc::new(FakeExecutionProjectionRepository::default()),
        )
    }

    #[test]
    fn get_workflow_source_format_returns_lua_from_gateway() {
        let service =
            service_with_source_format(Some(crate::domain::workflow::WorkflowSourceFormat::Lua));

        assert_eq!(
            service.get_workflow_source_format("wf").unwrap(),
            crate::domain::workflow::WorkflowSourceFormat::Lua
        );
    }

    #[test]
    fn get_workflow_source_format_propagates_gateway_error() {
        let service = service_with_source_format(None);

        let error = service.get_workflow_source_format("wf").unwrap_err();

        assert!(error.to_string().contains("source format unavailable"));
    }

    #[test]
    fn get_workflow_source_format_rejects_invalid_file_stem() {
        let service =
            service_with_source_format(Some(crate::domain::workflow::WorkflowSourceFormat::Lua));

        assert!(service.get_workflow_source_format("../escape").is_err());
    }

    struct Fixture {
        service: WorkflowQueryService,
        facets: Arc<FakeFacetRepository>,
        events: Arc<FakeEventRepository>,
        projections: Arc<FakeExecutionProjectionRepository>,
    }

    impl Fixture {
        fn new() -> Self {
            let definitions = Arc::new(FakeDefinitionRepository {
                workflow: workflow(),
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
                facets,
                events,
                projections,
            }
        }
    }

    fn workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
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
        let fixture = Fixture::new();
        let summaries = fixture.service.list_workflows(&["wf".to_string()]).unwrap();
        assert_eq!(summaries[0].name, "wf");
        assert!(summaries[0].is_running);
        assert!(fixture.service.get_workflow("wf").unwrap().is_some());
        assert!(fixture.service.get_workflow("missing").unwrap().is_none());
        assert!(fixture.service.get_workflow("bad name!").is_err());
        assert!(fixture.service.get_workflow_source("bad name!").is_err());
    }

    #[tokio::test]
    async fn event_and_facet_queries_validate_execution_ids_and_delegate() {
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

        assert_eq!(
            fixture
                .service
                .read_events(test_execution_id())
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(matches!(
            fixture.service.read_events("not-a-uuid").await.unwrap_err(),
            WorkflowError::Validation(_)
        ));
        assert_eq!(
            fixture
                .service
                .get_facet(FacetKind::Instruction, "implement")
                .unwrap(),
            "instruction body"
        );
        assert_eq!(
            fixture
                .service
                .list_facet_summaries(FacetKind::Instruction)
                .unwrap()[0]
                .key,
            "implement"
        );
    }

    #[tokio::test]
    async fn get_execution_log_page_projects_only_the_requested_event_window() {
        let fixture = Fixture::new();
        for (event_kind, timestamp) in [("execution_started", 1.0), ("node_started", 2.0)] {
            fixture
                .events
                .append(&WorkflowEventDraft {
                    execution_id: test_execution_id().to_string(),
                    event_kind: event_kind.to_string(),
                    timestamp,
                    payload: serde_json::json!({}),
                })
                .unwrap();
        }

        let events = fixture
            .service
            .get_execution_log_page(test_execution_id(), WorkflowPageRequest::new(1, 1))
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "node_started");
        assert_eq!(events[0].timestamp_ms, 2000.0);
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
        assert_eq!(
            fixture
                .service
                .get_execution_log_page(id, WorkflowPageRequest::new(0, 10))
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(fixture
            .service
            .get_execution_state("agent-session-invalid")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn get_execution_log_projects_event_drafts_to_wire_timestamp_fields() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                execution_id: test_execution_id().to_string(),
                event_kind: "execution_started".to_string(),
                timestamp: 1.25,
                payload: serde_json::json!({
                    "workflow_name": "wf",
                    "worktree_path": "/wt",
                }),
            })
            .unwrap();

        let events = fixture
            .service
            .get_execution_log_page(test_execution_id(), WorkflowPageRequest::new(0, 10))
            .await
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "execution_started");
        assert_eq!(events[0].execution_id, test_execution_id());
        assert_eq!(events[0].payload["workflow_name"], "wf");
        assert_eq!(events[0].timestamp_ms, 1250.0);
    }

    #[tokio::test]
    async fn get_execution_log_renames_submission_timestamp_to_millisecond_field() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                execution_id: test_execution_id().to_string(),
                event_kind: "artifact_produced".to_string(),
                timestamp: 4.0,
                payload: serde_json::json!({
                    "node_execution_id": format!("{}:review:1", test_execution_id()),
                    "node_name": "review",
                    "contract": "review-result",
                    "value": {"status": "ok"},
                    "submitted_at": 4.0,
                    "request_id": "req-2",
                }),
            })
            .unwrap();

        let events = fixture
            .service
            .get_execution_log_page(test_execution_id(), WorkflowPageRequest::new(0, 10))
            .await
            .unwrap();

        assert_eq!(events[0].payload["submittedAtMs"], 4000.0);
        assert!(!events[0].payload.contains_key("submitted_at"));
        assert_eq!(events[0].timestamp_ms, 4000.0);
    }
}
