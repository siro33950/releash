use super::*;

#[test]
fn test_実行ログ応答_payloadを展開する() {
    // Given
    let response = WorkflowEventResponse::from(WorkflowEventView {
        event: "node_started".into(),
        execution_id: "id".into(),
        timestamp_ms: 2000.0,
        payload: serde_json::from_value(serde_json::json!({"node": "main"})).unwrap(),
    });
    // When
    let value = serde_json::to_value(response).unwrap();
    // Then
    assert_eq!(
        value,
        serde_json::json!({
            "event": "node_started",
            "execution_id": "id",
            "timestampMs": 2000.0,
            "node": "main"
        })
    );
}

#[test]
fn test_診断応答_変更前のjson形を維持する() {
    // Given
    let report = diagnostic::DiagnosticReport {
        items: vec![
            diagnostic::DiagnosticItem {
                code: "E001".into(),
                severity: diagnostic::Severity::Error,
                stage: diagnostic::DiagnosticStage::ParseShape,
                span: Some(diagnostic::DiagnosticSpan {
                    source: Some("workflow.yml".into()),
                    start_line: 2,
                    start_col: 3,
                    end_line: 2,
                    end_col: 8,
                }),
                message: "invalid field".into(),
                workflow_name: Some("demo".into()),
                node_name: Some("main".into()),
                facet_key: Some("policy".into()),
                facet_kind: Some("instruction".into()),
                field: Some("prompt".into()),
            },
            diagnostic::DiagnosticItem {
                code: "I001".into(),
                severity: diagnostic::Severity::Info,
                stage: diagnostic::DiagnosticStage::ControlFlow,
                span: Some(diagnostic::DiagnosticSpan {
                    source: None,
                    start_line: 1,
                    start_col: 1,
                    end_line: 1,
                    end_col: 4,
                }),
                message: "unreachable".into(),
                workflow_name: None,
                node_name: None,
                facet_key: None,
                facet_kind: None,
                field: None,
            },
            diagnostic::DiagnosticItem {
                code: "E002".into(),
                severity: diagnostic::Severity::Error,
                stage: diagnostic::DiagnosticStage::Resolve,
                span: None,
                message: "missing reference".into(),
                workflow_name: None,
                node_name: None,
                facet_key: None,
                facet_kind: None,
                field: None,
            },
            diagnostic::DiagnosticItem {
                code: "E003".into(),
                severity: diagnostic::Severity::Error,
                stage: diagnostic::DiagnosticStage::Typecheck,
                span: None,
                message: "invalid type".into(),
                workflow_name: None,
                node_name: None,
                facet_key: None,
                facet_kind: None,
                field: None,
            },
        ],
        workflow_summaries: HashMap::from([(
            "demo".into(),
            diagnostic::DiagnosticSummary {
                error_count: 2,
                info_count: 1,
            },
        )]),
        facet_summaries: HashMap::from([(
            "instruction/policy".into(),
            diagnostic::DiagnosticSummary {
                error_count: 1,
                info_count: 0,
            },
        )]),
        facet_usage: HashMap::from([(
            "instruction/policy".into(),
            vec![diagnostic::FacetUsageEntry {
                workflow_name: "demo".into(),
                node_name: "main".into(),
                slot: "instruction".into(),
            }],
        )]),
    };

    // When
    let value = serde_json::to_value(DiagnosticReportResponse::from(report)).unwrap();
    // Then
    assert_eq!(
        value,
        serde_json::json!({
            "items": [
                {
                    "code": "E001", "severity": "error", "stage": "parse_shape",
                    "span": {"source": "workflow.yml", "start_line": 2, "start_col": 3, "end_line": 2, "end_col": 8},
                    "message": "invalid field", "workflow_name": "demo", "node_name": "main",
                    "facet_key": "policy", "facet_kind": "instruction", "field": "prompt"
                },
                {
                    "code": "I001", "severity": "info", "stage": "control_flow",
                    "span": {"start_line": 1, "start_col": 1, "end_line": 1, "end_col": 4},
                    "message": "unreachable"
                },
                {"code": "E002", "severity": "error", "stage": "resolve", "message": "missing reference"},
                {"code": "E003", "severity": "error", "stage": "typecheck", "message": "invalid type"}
            ],
            "workflow_summaries": {"demo": {"error_count": 2, "info_count": 1}},
            "facet_summaries": {"instruction/policy": {"error_count": 1, "info_count": 0}},
            "facet_usage": {"instruction/policy": [{"workflow_name": "demo", "node_name": "main", "slot": "instruction"}]}
        })
    );
}
