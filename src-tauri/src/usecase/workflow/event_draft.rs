use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::domain::workflow::contract::{ArtifactSubmittedSnapshot, ContractLookupError};
use crate::domain::workflow::services::contract_schema;
use crate::domain::workflow::SchemaDef;

use super::ports::WorkflowEventDraft;

#[cfg(test)]
pub(crate) fn resolve_node_artifact_contract_from_drafts(
    events: &[WorkflowEventDraft],
    node_name: &str,
    execution_id: &str,
) -> Result<String, ContractLookupError> {
    let workflow = execution_started_workflow_from_drafts(events, execution_id)?;

    lookup_node_artifact_contract(workflow.definition, node_name)
        .map_err(
            |details| ContractLookupError::InvalidExecutionStartedPayload {
                details: format!("invalid payload for execution_started event: {details}"),
            },
        )?
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
    execution_id: &str,
) -> Result<ArtifactSchemaContext, ContractLookupError> {
    let workflow = execution_started_workflow_from_drafts(events, execution_id)?;
    let contract = lookup_node_artifact_contract(workflow.definition, node_name)
        .map_err(
            |details| ContractLookupError::InvalidExecutionStartedPayload {
                details: format!("invalid payload for execution_started event: {details}"),
            },
        )?
        .ok_or_else(|| ContractLookupError::NoArtifactContract {
            workflow_name: workflow.name.clone(),
            node: node_name.to_string(),
        })?;
    let schemas = schemas_from_workflow(workflow.definition).map_err(|details| {
        ContractLookupError::InvalidExecutionStartedPayload {
            details: format!("invalid payload for execution_started event: {details}"),
        }
    })?;
    Ok(ArtifactSchemaContext { contract, schemas })
}

pub(crate) fn node_exists_in_drafts(
    events: &[WorkflowEventDraft],
    node_name: &str,
    execution_id: &str,
) -> Result<bool, ContractLookupError> {
    let workflow = execution_started_workflow_from_drafts(events, execution_id)?;
    workflow_contains_node(workflow.definition, node_name).map_err(|details| {
        ContractLookupError::InvalidExecutionStartedPayload {
            details: format!("invalid payload for execution_started event: {details}"),
        }
    })
}

struct ExecutionStartedWorkflow<'a> {
    name: String,
    definition: &'a Value,
}

fn execution_started_workflow_from_drafts<'a>(
    events: &'a [WorkflowEventDraft],
    execution_id: &str,
) -> Result<ExecutionStartedWorkflow<'a>, ContractLookupError> {
    events
        .iter()
        .find(|event| event.event_kind == "execution_started")
        .map(|event| {
            execution_started_workflow_from_payload(&event.payload).map_err(|details| {
                ContractLookupError::InvalidExecutionStartedPayload {
                    details: format!("invalid payload for execution_started event: {details}"),
                }
            })
        })
        .transpose()?
        .ok_or_else(|| ContractLookupError::ExecutionNotFound {
            execution_id: execution_id.to_string(),
        })
}

fn execution_started_workflow_from_payload(
    payload: &Value,
) -> Result<ExecutionStartedWorkflow<'_>, String> {
    let definition = payload
        .get("definition")
        .ok_or_else(|| "missing definition".to_string())?;
    let name = definition
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "definition.name must be a string".to_string())?
        .to_string();
    Ok(ExecutionStartedWorkflow { name, definition })
}

fn schemas_from_workflow(workflow: &Value) -> Result<BTreeMap<String, SchemaDef>, String> {
    let Some(schemas) = workflow.get("schemas") else {
        return Ok(BTreeMap::new());
    };
    let schemas = schemas
        .as_object()
        .ok_or_else(|| "definition.schemas must be an object".to_string())?;
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
        .ok_or_else(|| "definition.nodes must be an array".to_string())?;
    for node in nodes {
        if node.get("name").and_then(Value::as_str) == Some(node_name) {
            return artifact_contract_from_node(node);
        }
    }
    Ok(None)
}

fn workflow_contains_node(workflow: &Value, node_name: &str) -> Result<bool, String> {
    let nodes = workflow
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "definition.nodes must be an array".to_string())?;
    for node in nodes {
        if node.get("name").and_then(Value::as_str) == Some(node_name) {
            return Ok(true);
        }
    }
    Ok(false)
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
#[serde(deny_unknown_fields)]
struct ArtifactProducedDraftPayload {
    #[serde(rename = "node_execution_id")]
    _node_execution_id: String,
    node_name: String,
    #[serde(default)]
    contract: Option<String>,
    value: Value,
    #[serde(default)]
    submitted_at: Option<f64>,
    #[serde(default)]
    request_id: Option<String>,
}

