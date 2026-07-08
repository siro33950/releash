use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::domain::workflow::contract::{ArtifactSubmittedSnapshot, ContractLookupError};
use crate::domain::workflow::services::contract_schema;
use crate::domain::workflow::SchemaDef;

use super::ports::WorkflowEventDraft;

pub(crate) fn resolve_node_artifact_contract_from_drafts(
    events: &[WorkflowEventDraft],
    node_name: &str,
    run_id: &str,
) -> Result<String, ContractLookupError> {
    let workflow = run_started_workflow_from_drafts(events, run_id)?;

    lookup_node_artifact_contract(workflow.definition, node_name)
        .map_err(|details| ContractLookupError::InvalidRunStartedPayload {
            details: format!("invalid payload for run_started event: {details}"),
        })?
        .ok_or_else(|| ContractLookupError::NoArtifactContract {
            workflow_name: workflow.name,
            node: node_name.to_string(),
        })
}

pub(crate) struct ArtifactSchemaContext {
    pub contract: String,
    pub schemas: BTreeMap<String, SchemaDef>,
}

pub(crate) fn resolve_node_artifact_schema_from_drafts(
    events: &[WorkflowEventDraft],
    node_name: &str,
    run_id: &str,
) -> Result<ArtifactSchemaContext, ContractLookupError> {
    let workflow = run_started_workflow_from_drafts(events, run_id)?;
    let contract = lookup_node_artifact_contract(workflow.definition, node_name)
        .map_err(|details| ContractLookupError::InvalidRunStartedPayload {
            details: format!("invalid payload for run_started event: {details}"),
        })?
        .ok_or_else(|| ContractLookupError::NoArtifactContract {
            workflow_name: workflow.name.clone(),
            node: node_name.to_string(),
        })?;
    let schemas = schemas_from_workflow(workflow.definition).map_err(|details| {
        ContractLookupError::InvalidRunStartedPayload {
            details: format!("invalid payload for run_started event: {details}"),
        }
    })?;
    Ok(ArtifactSchemaContext { contract, schemas })
}

struct RunStartedWorkflow<'a> {
    name: String,
    definition: &'a Value,
}

fn run_started_workflow_from_drafts<'a>(
    events: &'a [WorkflowEventDraft],
    run_id: &str,
) -> Result<RunStartedWorkflow<'a>, ContractLookupError> {
    events
        .iter()
        .find(|event| event.event_kind == "run_started")
        .map(|event| {
            run_started_workflow_from_payload(&event.payload).map_err(|details| {
                ContractLookupError::InvalidRunStartedPayload {
                    details: format!("invalid payload for run_started event: {details}"),
                }
            })
        })
        .transpose()?
        .ok_or_else(|| ContractLookupError::RunNotFound {
            run_id: run_id.to_string(),
        })
}

fn run_started_workflow_from_payload(payload: &Value) -> Result<RunStartedWorkflow<'_>, String> {
    let definition = payload
        .get("workflow_definition")
        .or_else(|| payload.get("workflowDefinition"))
        .ok_or_else(|| "missing workflow_definition".to_string())?;
    let name = definition
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "workflow_definition.name must be a string".to_string())?
        .to_string();
    Ok(RunStartedWorkflow { name, definition })
}

fn schemas_from_workflow(workflow: &Value) -> Result<BTreeMap<String, SchemaDef>, String> {
    let Some(schemas) = workflow.get("schemas") else {
        return Ok(BTreeMap::new());
    };
    let schemas = schemas
        .as_object()
        .ok_or_else(|| "workflow_definition.schemas must be an object".to_string())?;
    schemas
        .iter()
        .map(|(name, value)| {
            contract_schema::schema_def_from_json(value)
                .map(|schema| (name.clone(), schema))
                .map_err(|reason| format!("schemas.{name}: {reason}"))
        })
        .collect()
}

