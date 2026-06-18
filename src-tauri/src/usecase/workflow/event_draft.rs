use serde::Deserialize;
use serde_json::Value;

use crate::domain::workflow::contract::{self, ContractLookupError, OutputSubmittedSnapshot};
use crate::domain::workflow::WorkflowDefinition;

use super::ports::WorkflowEventDraft;

#[derive(Debug, Deserialize)]
struct RunStartedDraftPayload {
    #[serde(alias = "workflowDefinition")]
    workflow_definition: WorkflowDefinition,
}

pub(crate) fn resolve_step_output_contract_from_drafts(
    events: &[WorkflowEventDraft],
    step_name: &str,
    run_id: &str,
) -> Result<String, ContractLookupError> {
    let workflow = events
        .iter()
        .find(|event| event.event_kind == "run_started")
        .map(|event| {
            serde_json::from_value::<RunStartedDraftPayload>(event.payload.clone()).map_err(|err| {
                ContractLookupError::InvalidRunStartedPayload {
                    details: format!("invalid payload for run_started event: {err}"),
                }
            })
        })
        .transpose()?
        .map(|payload| payload.workflow_definition)
        .ok_or_else(|| ContractLookupError::RunNotFound {
            run_id: run_id.to_string(),
        })?;

    contract::lookup_step_output_contract(&workflow, step_name).ok_or_else(|| {
        ContractLookupError::NoOutputContract {
            workflow_name: workflow.name,
            step: step_name.to_string(),
        }
    })
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
                        "type": "agent",
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
                        "type": "agent",
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
                        "type": "agent"
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
