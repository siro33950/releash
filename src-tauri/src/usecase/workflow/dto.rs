use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::domain::workflow as domain;
use crate::domain::workflow::services::contract_schema;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkflowDto {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(rename = "sourceFormat")]
    pub source_format: domain::WorkflowSourceFormat,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemas: BTreeMap<String, serde_json::Value>,
    pub nodes: Vec<NodeDefinitionDto>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NodeKindDto {
    #[default]
    Session,
    Command,
    Fanout,
    Sequence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct FacetRefsDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct SessionSpecDto {
    pub provider: SessionProviderDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    pub facets: FacetRefsDto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionProviderDto {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NodeCompletionDto {
    #[default]
    Auto,
    Approval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct InputParamDto {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct FanoutSpecDto {
    pub children: Vec<ChildEntryDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<ItemsSourceDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct SequenceSpecDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub children: Vec<ChildEntryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct ChildEntryDto {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ChildInputDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<RuleDto>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ChildInputDto {
    pub parameter: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum ItemsSourceDto {
    Literal(Vec<serde_json::Value>),
    ArtifactField(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeDefinitionDto {
    pub name: String,
    pub kind: NodeKindDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionSpecDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fanout: Option<FanoutSpecDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<SequenceSpecDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<InputParamDto>,
    #[serde(default)]
    pub completion: NodeCompletionDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub(crate) enum RuleDto {
    When {
        on: String,
        then: String,
        next: String,
    },
    Switch {
        on: String,
        cases: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<String>,
    },
    LoopGuard {
        max_iterations: u32,
        on_exhausted: String,
    },
    Next {
        next: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct WorkflowSummaryDto {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    #[serde(default)]
    pub is_running: bool,
    #[serde(rename = "sourceFormat")]
    pub source_format: domain::WorkflowSourceFormat,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct FacetSummaryDto {
    pub key: String,
    pub kind: String,
    pub description: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionStatusDto {
    Running,
    WaitingApproval,
    Completed,
    Aborted,
    Interrupted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionOriginDto {
    DesktopUi,
    Cli,
    Agent,
    Api,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionInterruptionReasonDto {
    Crash,
    Stale,
    Stop,
    Orphan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TokenUsageDto {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowExecutionSummaryDto {
    pub execution_id: String,
    pub workflow_name: String,
    pub status: ExecutionStatusDto,
    pub worktree_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node: Option<String>,
    pub created_from: ExecutionOriginDto,
    pub started_at: f64,
    pub updated_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interruption_reason: Option<ExecutionInterruptionReasonDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_from_node: Option<String>,
    pub total_token_usage: TokenUsageDto,
}

pub(crate) fn workflow_to_dto(definition: &domain::WorkflowDefinition) -> WorkflowDto {
    workflow_to_dto_with_source_format(definition, domain::WorkflowSourceFormat::Yaml)
}

pub(crate) fn workflow_to_dto_with_source_format(
    definition: &domain::WorkflowDefinition,
    source_format: domain::WorkflowSourceFormat,
) -> WorkflowDto {
    WorkflowDto {
        name: definition.name.clone(),
        description: definition.description.clone(),
        builtin: definition.builtin,
        source_format,
        schemas: definition
            .schemas
            .iter()
            .map(|(name, schema)| {
                (
                    name.clone(),
                    contract_schema::schema_def_to_json_value(schema),
                )
            })
            .collect(),
        nodes: definition.nodes.iter().map(node_to_dto).collect(),
    }
}

pub(crate) fn workflow_summary_to_dto(summary: domain::WorkflowSummary) -> WorkflowSummaryDto {
    WorkflowSummaryDto {
        name: summary.name,
        description: summary.description,
        builtin: summary.builtin,
        is_running: summary.is_running,
        source_format: summary.source_format,
    }
}

pub(crate) fn facet_summary_to_dto(summary: domain::FacetSummary) -> FacetSummaryDto {
    FacetSummaryDto {
        key: summary.key,
        kind: summary.kind,
        description: summary.description,
        builtin: summary.builtin,
    }
}

pub(crate) fn workflow_execution_summary_to_dto(
    summary: domain::WorkflowExecutionSummary,
) -> WorkflowExecutionSummaryDto {
    WorkflowExecutionSummaryDto {
        execution_id: summary.execution_id,
        workflow_name: summary.workflow_name,
        status: execution_status_to_dto(summary.status),
        worktree_path: summary.worktree_path,
        current_node: summary.current_node,
        created_from: execution_origin_to_dto(summary.created_from),
        started_at: summary.started_at,
        updated_at: summary.updated_at,
        completed_at: summary.completed_at,
        error_reason: summary.error_reason,
        interruption_reason: summary
            .interruption_reason
            .map(execution_interruption_reason_to_dto),
        resume_from_node: summary.resume_from_node,
        total_token_usage: TokenUsageDto {
            input_tokens: summary.total_token_usage.input_tokens,
            output_tokens: summary.total_token_usage.output_tokens,
        },
    }
}

fn node_to_dto(node: &domain::NodeDefinition) -> NodeDefinitionDto {
    NodeDefinitionDto {
        name: node.name.clone(),
        kind: node_kind_to_dto(node.kind_name()),
        command: node.command().map(str::to_string),
        session: node.session().map(session_to_dto),
        fanout: node.fanout().map(fanout_to_dto),
        sequence: node.sequence().map(sequence_to_dto),
        artifact: node.artifact.clone(),
        input: node.input.iter().map(input_param_to_dto).collect(),
        completion: completion_to_dto(node.completion),
        worktree: node.worktree.clone(),
    }
}

fn sequence_to_dto(sequence: &domain::SequenceSpec) -> SequenceSpecDto {
    SequenceSpecDto {
        entry: sequence.entry.clone(),
        output: sequence.output.clone(),
        children: sequence.children.iter().map(child_entry_to_dto).collect(),
    }
}

fn child_entry_to_dto(entry: &domain::ChildEntry) -> ChildEntryDto {
    ChildEntryDto {
        name: entry.name.clone(),
        inputs: entry
            .inputs
            .iter()
            .map(|(parameter, source)| ChildInputDto {
                parameter: parameter.clone(),
                source: source.raw().to_string(),
            })
            .collect(),
        rules: entry
            .rules
            .as_ref()
            .map(|rules| rules.iter().map(rule_to_dto).collect()),
    }
}

fn input_param_to_dto(param: &domain::InputParam) -> InputParamDto {
    InputParamDto {
        name: param.name.clone(),
        contract: param.contract.clone(),
    }
}

fn session_to_dto(session: &domain::SessionSpec) -> SessionSpecDto {
    SessionSpecDto {
        provider: provider_to_dto(session.provider),
        model: session.model.clone(),
        permission: session.permission.map(|permission| permission.to_string()),
        facets: facet_refs_to_dto(&session.facets),
    }
}

fn provider_to_dto(
    provider: crate::domain::provider_lifecycle::ProviderKind,
) -> SessionProviderDto {
    match provider {
        crate::domain::provider_lifecycle::ProviderKind::Claude => SessionProviderDto::Claude,
        crate::domain::provider_lifecycle::ProviderKind::Codex => SessionProviderDto::Codex,
    }
}

fn fanout_to_dto(fanout: &domain::FanoutSpec) -> FanoutSpecDto {
    FanoutSpecDto {
        children: fanout.children.iter().map(child_entry_to_dto).collect(),
        items: fanout.items.as_ref().map(items_source_to_dto),
    }
}

fn items_source_to_dto(items: &domain::ItemsSource) -> ItemsSourceDto {
    match items {
        domain::ItemsSource::Literal(values) => ItemsSourceDto::Literal(values.clone()),
        domain::ItemsSource::ArtifactField { node, field_path } => {
            ItemsSourceDto::ArtifactField(format!("{node}.{}", field_path.as_string()))
        }
    }
}

fn node_kind_to_dto(kind: domain::NodeKindName) -> NodeKindDto {
    match kind {
        domain::NodeKindName::Command => NodeKindDto::Command,
        domain::NodeKindName::Session => NodeKindDto::Session,
        domain::NodeKindName::Fanout => NodeKindDto::Fanout,
        domain::NodeKindName::Sequence => NodeKindDto::Sequence,
    }
}

fn completion_to_dto(completion: domain::NodeCompletion) -> NodeCompletionDto {
    match completion {
        domain::NodeCompletion::Auto => NodeCompletionDto::Auto,
        domain::NodeCompletion::Approval => NodeCompletionDto::Approval,
    }
}

fn facet_refs_to_dto(facets: &domain::FacetRefs) -> FacetRefsDto {
    FacetRefsDto {
        policy: facets.policy.clone(),
        knowledge: facets.knowledge.clone(),
        instruction: facets.instruction.clone(),
    }
}

fn rule_to_dto(rule: &domain::Rule) -> RuleDto {
    match rule {
        domain::Rule::When { on, then, next } => RuleDto::When {
            on: on.clone(),
            then: then.clone(),
            next: next.clone(),
        },
        domain::Rule::Switch { on, cases, next } => RuleDto::Switch {
            on: on.clone(),
            cases: cases.clone(),
            next: next.clone(),
        },
        domain::Rule::LoopGuard {
            max_iterations,
            on_exhausted,
        } => RuleDto::LoopGuard {
            max_iterations: *max_iterations,
            on_exhausted: on_exhausted.clone(),
        },
        domain::Rule::Next(next) => RuleDto::Next { next: next.clone() },
    }
}

fn execution_status_to_dto(status: domain::ExecutionStatus) -> ExecutionStatusDto {
    match status {
        domain::ExecutionStatus::Running => ExecutionStatusDto::Running,
        #[cfg(test)]
        domain::ExecutionStatus::WaitingApproval => ExecutionStatusDto::WaitingApproval,
        domain::ExecutionStatus::Completed => ExecutionStatusDto::Completed,
        domain::ExecutionStatus::Aborted => ExecutionStatusDto::Aborted,
        #[cfg(test)]
        domain::ExecutionStatus::Interrupted => ExecutionStatusDto::Interrupted,
    }
}

fn execution_origin_to_dto(source: domain::ExecutionOrigin) -> ExecutionOriginDto {
    match source {
        domain::ExecutionOrigin::DesktopUi => ExecutionOriginDto::DesktopUi,
        domain::ExecutionOrigin::Api => ExecutionOriginDto::Api,
        domain::ExecutionOrigin::Cli => ExecutionOriginDto::Cli,
        domain::ExecutionOrigin::Agent => ExecutionOriginDto::Agent,
    }
}

fn execution_interruption_reason_to_dto(
    reason: domain::ExecutionInterruptionReason,
) -> ExecutionInterruptionReasonDto {
    match reason {
        domain::ExecutionInterruptionReason::Crash => ExecutionInterruptionReasonDto::Crash,
        domain::ExecutionInterruptionReason::Stale => ExecutionInterruptionReasonDto::Stale,
        domain::ExecutionInterruptionReason::Stop => ExecutionInterruptionReasonDto::Stop,
        domain::ExecutionInterruptionReason::Orphan => ExecutionInterruptionReasonDto::Orphan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_dto_serializes_like_canonical_wire_shape() {
        let workflow = WorkflowDto {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            source_format: domain::WorkflowSourceFormat::Yaml,
            schemas: [(
                "plan".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinitionDto {
                name: "node".to_string(),
                kind: NodeKindDto::Session,
                session: Some(SessionSpecDto {
                    provider: SessionProviderDto::Claude,
                    model: None,
                    permission: None,
                    facets: FacetRefsDto {
                        instruction: Some("inst".to_string()),
                        ..Default::default()
                    },
                }),
                artifact: Some("plan".to_string()),
                input: vec![InputParamDto {
                    name: "item".to_string(),
                    contract: Some("plan".to_string()),
                }],
                ..Default::default()
            }],
        };

        assert_eq!(
            serde_json::to_value(workflow).unwrap(),
            serde_json::json!({
                "name": "wf",
                "description": "desc",
                "builtin": false,
                "sourceFormat": "yaml",
                "schemas": {
                    "plan": {
                        "type": "object",
                        "properties": {},
                        "required": []
                    }
                },
                "nodes": [{
                    "name": "node",
                    "kind": "session",
                    "session": {
                        "provider": "claude",
                        "facets": {
                            "instruction": "inst"
                        }
                    },
                    "artifact": "plan",
                    "input": [{"name": "item", "contract": "plan"}],
                    "completion": "auto"
                }]
            })
        );
    }

    #[test]
    fn workflow_dto_exposes_lua_only_as_definition_source_metadata() {
        let workflow = domain::WorkflowDefinition {
            name: "lua-workflow".to_string(),
            description: "Lua".to_string(),
            ..domain::WorkflowDefinition::default()
        };

        let value = serde_json::to_value(workflow_to_dto_with_source_format(
            &workflow,
            domain::WorkflowSourceFormat::Lua,
        ))
        .unwrap();

        assert_eq!(value["sourceFormat"], "lua");
        assert!(serde_json::to_value(workflow)
            .unwrap()
            .get("sourceFormat")
            .is_none());
    }

    #[test]
    fn workflow_to_dto_maps_knowledge_refs_to_ordered_json_array() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            nodes: vec![domain::NodeDefinition {
                name: "review".to_string(),
                kind: domain::NodeKind::Session(domain::SessionSpec {
                    provider: crate::domain::provider_lifecycle::ProviderKind::Codex,
                    permission: Some(domain::SessionPermission::ReadOnly),
                    facets: domain::FacetRefs {
                        knowledge: vec!["knowledge-a".to_string(), "knowledge-b".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            entry: "review".to_string(),
            ..Default::default()
        };

        let dto = workflow_to_dto(&definition);

        assert_eq!(
            dto.nodes[0].session.as_ref().unwrap().facets.knowledge,
            vec!["knowledge-a", "knowledge-b"]
        );
        assert_eq!(
            serde_json::to_value(dto).unwrap()["nodes"][0]["session"]["facets"]["knowledge"],
            serde_json::json!(["knowledge-a", "knowledge-b"])
        );
        assert_eq!(
            serde_json::to_value(workflow_to_dto(&definition)).unwrap()["nodes"][0]["session"]
                ["provider"],
            serde_json::json!("codex")
        );
        assert_eq!(
            serde_json::to_value(workflow_to_dto(&definition)).unwrap()["nodes"][0]["session"]
                ["permission"],
            serde_json::json!("read-only")
        );
    }

    #[test]
    fn workflow_to_dto_preserves_loop_guard() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            nodes: vec![
                domain::NodeDefinition {
                    name: "main".to_string(),
                    kind: domain::NodeKind::Sequence(domain::SequenceSpec {
                        entry: None,
                        output: None,
                        children: vec![domain::ChildEntry {
                            on_failure: None,
                            name: "fix".to_string(),
                            inputs: Vec::new(),
                            rules: Some(vec![domain::Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "done".to_string(),
                            }]),
                        }],
                    }),
                    ..Default::default()
                },
                domain::NodeDefinition {
                    name: "fix".to_string(),
                    ..Default::default()
                },
            ],
            entry: "main".to_string(),
            ..Default::default()
        };

        let dto = workflow_to_dto(&definition);

        assert_eq!(
            serde_json::to_value(dto).unwrap()["nodes"][0]["sequence"]["children"][0]["rules"][0],
            serde_json::json!({
                "type": "loop_guard",
                "max_iterations": 2,
                "on_exhausted": "done"
            })
        );
    }

    #[test]
    fn fanout_spec_dto_serializes_child_and_items_sources() {
        let literal = FanoutSpecDto {
            children: vec![ChildEntryDto {
                name: "review".to_string(),
                inputs: Vec::new(),
                rules: None,
            }],
            items: Some(ItemsSourceDto::Literal(vec![serde_json::json!({
                "thread_id": "thread-1"
            })])),
        };
        assert_eq!(
            serde_json::to_value(literal).unwrap(),
            serde_json::json!({
                "children": [{"name": "review"}],
                "items": [{"thread_id": "thread-1"}]
            })
        );

        let reference = FanoutSpecDto {
            children: vec![
                ChildEntryDto {
                    name: "review-opus".to_string(),
                    inputs: Vec::new(),
                    rules: None,
                },
                ChildEntryDto {
                    name: "review-gpt".to_string(),
                    inputs: Vec::new(),
                    rules: None,
                },
            ],
            items: Some(ItemsSourceDto::ArtifactField("scan.threads".to_string())),
        };
        assert_eq!(
            serde_json::to_value(reference).unwrap(),
            serde_json::json!({
                "children": [{"name": "review-opus"}, {"name": "review-gpt"}],
                "items": "scan.threads"
            })
        );
    }

    #[test]
    fn execution_summary_dto_serializes_like_canonical_wire_shape() {
        let summary = workflow_execution_summary_to_dto(domain::WorkflowExecutionSummary {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            status: domain::ExecutionStatus::Interrupted,
            worktree_path: "/repo".to_string(),
            current_node: None,
            created_from: domain::ExecutionOrigin::DesktopUi,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: Some(domain::ExecutionInterruptionReason::Stop),
            resume_from_node: Some("review".to_string()),
            total_token_usage: domain::TokenUsage {
                input_tokens: 13,
                output_tokens: 8,
            },
        });

        assert_eq!(
            serde_json::to_value(summary).unwrap(),
            serde_json::json!({
                "executionId": "00000000-0000-4000-8000-000000000001",
                "workflowName": "wf",
                "status": "interrupted",
                "worktreePath": "/repo",
                "createdFrom": "desktop_ui",
                "startedAt": 1.0,
                "updatedAt": 2.0,
                "interruptionReason": "stop",
                "resumeFromNode": "review",
                "totalTokenUsage": {
                    "inputTokens": 13,
                    "outputTokens": 8
                }
            })
        );
    }
}