fn lookup_node_artifact_contract(
    workflow: &Value,
    node_name: &str,
) -> Result<Option<String>, String> {
    let nodes = workflow
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "workflow_definition.nodes must be an array".to_string())?;
    for node in nodes {
        if node.get("name").and_then(Value::as_str) == Some(node_name) {
            return artifact_contract_from_node(node);
        }
        if let Some(contract) = lookup_child_artifact_contract(
            node.get("parallel_children"),
            node_name,
            "parallel_children",
        )? {
            return Ok(Some(contract));
        }
        if let Some(fanout) = node.get("fanout") {
            if let Some(contract) = lookup_child_artifact_contract(
                fanout.get("parallel_children"),
                node_name,
                "fanout.parallel_children",
            )? {
                return Ok(Some(contract));
            }
        }
    }
    Ok(None)
}

fn lookup_child_artifact_contract(
    children: Option<&Value>,
    node_name: &str,
    field_path: &str,
) -> Result<Option<String>, String> {
    let Some(children) = children else {
        return Ok(None);
    };
    let children = children
        .as_array()
        .ok_or_else(|| format!("{field_path} must be an array"))?;
    for child in children {
        if child.get("name").and_then(Value::as_str) == Some(node_name) {
            return artifact_contract_from_node(child);
        }
    }
    Ok(None)
}

