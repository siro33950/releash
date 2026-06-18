use serde::Deserialize;

use crate::adaptor::gateway::workflow::{
    event as legacy_event, facet as legacy_facet, run as legacy_run,
};
use crate::domain::workflow as domain;
use crate::domain::workflow::value_objects::ResolvedFacets;
use crate::usecase::workflow::ports::WorkflowEventDraft;

pub(crate) fn domain_run_record_to_legacy(
    run: &domain::WorkflowRunRecord,
) -> legacy_run::WorkflowRun {
    legacy_run::WorkflowRun {
        run_id: run.run_id.clone(),
        workflow_name: run.workflow_name.clone(),
        task: run.task.clone(),
        status: domain_run_status_to_legacy(run.status),
        worktree_path: run.worktree_path.clone(),
        current_node_name: run.current_node_name.clone(),
        trigger_source: domain_trigger_source_to_legacy(run.trigger_source),
        started_at: run.started_at,
        updated_at: run.updated_at,
        completed_at: run.completed_at,
        error_reason: run.error_reason.clone(),
    }
}

pub(crate) fn legacy_run_summary_to_domain(
    run: legacy_run::WorkflowRunSummary,
) -> domain::WorkflowRunSummary {
    domain::WorkflowRunSummary {
        run_id: run.run_id,
        workflow_name: run.workflow_name,
        task: run.task,
        status: legacy_run_status_to_domain(run.status),
        worktree_path: run.worktree_path,
        current_node_name: run.current_node_name,
        trigger_source: legacy_trigger_source_to_domain(run.trigger_source),
        started_at: run.started_at,
        updated_at: run.updated_at,
        completed_at: run.completed_at,
        error_reason: run.error_reason,
    }
}

#[cfg(test)]
pub(crate) fn domain_run_summary_to_legacy(
    run: domain::WorkflowRunSummary,
) -> legacy_run::WorkflowRunSummary {
    legacy_run::WorkflowRunSummary {
        run_id: run.run_id,
        workflow_name: run.workflow_name,
        task: run.task,
        status: domain_run_status_to_legacy(run.status),
        worktree_path: run.worktree_path,
        current_node_name: run.current_node_name,
        trigger_source: domain_trigger_source_to_legacy(run.trigger_source),
        started_at: run.started_at,
        updated_at: run.updated_at,
        completed_at: run.completed_at,
        error_reason: run.error_reason,
    }
}

pub(crate) fn domain_run_filter_to_legacy(
    filter: domain::RunListFilter,
) -> legacy_run::RunListFilter {
    let status = match filter.status {
        Some(domain::RunStatusFilter::Active) => Some(legacy_run::RunStatusFilter::Active),
        Some(domain::RunStatusFilter::Terminal) => Some(legacy_run::RunStatusFilter::Terminal),
        Some(domain::RunStatusFilter::All) | None => None,
    };
    legacy_run::RunListFilter {
        status,
        worktree_path: filter.worktree_path,
    }
}

pub(crate) fn domain_run_status_to_legacy(status: domain::RunStatus) -> legacy_run::RunStatus {
    match status {
        domain::RunStatus::Running => legacy_run::RunStatus::Running,
        domain::RunStatus::WaitingApproval => legacy_run::RunStatus::WaitingApproval,
        domain::RunStatus::Completed => legacy_run::RunStatus::Completed,
        domain::RunStatus::Failed => legacy_run::RunStatus::Failed,
        domain::RunStatus::Aborted => legacy_run::RunStatus::Aborted,
    }
}

fn legacy_run_status_to_domain(status: legacy_run::RunStatus) -> domain::RunStatus {
    match status {
        legacy_run::RunStatus::Running => domain::RunStatus::Running,
        legacy_run::RunStatus::WaitingApproval => domain::RunStatus::WaitingApproval,
        legacy_run::RunStatus::Completed => domain::RunStatus::Completed,
        legacy_run::RunStatus::Failed => domain::RunStatus::Failed,
        legacy_run::RunStatus::Aborted => domain::RunStatus::Aborted,
    }
}

fn domain_trigger_source_to_legacy(source: domain::TriggerSource) -> legacy_run::TriggerSource {
    match source {
        domain::TriggerSource::DesktopUi => legacy_run::TriggerSource::DesktopUi,
        domain::TriggerSource::Remote => legacy_run::TriggerSource::Remote,
        domain::TriggerSource::Cli => legacy_run::TriggerSource::Cli,
        domain::TriggerSource::Agent => legacy_run::TriggerSource::Agent,
    }
}