pub(crate) fn latest_artifact_produced_from_drafts(
    events: &[WorkflowEventDraft],
    node_name: &str,
) -> Option<ArtifactSubmittedSnapshot> {
    events.iter().rev().find_map(|event| {
        if event.event_kind != "artifact_produced" {
            return None;
        }
        let payload =
            serde_json::from_value::<ArtifactProducedDraftPayload>(event.payload.clone()).ok()?;
        (payload.node_name == node_name).then_some(ArtifactSubmittedSnapshot {
            contract: payload.contract,
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
    fn resolve_node_artifact_contract_from_drafts_reads_execution_started_definition() {
        let events = vec![WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "definition": {
                    "name": "wf",
                    "description": "",
                    "builtin": false,
                    "nodes": [{
                        "name": "review",
                        "session": {"provider": "claude", "gate": "auto"},
                        "artifact": "review-verdict"
                    }]
                }
            }),
        }];

        let contract = resolve_node_artifact_contract_from_drafts(&events, "review", "execution-1")
            .expect("contract should resolve");

        assert_eq!(contract, "review-verdict");
    }

    #[test]
    fn resolve_node_artifact_contract_from_drafts_rejects_noncanonical_definition_key() {
        let events = vec![WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflowDefinition": {
                    "name": "wf",
                    "description": "",
                    "builtin": false,
                    "nodes": [{
                        "name": "review",
                        "session": {"provider": "claude", "gate": "auto"},
                        "artifact": "review-verdict"
                    }]
                }
            }),
        }];

        let error = resolve_node_artifact_contract_from_drafts(&events, "review", "execution-1")
            .expect_err("noncanonical definition key must be rejected");
        assert!(matches!(
            error,
            ContractLookupError::InvalidExecutionStartedPayload { .. }
        ));
    }

    #[test]
    fn resolve_node_artifact_contract_from_drafts_reads_top_level_fanout_child_contract() {
        let events = vec![WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "definition": {
                    "name": "wf",
                    "description": "",
                    "builtin": false,
                    "nodes": [
                    {
                        "name": "review-fanout",
                        "fanout": {
                            "child": "security-review"
                        }
                    },
                    {
                        "name": "security-review",
                        "session": {"provider": "claude", "gate": "auto"},
                        "artifact": "review-verdict"
                    }]
                }
            }),
        }];

        let contract =
            resolve_node_artifact_contract_from_drafts(&events, "security-review", "execution-1")
                .expect("top-level fanout child contract should resolve");

        assert_eq!(contract, "review-verdict");
    }

    #[test]
    fn resolve_node_artifact_contract_from_drafts_reports_missing_contract() {
        let events = vec![WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "definition": {
                    "name": "wf",
                    "description": "",
                    "builtin": false,
                    "nodes": [{
                        "name": "review",
                        "session": {"provider": "claude", "gate": "auto"}
                    }]
                }
            }),
        }];

        let err = resolve_node_artifact_contract_from_drafts(&events, "review", "execution-1")
            .expect_err("missing contract should be explicit");

        assert_eq!(
            err,
            ContractLookupError::NoArtifactContract {
                workflow_name: "wf".to_string(),
                node: "review".to_string()
            }
        );
    }

    fn artifact_produced(node: &str, timestamp: f64, verdict: &str) -> WorkflowEventDraft {
        WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp,
            payload: serde_json::json!({
                "node_execution_id": format!("node-{timestamp}"),
                "node_name": node,
                "contract": "review-verdict",
                "value": {"verdict": verdict},
                "submitted_at": timestamp - 1.0,
                "request_id": format!("req-{timestamp}")
            }),
        }
    }

    #[test]
    fn latest_artifact_produced_from_drafts_picks_latest_matching_node() {
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
    fn latest_artifact_produced_from_drafts_rejects_noncanonical_payload() {
        let events = vec![WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp: 10.0,
            payload: serde_json::json!({
                "node_execution_id": "node-10",
                "nodeName": "review",
                "contract": "review-verdict",
                "structuredOutput": {"verdict": "LGTM"},
                "submittedAt": 9.0,
                "requestId": "req-10"
            }),
        }];

        assert!(latest_artifact_produced_from_drafts(&events, "review").is_none());
    }

    #[test]
    fn latest_artifact_produced_from_drafts_returns_none_without_matching_node() {
        let events = vec![artifact_produced("other", 20.0, "BLOCKED")];

        assert!(latest_artifact_produced_from_drafts(&events, "review").is_none());
        assert!(latest_artifact_produced_from_drafts(&[], "review").is_none());
    }

    #[test]
    fn latest_artifact_produced_from_drafts_returns_contractless_artifact() {
        let events = vec![WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp: 10.0,
            payload: serde_json::json!({
                "node_execution_id": "node-10",
                "node_name": "review",
                "value": {"stdout": "ok"}
            }),
        }];

        let snapshot = latest_artifact_produced_from_drafts(&events, "review")
            .expect("contractless standard artifact should be returned");

        assert_eq!(snapshot.contract, None);
        assert_eq!(snapshot.value["stdout"], "ok");
    }
}
