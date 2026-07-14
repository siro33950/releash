#[cfg(test)]
use serde::Deserialize;

use crate::adaptor::gateway::workflow::{
    event as legacy_event, facet as legacy_facet, run as legacy_run,
};
use crate::domain::workflow as domain;
use crate::usecase::workflow::ports::WorkflowEventDraft;

#[cfg(test)]
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
        None => None,
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
        domain::RunStatus::Interrupted => legacy_run::RunStatus::Interrupted,
    }
}

fn legacy_run_status_to_domain(status: legacy_run::RunStatus) -> domain::RunStatus {
    match status {
        legacy_run::RunStatus::Running => domain::RunStatus::Running,
        legacy_run::RunStatus::WaitingApproval => domain::RunStatus::WaitingApproval,
        legacy_run::RunStatus::Completed => domain::RunStatus::Completed,
        legacy_run::RunStatus::Failed => domain::RunStatus::Failed,
        legacy_run::RunStatus::Aborted => domain::RunStatus::Aborted,
        legacy_run::RunStatus::Interrupted => domain::RunStatus::Interrupted,
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
    Ok(crate::adaptor::gateway::workflow::schema::Workflow {
        name: definition.name.clone(),
        description: definition.description.clone(),
        builtin: definition.builtin,
        schemas: definition
            .schemas
            .iter()
            .map(|(name, schema)| (name.clone(), domain_schema_to_legacy(schema)))
            .collect(),
        nodes: definition.nodes.iter().map(domain_node_to_legacy).collect(),
    })
}

pub(crate) fn legacy_workflow_to_domain(
    workflow: crate::adaptor::gateway::workflow::schema::Workflow,
) -> Result<domain::WorkflowDefinition, domain::WorkflowError> {
    Ok(crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain(&workflow))
}

fn domain_node_to_legacy(
    node: &domain::NodeDefinition,
) -> crate::adaptor::gateway::workflow::schema::NodeDefinition {
    crate::adaptor::gateway::workflow::schema::NodeDefinition {
        name: node.name.clone(),
        kind: domain_kind_to_legacy(&node.kind),
        artifact: node.artifact.clone(),
        input: node.input.clone(),
        inputs: node.inputs.clone(),
        rules: node
            .rules
            .iter()
            .cloned()
            .map(domain_rule_to_legacy)
            .collect(),
    }
}

fn domain_kind_to_legacy(
    kind: &domain::NodeKind,
) -> crate::adaptor::gateway::workflow::schema::NodeKind {
    match kind {
        domain::NodeKind::Command(spec) => {
            crate::adaptor::gateway::workflow::schema::NodeKind::Command(
                crate::adaptor::gateway::workflow::schema::CommandSpec {
                    command: spec.command.clone(),
                },
            )
        }
        domain::NodeKind::Session(spec) => {
            crate::adaptor::gateway::workflow::schema::NodeKind::Session(
                crate::adaptor::gateway::workflow::schema::SessionSpec {
                    model: spec.model.clone(),
                    permission: spec.permission.clone(),
                    gate: domain_gate_to_legacy(spec.gate),
                    facets: domain_facets_to_legacy(&spec.facets),
                },
            )
        }
        domain::NodeKind::Fanout(spec) => {
            crate::adaptor::gateway::workflow::schema::NodeKind::Fanout(
                crate::adaptor::gateway::workflow::schema::FanoutSpec {
                    child: spec.child.clone(),
                    items: spec.items.as_ref().map(domain_items_source_to_schema),
                },
            )
        }
    }
}

fn domain_gate_to_legacy(
    gate: domain::SessionGate,
) -> crate::adaptor::gateway::workflow::schema::SessionGate {
    match gate {
        domain::SessionGate::Auto => crate::adaptor::gateway::workflow::schema::SessionGate::Auto,
        domain::SessionGate::Approval => {
            crate::adaptor::gateway::workflow::schema::SessionGate::Approval
        }
    }
}

fn domain_facets_to_legacy(
    facets: &domain::FacetRefs,
) -> crate::adaptor::gateway::workflow::schema::FacetRefs {
    crate::adaptor::gateway::workflow::schema::FacetRefs {
        policy: facets.policy.clone(),
        knowledge: facets.knowledge.clone(),
        instruction: facets.instruction.clone(),
    }
}

fn domain_items_source_to_schema(
    items: &domain::ItemsSource,
) -> crate::adaptor::gateway::workflow::schema::ItemsSource {
    match items {
        domain::ItemsSource::Literal(values) => {
            crate::adaptor::gateway::workflow::schema::ItemsSource::Literal(values.clone())
        }
        domain::ItemsSource::ArtifactField { node, field } => {
            crate::adaptor::gateway::workflow::schema::ItemsSource::ArtifactField {
                node: node.clone(),
                field: field.clone(),
            }
        }
    }
}

fn domain_rule_to_legacy(rule: domain::Rule) -> crate::adaptor::gateway::workflow::schema::Rule {
    match rule {
        domain::Rule::When { on, then, next } => {
            crate::adaptor::gateway::workflow::schema::Rule::When { on, then, next }
        }
        domain::Rule::Switch { on, cases, next } => {
            crate::adaptor::gateway::workflow::schema::Rule::Switch { on, cases, next }
        }
        domain::Rule::LoopGuard {
            max_iterations,
            on_exhausted,
        } => crate::adaptor::gateway::workflow::schema::Rule::LoopGuard {
            max_iterations,
            on_exhausted,
        },
        domain::Rule::Next(next) => crate::adaptor::gateway::workflow::schema::Rule::Next(next),
    }
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

#[cfg(test)]
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
                request: String,
                workflow_definition: crate::adaptor::gateway::workflow::schema::Workflow,
            }

            let payload: Payload = parse_payload(event)?;
            Ok(legacy_event::WorkflowEvent::RunStarted {
                run_id: event.run_id.clone(),
                workflow_name: payload.workflow_name,
                workflow_file_stem: payload.workflow_file_stem,
                worktree_path: payload.worktree_path,
                request: payload.request,
                workflow_definition: payload.workflow_definition,
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
                aborted_step: None,
                timestamp: event.timestamp,
            })
        }
        "run_interrupted" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Payload {
                workflow_name: String,
                reason: String,
            }

            let payload: Payload = parse_payload(event)?;
            Ok(legacy_event::WorkflowEvent::RunInterrupted {
                run_id: event.run_id.clone(),
                workflow_name: payload.workflow_name,
                reason: payload.reason,
                timestamp: event.timestamp,
            })
        }
        "approval_resolved" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Payload {
                workflow_name: String,
                node_execution_id: String,
                node_name: String,
                comment: Option<String>,
            }

            let payload: Payload = parse_payload(event)?;
            Ok(legacy_event::WorkflowEvent::ApprovalResolved {
                run_id: event.run_id.clone(),
                workflow_name: payload.workflow_name,
                node_execution_id: payload.node_execution_id,
                node_name: payload.node_name,
                comment: payload.comment,
                timestamp: event.timestamp,
            })
        }
        "artifact_produced" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Payload {
                workflow_name: String,
                node_execution_id: String,
                node_name: String,
                contract: Option<String>,
                value: serde_json::Value,
            }

            let payload: Payload = parse_payload(event)?;
            Ok(legacy_event::WorkflowEvent::ArtifactProduced {
                run_id: event.run_id.clone(),
                workflow_name: payload.workflow_name,
                node_execution_id: payload.node_execution_id,
                node_name: payload.node_name,
                contract: payload.contract,
                value: payload.value,
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

#[cfg(test)]
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

fn domain_schema_to_legacy(
    schema: &domain::SchemaDef,
) -> crate::adaptor::gateway::workflow::schema::SchemaDef {
    match schema {
        domain::SchemaDef::Object {
            properties,
            required,
            additional_properties,
        } => crate::adaptor::gateway::workflow::schema::SchemaDef::Object {
            properties: properties
                .iter()
                .map(|(name, schema)| (name.clone(), domain_schema_to_legacy(schema)))
                .collect(),
            required: required.clone(),
            additional_properties: *additional_properties,
        },
        domain::SchemaDef::Array { items } => {
            crate::adaptor::gateway::workflow::schema::SchemaDef::Array {
                items: items.clone(),
            }
        }
        domain::SchemaDef::String { r#enum } => {
            crate::adaptor::gateway::workflow::schema::SchemaDef::String {
                r#enum: r#enum.clone(),
            }
        }
        domain::SchemaDef::Boolean => crate::adaptor::gateway::workflow::schema::SchemaDef::Boolean,
        domain::SchemaDef::Integer => crate::adaptor::gateway::workflow::schema::SchemaDef::Integer,
        domain::SchemaDef::Number => crate::adaptor::gateway::workflow::schema::SchemaDef::Number,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        FacetRefs, FanoutSpec, ItemsSource, NodeDefinition, NodeKind, RunListFilter, RunStatus,
        SessionSpec, TriggerSource,
    };

    #[test]
    fn run_filter_without_status_maps_to_legacy_unfiltered_status() {
        let legacy = domain_run_filter_to_legacy(RunListFilter {
            status: None,
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
            serde_json::to_value(legacy).unwrap(),
            serde_json::json!({
                "runId": "00000000-0000-4000-8000-000000000001",
                "workflowName": "wf",
                "status": "running",
                "worktreePath": "/repo",
                "triggerSource": "desktop_ui",
                "startedAt": 1.0,
                "updatedAt": 2.0
            })
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
            serde_json::to_value(legacy).unwrap(),
            serde_json::json!({
                "name": "wf",
                "description": "desc",
                "builtin": false,
                "is_running": true
            })
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
            serde_json::to_value(legacy).unwrap(),
            serde_json::json!({
                "key": "coding",
                "kind": "policy",
                "description": "desc",
                "builtin": false
            })
        );
    }

    #[test]
    fn workflow_mapping_preserves_facet_refs_without_runtime_contents() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "step".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("inst".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };

        let legacy = domain_workflow_to_legacy(&definition).unwrap();
        assert_eq!(
            legacy.nodes[0]
                .session()
                .unwrap()
                .facets
                .instruction
                .as_deref(),
            Some("inst")
        );

        let mapped = legacy_workflow_to_domain(legacy).unwrap();
        assert_eq!(
            mapped.nodes[0]
                .session()
                .unwrap()
                .facets
                .instruction
                .as_deref(),
            Some("inst")
        );
    }

    #[test]
    fn workflow_mapping_round_trips_fanout_child_and_literal_items() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "fanout".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    child: vec!["worker".to_string()],
                    items: Some(ItemsSource::Literal(vec![serde_json::json!({
                        "path": "src/lib.rs"
                    })])),
                }),
                ..Default::default()
            }],
        };

        let legacy = domain_workflow_to_legacy(&definition).unwrap();
        assert_eq!(
            legacy.nodes[0].fanout().unwrap().items,
            Some(
                crate::adaptor::gateway::workflow::schema::ItemsSource::Literal(vec![
                    serde_json::json!({"path": "src/lib.rs"}),
                ])
            )
        );

        let mapped = legacy_workflow_to_domain(legacy).unwrap();
        assert_eq!(mapped, definition);
    }

    #[test]
    fn workflow_definition_serializes_like_existing_wire_shape() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: [(
                "plan".to_string(),
                domain::SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                    additional_properties: false,
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinition {
                name: "implement".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("inst".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                input: Some("plan".to_string()),
                artifact: Some("plan".to_string()),
                rules: vec![domain::Rule::Next("done".to_string())],
                ..Default::default()
            }],
        };
        let legacy = domain_workflow_to_legacy(&definition).unwrap();

        assert_eq!(
            serde_json::to_value(legacy).unwrap(),
            serde_json::json!({
                "name": "wf",
                "description": "desc",
                "builtin": false,
                "schemas": {
                    "plan": {
                        "type": "object",
                        "additionalProperties": false
                    }
                },
                "nodes": [{
                    "name": "implement",
                    "session": {
                        "gate": "auto",
                        "facets": {
                            "instruction": "inst"
                        }
                    },
                    "artifact": "plan",
                    "input": "plan",
                    "rules": [{"next": "done"}]
                }]
            })
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
                "request": "ship feature",
                "workflowDefinition": {
                    "name": "wf",
                    "description": "",
                    "nodes": [{
                        "name": "step",
                        "session": {
                            "gate": "auto"
                        }
                    }]
                },
                "permissionMode": "edit"
            }),
        };

        let legacy = domain_event_draft_to_legacy(&event).unwrap();
        let json = serde_json::to_value(&legacy).unwrap();
        assert_eq!(json["event"], "run_started");
        assert_eq!(json["workflow_name"], "wf");
        assert_eq!(json["request"], "ship feature");
    }
}