fn legacy_trigger_source_to_domain(source: legacy_run::TriggerSource) -> domain::TriggerSource {
    match source {
        legacy_run::TriggerSource::DesktopUi => domain::TriggerSource::DesktopUi,
        legacy_run::TriggerSource::Remote => domain::TriggerSource::Remote,
        legacy_run::TriggerSource::Cli => domain::TriggerSource::Cli,
        legacy_run::TriggerSource::Agent => domain::TriggerSource::Agent,
    }
}

pub(crate) fn domain_workflow_to_legacy(
    definition: &domain::WorkflowDefinition,
) -> Result<crate::adaptor::gateway::workflow::schema::Workflow, domain::WorkflowError> {
    let value = serde_json::to_value(definition)
        .map_err(|e| domain::WorkflowError::external(format!("serialize workflow: {e}")))?;
    let mut workflow =
        serde_json::from_value::<crate::adaptor::gateway::workflow::schema::Workflow>(value)
            .map_err(|e| domain::WorkflowError::external(format!("map workflow to legacy: {e}")))?;
    copy_domain_resolved_facets_to_legacy(definition, &mut workflow);
    Ok(workflow)
}

pub(crate) fn legacy_workflow_to_domain(
    workflow: crate::adaptor::gateway::workflow::schema::Workflow,
) -> Result<domain::WorkflowDefinition, domain::WorkflowError> {
    let value = serde_json::to_value(&workflow)
        .map_err(|e| domain::WorkflowError::external(format!("serialize legacy workflow: {e}")))?;
    let mut definition = serde_json::from_value::<domain::WorkflowDefinition>(value)
        .map_err(|e| domain::WorkflowError::external(format!("map workflow to domain: {e}")))?;
    copy_legacy_resolved_facets_to_domain(&workflow, &mut definition);
    Ok(definition)
}

pub(crate) fn legacy_workflow_summary_to_domain(
    summary: crate::adaptor::gateway::workflow::schema::Summary,
) -> domain::WorkflowSummary {
    domain::WorkflowSummary {
        name: summary.name,
        description: summary.description,
        builtin: summary.builtin,
        is_running: summary.is_running,
    }
}

#[cfg(test)]
pub(crate) fn domain_workflow_summary_to_legacy(
    summary: domain::WorkflowSummary,
) -> crate::adaptor::gateway::workflow::schema::Summary {
    crate::adaptor::gateway::workflow::schema::Summary {
        name: summary.name,
        description: summary.description,
        builtin: summary.builtin,
        is_running: summary.is_running,
    }
}

pub(crate) fn domain_facet_kind_to_legacy(kind: domain::FacetKind) -> legacy_facet::FacetKind {
    match kind {
        domain::FacetKind::Policy => legacy_facet::FacetKind::Policy,
        domain::FacetKind::Knowledge => legacy_facet::FacetKind::Knowledge,
        domain::FacetKind::Instruction => legacy_facet::FacetKind::Instruction,
        domain::FacetKind::Contract => legacy_facet::FacetKind::Contract,
    }
}

pub(crate) fn legacy_facet_summary_to_domain(
    summary: crate::adaptor::gateway::workflow::schema::FacetSummary,
) -> domain::FacetSummary {
    domain::FacetSummary {
        key: summary.key,
        kind: summary.kind,
        description: summary.description,
        builtin: summary.builtin,
    }
}

#[cfg(test)]
pub(crate) fn domain_facet_summary_to_legacy(
    summary: domain::FacetSummary,
) -> crate::adaptor::gateway::workflow::schema::FacetSummary {
    crate::adaptor::gateway::workflow::schema::FacetSummary {
        key: summary.key,
        kind: summary.kind,
        description: summary.description,
        builtin: summary.builtin,
    }
}