fn artifact_contract_from_node(node: &Value) -> Result<Option<String>, String> {
    match node.get("artifact") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err("artifact must be a string".to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct ArtifactProducedDraftPayload {
    #[serde(alias = "nodeName")]
    node_name: String,
    #[serde(default)]
    contract: Option<String>,
    #[serde(alias = "structuredOutput", alias = "structured_output")]
    value: Value,
    #[serde(default, alias = "submittedAt")]
    submitted_at: Option<f64>,
    #[serde(default, alias = "requestId")]
    request_id: Option<String>,
}

pub(crate) fn latest_artifact_produced_from_drafts(
    events: &[WorkflowEventDraft],
    step_name: &str,
) -> Option<ArtifactSubmittedSnapshot> {
    events.iter().rev().find_map(|event| {
        if event.event_kind != "artifact_produced" {
            return None;
        }
        let payload =
            serde_json::from_value::<ArtifactProducedDraftPayload>(event.payload.clone()).ok()?;
        let contract = payload.contract?;
        (payload.node_name == step_name).then_some(ArtifactSubmittedSnapshot {
            contract,
            value: payload.value,
            submitted_at: payload.submitted_at,
            request_id: payload.request_id,
            timestamp: event.timestamp,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_node_artifact_contract_from_drafts_reads_run_started_definition() {
        let events = vec![WorkflowEventDraft {
            run_id: "run-1".to_string(),
            event_kind: "run_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflow_definition": {
                    "name": "wf",
                    "description": "",
                    "builtin": false,
                    "variables": {},
                    "nodes": [{
                        "name": "review",
                        "session": {},
                        "artifact": "review-verdict"
                    }]
                }
            }),
        }];

        let contract = resolve_node_artifact_contract_from_drafts(&events, "review", "run-1")
            .expect("contract should resolve");

        assert_eq!(contract, "review-verdict");
    }

    #[test]
    fn resolve_node_artifact_contract_from_drafts_reads_camel_case_run_started_definition() {
        let events = vec![WorkflowEventDraft {
            run_id: "run-1".to_string(),
            event_kind: "run_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflowDefinition": {
                    "name": "wf",
                    "description": "",
                    "builtin": false,
                    "variables": {},
                    "nodes": [{
                        "name": "review",
                        "session": {},
                        "artifact": "review-verdict"
                    }]
                }
            }),
        }];

        let contract = resolve_node_artifact_contract_from_drafts(&events, "review", "run-1")
            .expect("contract should resolve");

        assert_eq!(contract, "review-verdict");
    }

    #[test]
    fn resolve_node_artifact_contract_from_drafts_reads_fanout_child_contract() {
        let events = vec![WorkflowEventDraft {
            run_id: "run-1".to_string(),
            event_kind: "run_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflow_definition": {
                    "name": "wf",
                    "description": "",
                    "builtin": false,
                    "variables": {},
                    "nodes": [{
                        "name": "review",
                        "fanout": {
                            "parallel_children": [{
                                "name": "security-review",
                                "artifact": "review-verdict"
                            }]
                        }
                    }]
                }
            }),
        }];

        let contract =
            resolve_node_artifact_contract_from_drafts(&events, "security-review", "run-1")
                .expect("fanout child contract should resolve");

        assert_eq!(contract, "review-verdict");
    }

    #[test]
    fn resolve_node_artifact_contract_from_drafts_reports_missing_contract() {
        let events = vec![WorkflowEventDraft {
            run_id: "run-1".to_string(),
            event_kind: "run_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflow_definition": {
                    "name": "wf",
                    "description": "",
                    "builtin": false,
                    "variables": {},
                    "nodes": [{
                        "name": "review",
                        "session": {}
                    }]
                }
            }),
        }];

        let err = resolve_node_artifact_contract_from_drafts(&events, "review", "run-1")
            .expect_err("missing contract should be explicit");

        assert_eq!(
            err,
            ContractLookupError::NoArtifactContract {
                workflow_name: "wf".to_string(),
                node: "review".to_string()
            }
        );
    }

    fn artifact_produced(step: &str, timestamp: f64, verdict: &str) -> WorkflowEventDraft {
        WorkflowEventDraft {
            run_id: "run-1".to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp,
            payload: serde_json::json!({
                "node_name": step,
                "contract": "review-verdict",
                "value": {"verdict": verdict},
                "submitted_at": timestamp - 1.0,
                "request_id": format!("req-{timestamp}")
            }),
        }
    }

    #[test]
    fn latest_artifact_produced_from_drafts_picks_latest_matching_step() {
        let events = vec![
            artifact_produced("review", 10.0, "NEEDS_FIX"),
            artifact_produced("other", 20.0, "BLOCKED"),
            artifact_produced("review", 30.0, "LGTM"),
        ];

        let snapshot = latest_artifact_produced_from_drafts(&events, "review")
            .expect("latest matching output should be returned");

        assert_eq!(snapshot.timestamp, 30.0);
        assert_eq!(snapshot.value["verdict"], "LGTM");
        assert_eq!(snapshot.submitted_at, Some(29.0));
        assert_eq!(snapshot.request_id.as_deref(), Some("req-30"));
    }

    #[test]
    fn latest_artifact_produced_from_drafts_reads_camel_case_output_payload() {
        let events = vec![WorkflowEventDraft {
            run_id: "run-1".to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp: 10.0,
            payload: serde_json::json!({
                "nodeName": "review",
                "contract": "review-verdict",
                "structuredOutput": {"verdict": "LGTM"},
                "submittedAt": 9.0,
                "requestId": "req-10"
            }),
        }];

        let snapshot = latest_artifact_produced_from_drafts(&events, "review")
            .expect("latest matching output should be returned");

        assert_eq!(snapshot.timestamp, 10.0);
        assert_eq!(snapshot.value["verdict"], "LGTM");
        assert_eq!(snapshot.submitted_at, Some(9.0));
        assert_eq!(snapshot.request_id.as_deref(), Some("req-10"));
    }

    #[test]
    fn latest_artifact_produced_from_drafts_returns_none_without_matching_step() {
        let events = vec![artifact_produced("other", 20.0, "BLOCKED")];

        assert!(latest_artifact_produced_from_drafts(&events, "review").is_none());
        assert!(latest_artifact_produced_from_drafts(&[], "review").is_none());
    }

    #[test]
    fn latest_artifact_produced_from_drafts_ignores_contractless_artifact() {
        let events = vec![WorkflowEventDraft {
            run_id: "run-1".to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp: 10.0,
            payload: serde_json::json!({
                "node_name": "review",
                "value": {"stdout": "ok"}
            }),
        }];

        assert!(latest_artifact_produced_from_drafts(&events, "review").is_none());
    }
}
