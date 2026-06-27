use std::path::Path;

use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestEventKind {
    OutputSubmitted,
    CliMutationRequested,
}

impl RequestEventKind {
    fn validation_label(self) -> &'static str {
        match self {
            Self::OutputSubmitted => "SubmitOutput",
            Self::CliMutationRequested => "CLI mutation",
        }
    }

    fn matches_request_id(self, event: &WorkflowEvent, request_id: &str) -> bool {
        match self {
            Self::OutputSubmitted => matches!(
                event,
                WorkflowEvent::OutputSubmitted { request_id: Some(id), .. }
                    | WorkflowEvent::ContractRepairRequested { request_id: Some(id), .. }
                    if id == request_id
            ),
            Self::CliMutationRequested => matches!(
                event,
                WorkflowEvent::CliMutationRequested { request_id: id, .. } if id == request_id
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RequestEventLookupError {
    InvalidRunId(String),
    InvalidRequestId(String),
    ReadLog(String),
}

pub(crate) fn request_event_already_recorded(
    data_dir: &Path,
    event_kind: RequestEventKind,
    run_id: &str,
    request_id: &str,
) -> Result<bool, RequestEventLookupError> {
    uuid::Uuid::parse_str(run_id).map_err(|_| {
        RequestEventLookupError::InvalidRunId(format!(
            "{} run_id must be UUID",
            event_kind.validation_label()
        ))
    })?;
    uuid::Uuid::parse_str(request_id).map_err(|_| {
        RequestEventLookupError::InvalidRequestId(format!(
            "{} request_id must be UUID",
            event_kind.validation_label()
        ))
    })?;

    let events = WorkflowEventLog::new(data_dir)
        .read_log(run_id)
        .map_err(RequestEventLookupError::ReadLog)?;
    Ok(events
        .iter()
        .any(|event| event_kind.matches_request_id(event, request_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::event::CliMutationRequestRecord;
    use crate::adaptor::gateway::workflow::failure_wire::{
        submission_violation_reason, SubmissionViolation,
    };

    fn uuid(suffix: u16) -> String {
        format!("00000000-0000-0000-0000-{suffix:012}")
    }

    #[test]
    fn request_event_already_recorded_finds_output_submitted_request_id() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = uuid(1);
        let request_id = uuid(2);
        WorkflowEventLog::new(tmp.path())
            .append_batch(&[WorkflowEvent::OutputSubmitted {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                contract: "review-verdict".to_string(),
                structured_output: serde_json::json!({"verdict": "LGTM"}),
                request_id: Some(request_id.clone()),
                submitted_at: Some(10.0),
                timestamp: 11.0,
            }])
            .unwrap();

        assert_eq!(
            request_event_already_recorded(
                tmp.path(),
                RequestEventKind::OutputSubmitted,
                &run_id,
                &request_id,
            ),
            Ok(true)
        );
        assert_eq!(
            request_event_already_recorded(
                tmp.path(),
                RequestEventKind::OutputSubmitted,
                &run_id,
                &uuid(3),
            ),
            Ok(false)
        );
    }

    #[test]
    fn request_event_already_recorded_treats_contract_repair_as_submit_output_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = uuid(10);
        let request_id = uuid(11);
        WorkflowEventLog::new(tmp.path())
            .append_batch(&[WorkflowEvent::ContractRepairRequested {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                node_name: "review".to_string(),
                run_index: 1,
                request_id: Some(request_id.clone()),
                attempt: 1,
                violation_reason: submission_violation_reason(
                    SubmissionViolation::InvalidSubmitOutput,
                )
                .to_string(),
                timestamp: 11.0,
            }])
            .unwrap();

        assert_eq!(
            request_event_already_recorded(
                tmp.path(),
                RequestEventKind::OutputSubmitted,
                &run_id,
                &request_id,
            ),
            Ok(true)
        );
    }

    #[test]
    fn request_event_already_recorded_finds_cli_mutation_request_id() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = uuid(4);
        let request_id = uuid(5);
        WorkflowEventLog::new(tmp.path())
            .append_batch(&[WorkflowEvent::CliMutationRequested {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                request_id: request_id.clone(),
                request: CliMutationRequestRecord::Abort { node_name: None },
                requested_at: 20.0,
                timestamp: 21.0,
            }])
            .unwrap();

        assert_eq!(
            request_event_already_recorded(
                tmp.path(),
                RequestEventKind::CliMutationRequested,
                &run_id,
                &request_id,
            ),
            Ok(true)
        );
        assert_eq!(
            request_event_already_recorded(
                tmp.path(),
                RequestEventKind::CliMutationRequested,
                &run_id,
                &uuid(6),
            ),
            Ok(false)
        );
    }

    #[test]
    fn request_event_already_recorded_preserves_validation_labels() {
        assert_eq!(
            request_event_already_recorded(
                std::path::Path::new("/tmp"),
                RequestEventKind::OutputSubmitted,
                "not-a-uuid",
                &uuid(7),
            ),
            Err(RequestEventLookupError::InvalidRunId(
                "SubmitOutput run_id must be UUID".to_string()
            ))
        );
        assert_eq!(
            request_event_already_recorded(
                std::path::Path::new("/tmp"),
                RequestEventKind::CliMutationRequested,
                &uuid(8),
                "not-a-uuid",
            ),
            Err(RequestEventLookupError::InvalidRequestId(
                "CLI mutation request_id must be UUID".to_string()
            ))
        );
    }
}