pub(crate) fn domain_event_draft_to_legacy(
    event: &WorkflowEventDraft,
) -> Result<legacy_event::WorkflowEvent, domain::WorkflowError> {
    match event.event_kind.as_str() {
        "run_started" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Payload {
                workflow_name: String,
                workflow_file_stem: String,
                worktree_path: String,
                workflow_definition: domain::WorkflowDefinition,
                #[allow(dead_code)]
                permission_mode: Option<String>,
            }

            let payload: Payload = parse_payload(event)?;
            Ok(legacy_event::WorkflowEvent::RunStarted {
                run_id: event.run_id.clone(),
                workflow_name: payload.workflow_name,
                workflow_file_stem: payload.workflow_file_stem,
                worktree_path: payload.worktree_path,
                workflow_definition: domain_workflow_to_legacy(&payload.workflow_definition)?,
                timestamp: event.timestamp,
            })
        }
        "run_aborted" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Payload {
                workflow_name: String,
            }

            let payload: Payload = parse_payload(event)?;
            Ok(legacy_event::WorkflowEvent::RunAborted {
                run_id: event.run_id.clone(),
                workflow_name: payload.workflow_name,
                timestamp: event.timestamp,
            })
        }
        "approval_resolved" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Payload {
                workflow_name: String,
                node_name: String,
                decision: String,
                comment: Option<String>,
            }

            let payload: Payload = parse_payload(event)?;
            let decision = match payload.decision.as_str() {
                "approve" => legacy_event::ApprovalDecisionRecord::Approve,
                "reject" => legacy_event::ApprovalDecisionRecord::Reject,
                "abort" => legacy_event::ApprovalDecisionRecord::Abort,
                other => {
                    return Err(domain::WorkflowError::validation(format!(
                        "unsupported approval decision: {other}"
                    )));
                }
            };
            Ok(legacy_event::WorkflowEvent::ApprovalResolved {
                run_id: event.run_id.clone(),
                workflow_name: payload.workflow_name,
                node_name: payload.node_name,
                decision,
                comment: payload.comment,
                timestamp: event.timestamp,
            })
        }
        "output_submitted" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Payload {
                workflow_name: String,
                node_name: String,
                contract: String,
                structured_output: serde_json::Value,
            }

            let payload: Payload = parse_payload(event)?;
            Ok(legacy_event::WorkflowEvent::OutputSubmitted {
                run_id: event.run_id.clone(),
                workflow_name: payload.workflow_name,
                node_name: payload.node_name,
                contract: payload.contract,
                structured_output: payload.structured_output,
                request_id: None,
                submitted_at: None,
                timestamp: event.timestamp,
            })
        }
        other => Err(domain::WorkflowError::validation(format!(
            "unsupported workflow event kind: {other}"
        ))),
    }
}

pub(crate) fn legacy_event_to_domain_draft(
    event: &legacy_event::WorkflowEvent,
) -> Result<WorkflowEventDraft, domain::WorkflowError> {
    let mut value = serde_json::to_value(event)
        .map_err(|e| domain::WorkflowError::external(format!("serialize workflow event: {e}")))?;
    let object = value.as_object_mut().ok_or_else(|| {
        domain::WorkflowError::external("workflow event did not serialize as object")
    })?;
    let event_kind = object
        .remove("event")
        .and_then(|value| value.as_str().map(str::to_string))
        .ok_or_else(|| domain::WorkflowError::external("workflow event missing event tag"))?;
    let timestamp = object
        .remove("timestamp")
        .and_then(|value| value.as_f64())
        .unwrap_or_default();
    object.remove("run_id");
    Ok(WorkflowEventDraft {
        run_id: event.run_id().to_string(),
        event_kind,
        timestamp,
        payload: value,
    })
}

fn parse_payload<T: for<'de> Deserialize<'de>>(
    event: &WorkflowEventDraft,
) -> Result<T, domain::WorkflowError> {
    serde_json::from_value(event.payload.clone()).map_err(|e| {
        domain::WorkflowError::validation(format!(
            "invalid payload for {} event: {e}",
            event.event_kind
        ))
    })
}

fn copy_domain_resolved_facets_to_legacy(
    definition: &domain::WorkflowDefinition,
    workflow: &mut crate::adaptor::gateway::workflow::schema::Workflow,
) {
    for (source, target) in definition.nodes.iter().zip(workflow.nodes.iter_mut()) {
        target.resolved_facets = domain_resolved_facets_to_legacy(&source.resolved_facets);
        if let (Some(source_children), Some(target_children)) = (
            source.parallel_children.as_ref(),
            target.parallel_children.as_mut(),
        ) {
            for (source_child, target_child) in
                source_children.iter().zip(target_children.iter_mut())
            {
                target_child.resolved_facets =
                    domain_resolved_facets_to_legacy(&source_child.resolved_facets);
            }
        }
    }
}

fn copy_legacy_resolved_facets_to_domain(
    workflow: &crate::adaptor::gateway::workflow::schema::Workflow,
    definition: &mut domain::WorkflowDefinition,
) {
    for (source, target) in workflow.nodes.iter().zip(definition.nodes.iter_mut()) {
        target.resolved_facets = legacy_resolved_facets_to_domain(&source.resolved_facets);
        if let (Some(source_children), Some(target_children)) = (
            source.parallel_children.as_ref(),
            target.parallel_children.as_mut(),
        ) {
            for (source_child, target_child) in
                source_children.iter().zip(target_children.iter_mut())
            {
                target_child.resolved_facets =
                    legacy_resolved_facets_to_domain(&source_child.resolved_facets);
            }
        }
    }
}

fn domain_resolved_facets_to_legacy(
    resolved: &ResolvedFacets,
) -> crate::adaptor::gateway::workflow::schema::ResolvedFacets {
    crate::adaptor::gateway::workflow::schema::ResolvedFacets {
        policy: resolved.policy.clone(),
        knowledge: resolved.knowledge.clone(),
        instruction: resolved.instruction.clone(),
        output_contract: resolved.output_contract.clone(),
        input_contracts: resolved.input_contracts.clone(),
    }
}

