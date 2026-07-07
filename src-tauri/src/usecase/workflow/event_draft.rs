use serde::Deserialize;
use serde_json::Value;

use crate::domain::workflow::contract::{ContractLookupError, OutputSubmittedSnapshot};

use super::ports::WorkflowEventDraft;

pub(crate) fn resolve_step_output_contract_from_drafts(
    events: &[WorkflowEventDraft],
    step_name: &str,
    run_id: &str,
) -> Result<String, ContractLookupError> {
    let workflow = events
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
        })?;

    lookup_step_output_contract(workflow.definition, step_name)
        .map_err(|details| ContractLookupError::InvalidRunStartedPayload {
            details: format!("invalid payload for run_started event: {details}"),
        })?
        .ok_or_else(|| ContractLookupError::NoOutputContract {
            workflow_name: workflow.name,
            step: step_name.to_string(),
        })
}

struct RunStartedWorkflow<'a> {
    name: String,
    definition: &'a Value,
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

fn lookup_step_output_contract(
    workflow: &Value,
    step_name: &str,
) -> Result<Option<String>, String> {
    let nodes = workflow
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "workflow_definition.nodes must be an array".to_string())?;
    for node in nodes {
        if node.get("name").and_then(Value::as_str) == Some(step_name) {
            return output_contract_from_node(node);
        }
        if let Some(contract) = lookup_child_output_contract(
            node.get("parallel_children"),
            step_name,
            "parallel_children",
        )? {
            return Ok(Some(contract));
        }
        if let Some(fanout) = node.get("fanout") {
            if let Some(contract) = lookup_child_output_contract(
                fanout.get("parallel_children"),
                step_name,
                "fanout.parallel_children",
            )? {
                return Ok(Some(contract));
            }
        }
    }
    Ok(None)
}

fn lookup_child_output_contract(
    children: Option<&Value>,
    step_name: &str,
    field_path: &str,
) -> Result<Option<String>, String> {
    let Some(children) = children else {
        return Ok(None);
    };
    let children = children
        .as_array()
        .ok_or_else(|| format!("{field_path} must be an array"))?;
    for child in children {
        if child.get("name").and_then(Value::as_str) == Some(step_name) {
            return output_contract_from_node(child);
        }
    }
    Ok(None)
}

fn output_contract_from_node(node: &Value) -> Result<Option<String>, String> {
    match node.get("output_contract") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err("output_contract must be a string".to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct OutputSubmittedDraftPayload {
    #[serde(alias = "nodeName")]
    node_name: String,
    contract: String,
    #[serde(alias = "structuredOutput")]
    structured_output: Value,
    #[serde(default, alias = "submittedAt")]
    submitted_at: Option<f64>,
    #[serde(default, alias = "requestId")]
    request_id: Option<String>,
}

pub(crate) fn latest_output_submitted_from_drafts(
    events: &[WorkflowEventDraft],
    step_name: &str,
) -> Option<OutputSubmittedSnapshot> {
    events.iter().rev().find_map(|event| {
        if event.event_kind != "output_submitted" {
            return None;
        }
        let payload =
            serde_json::from_value::<OutputSubmittedDraftPayload>(event.payload.clone()).ok()?;
        (payload.node_name == step_name).then_some(OutputSubmittedSnapshot {
            contract: payload.contract,
            structured_output: payload.structured_output,
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
    fn resolve_step_output_contract_from_drafts_reads_run_started_definition() {
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
                        "output_contract": "review-verdict"
                    }]
                }
            }),
        }];

        let contract = resolve_step_output_contract_from_drafts(&events, "review", "run-1")
            .expect("contract should resolve");

        assert_eq!(contract, "review-verdict");
    }

    #[test]
    fn resolve_step_output_contract_from_drafts_reads_camel_case_run_started_definition() {
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
                        "output_contract": "review-verdict"
                    }]
                }
            }),
        }];

        let contract = resolve_step_output_contract_from_drafts(&events, "review", "run-1")
            .expect("contract should resolve");

        assert_eq!(contract, "review-verdict");
    }

    #[test]
    fn resolve_step_output_contract_from_drafts_reads_fanout_child_contract() {
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
                                "output_contract": "review-verdict"
                            }]
                        }
                    }]
                }
            }),
        }];

        let contract =
            resolve_step_output_contract_from_drafts(&events, "security-review", "run-1")
                .expect("fanout child contract should resolve");

        assert_eq!(contract, "review-verdict");
    }

    #[test]
    fn resolve_step_output_contract_from_drafts_reports_missing_contract() {
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

        let err = resolve_step_output_contract_from_drafts(&events, "review", "run-1")
            .expect_err("missing contract should be explicit");

        assert_eq!(
            err,
            ContractLookupError::NoOutputContract {
                workflow_name: "wf".to_string(),
                step: "review".to_string()
            }
        );
    }

    fn output_submitted(step: &str, timestamp: f64, verdict: &str) -> WorkflowEventDraft {
        WorkflowEventDraft {
            run_id: "run-1".to_string(),
            event_kind: "output_submitted".to_string(),
            timestamp,
            payload: serde_json::json!({
                "node_name": step,
                "contract": "review-verdict",
                "structured_output": {"verdict": verdict},
                "submitted_at": timestamp - 1.0,
                "request_id": format!("req-{timestamp}")
            }),
        }
    }

    #[test]
    fn latest_output_submitted_from_drafts_picks_latest_matching_step() {
        let events = vec![
            output_submitted("review", 10.0, "NEEDS_FIX"),
            output_submitted("other", 20.0, "BLOCKED"),
            output_submitted("review", 30.0, "LGTM"),
        ];

        let snapshot = latest_output_submitted_from_drafts(&events, "review")
            .expect("latest matching output should be returned");

        assert_eq!(snapshot.timestamp, 30.0);
        assert_eq!(snapshot.structured_output["verdict"], "LGTM");
        assert_eq!(snapshot.submitted_at, Some(29.0));
        assert_eq!(snapshot.request_id.as_deref(), Some("req-30"));
    }

    #[test]
    fn latest_output_submitted_from_drafts_reads_camel_case_output_payload() {
        let events = vec![WorkflowEventDraft {
            run_id: "run-1".to_string(),
            event_kind: "output_submitted".to_string(),
            timestamp: 10.0,
            payload: serde_json::json!({
                "nodeName": "review",
                "contract": "review-verdict",
                "structuredOutput": {"verdict": "LGTM"},
                "submittedAt": 9.0,
                "requestId": "req-10"
            }),
        }];

        let snapshot = latest_output_submitted_from_drafts(&events, "review")
            .expect("latest matching output should be returned");

        assert_eq!(snapshot.timestamp, 10.0);
        assert_eq!(snapshot.structured_output["verdict"], "LGTM");
        assert_eq!(snapshot.submitted_at, Some(9.0));
        assert_eq!(snapshot.request_id.as_deref(), Some("req-10"));
    }

    #[test]
    fn latest_output_submitted_from_drafts_returns_none_without_matching_step() {
        let events = vec![output_submitted("other", 20.0, "BLOCKED")];

        assert!(latest_output_submitted_from_drafts(&events, "review").is_none());
        assert!(latest_output_submitted_from_drafts(&[], "review").is_none());
    }
}