fn legacy_resolved_facets_to_domain(
    resolved: &crate::adaptor::gateway::workflow::schema::ResolvedFacets,
) -> ResolvedFacets {
    ResolvedFacets {
        policy: resolved.policy.clone(),
        knowledge: resolved.knowledge.clone(),
        instruction: resolved.instruction.clone(),
        output_contract: resolved.output_contract.clone(),
        input_contracts: resolved.input_contracts.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        NodeDefinition, NodeType, RunListFilter, RunStatus, RunStatusFilter, TriggerSource,
    };

    #[test]
    fn run_filter_all_maps_to_legacy_unfiltered_status() {
        let legacy = domain_run_filter_to_legacy(RunListFilter {
            status: Some(RunStatusFilter::All),
            worktree_path: Some("/repo".to_string()),
        });
        assert_eq!(legacy.status, None);
        assert_eq!(legacy.worktree_path.as_deref(), Some("/repo"));
    }

    #[test]
    fn domain_run_summary_serializes_like_existing_wire_shape() {
        let domain = domain::WorkflowRunSummary {
            run_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            task: None,
            status: RunStatus::Running,
            worktree_path: "/repo".to_string(),
            current_node_name: None,
            trigger_source: TriggerSource::DesktopUi,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
        };
        let legacy = domain_run_summary_to_legacy(domain.clone());

        assert_eq!(
            serde_json::to_value(domain).unwrap(),
            serde_json::to_value(legacy).unwrap()
        );
    }

    #[test]
    fn workflow_summary_serializes_like_existing_wire_shape() {
        let domain = domain::WorkflowSummary {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            is_running: true,
        };
        let legacy = domain_workflow_summary_to_legacy(domain.clone());

        assert_eq!(
            serde_json::to_value(domain).unwrap(),
            serde_json::to_value(legacy).unwrap()
        );
    }

    #[test]
    fn facet_summary_serializes_like_existing_wire_shape() {
        let domain = domain::FacetSummary {
            key: "coding".to_string(),
            kind: "policy".to_string(),
            description: "desc".to_string(),
            builtin: false,
        };
        let legacy = domain_facet_summary_to_legacy(domain.clone());

        assert_eq!(
            serde_json::to_value(domain).unwrap(),
            serde_json::to_value(legacy).unwrap()
        );
    }

    #[test]
    fn workflow_mapping_preserves_resolved_facets() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            variables: Default::default(),
            nodes: vec![NodeDefinition {
                name: "step".to_string(),
                node_type: NodeType::Agent,
                instruction: Some("inst".to_string()),
                resolved_facets: ResolvedFacets {
                    instruction: Some("resolved".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }],
        };

        let legacy = domain_workflow_to_legacy(&definition).unwrap();
        assert_eq!(
            legacy.nodes[0].resolved_facets.instruction.as_deref(),
            Some("resolved")
        );

        let mapped = legacy_workflow_to_domain(legacy).unwrap();
        assert_eq!(
            mapped.nodes[0].resolved_facets.instruction.as_deref(),
            Some("resolved")
        );
    }

    #[test]
    fn workflow_definition_serializes_like_existing_wire_shape() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            variables: Default::default(),
            nodes: vec![NodeDefinition {
                name: "implement".to_string(),
                node_type: NodeType::Agent,
                instruction: Some("inst".to_string()),
                input_contracts: Some(vec!["input".to_string()]),
                output_contract: Some("output".to_string()),
                transition_rules: vec![domain::TransitionRule {
                    r#match: "ok".to_string(),
                    next: "done".to_string(),
                }],
                ..Default::default()
            }],
        };
        let legacy = domain_workflow_to_legacy(&definition).unwrap();

        assert_eq!(
            serde_json::to_value(definition).unwrap(),
            serde_json::to_value(legacy).unwrap()
        );
    }

    #[test]
    fn run_started_draft_maps_to_existing_event_shape() {
        let event = WorkflowEventDraft {
            run_id: "00000000-0000-4000-8000-000000000001".to_string(),
            event_kind: "run_started".to_string(),
            timestamp: 10.0,
            payload: serde_json::json!({
                "workflowName": "wf",
                "workflowFileStem": "wf",
                "worktreePath": "/repo",
                "workflowDefinition": {
                    "name": "wf",
                    "description": "",
                    "nodes": [{
                        "name": "step",
                        "type": "agent"
                    }]
                },
                "permissionMode": "edit"
            }),
        };

        let legacy = domain_event_draft_to_legacy(&event).unwrap();
        let json = serde_json::to_value(&legacy).unwrap();
        assert_eq!(json["event"], "run_started");
        assert_eq!(json["workflow_name"], "wf");
    }
}
