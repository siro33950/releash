use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::domain::workflow as domain;
use crate::domain::workflow::services::contract_schema;
use crate::usecase::provider_dto::AgentSessionProviderDto;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowSourceFormatDto {
    #[default]
    Yaml,
    Lua,
}

impl From<domain::WorkflowSourceFormat> for WorkflowSourceFormatDto {
    fn from(value: domain::WorkflowSourceFormat) -> Self {
        match value {
            domain::WorkflowSourceFormat::Yaml => Self::Yaml,
            domain::WorkflowSourceFormat::Lua => Self::Lua,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDto {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(rename = "sourceFormat")]
    pub source_format: WorkflowSourceFormatDto,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemas: BTreeMap<String, serde_json::Value>,
    pub nodes: Vec<NodeDefinitionDto>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeKindDto {
    #[default]
    Session,
    Command,
    Fanout,
    Sequence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FacetRefsDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSpecDto {
    pub provider: AgentSessionProviderDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    pub facets: FacetRefsDto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompletionRequirementDto {
    Approval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeCompletionDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require: Option<CompletionRequirementDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegate: Option<SessionDelegateDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDelegateDto {
    pub child: String,
    pub inputs: Vec<ChildInputDto>,
    pub when: PredicateDto,
    pub max_iterations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputParamDto {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FanoutSpecDto {
    pub children: Vec<ChildEntryDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<ItemsSourceDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SequenceSpecDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    pub children: Vec<ChildEntryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChildEntryDto {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ChildInputDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<RuleDto>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChildInputDto {
    pub parameter: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ItemsSourceDto {
    Literal(Vec<serde_json::Value>),
    ArtifactField(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct NodeDefinitionDto {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<NodeCompletionDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PredicateDto {
    Ref(String),
    And { and: Vec<PredicateDto> },
    Or { or: Vec<PredicateDto> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RuleDto {
    When {
        on: PredicateDto,
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

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowSummaryDto {
    pub failure: Option<crate::domain::failure::WorkFailure>,
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub is_running: bool,
    pub source_format: WorkflowSourceFormatDto,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FacetSummaryDto {
    pub key: String,
    pub kind: String,
    pub description: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatusDto {
    Running,
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionOriginDto {
    DesktopUi,
    Cli,
    Agent,
    Api,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenUsageDto {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecutionSummaryDto {
    pub execution_id: String,
    pub workflow_name: String,
    pub status: ExecutionStatusDto,
    pub worktree_path: String,
    pub current_node: Option<String>,
    pub created_from: ExecutionOriginDto,
    pub started_at: f64,
    pub updated_at: f64,
    pub completed_at: Option<f64>,
    pub error_reason: Option<String>,
    pub total_token_usage: TokenUsageDto,
}

#[cfg(any(test, feature = "test-support"))]
#[cfg(test)]
pub fn workflow_to_dto(definition: &domain::WorkflowDefinition) -> WorkflowDto {
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
        source_format: source_format.into(),
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
        failure: summary.failure,
        name: summary.name,
        description: summary.description,
        builtin: summary.builtin,
        is_running: summary.is_running,
        source_format: summary.source_format.into(),
    }
}

pub fn facet_summary_to_dto(summary: domain::FacetSummary) -> FacetSummaryDto {
    FacetSummaryDto {
        key: summary.key,
        kind: summary.kind,
        description: summary.description,
        builtin: summary.builtin,
    }
}

pub fn workflow_execution_summary_to_dto(
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
        completion: completion_to_dto(&node.completion),
        worktree: node.worktree.map(|mode| {
            match mode {
                domain::WorktreeMode::Shared => "shared",
                domain::WorktreeMode::Isolated => "isolated",
            }
            .to_string()
        }),
    }
}

fn sequence_to_dto(sequence: &domain::SequenceSpec) -> SequenceSpecDto {
    SequenceSpecDto {
        entry: sequence.entry.clone(),
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
        provider: session.provider.into(),
        model: session.model.clone(),
        permission: session.permission.map(|permission| permission.to_string()),
        facets: facet_refs_to_dto(&session.facets),
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

fn completion_to_dto(completion: &domain::NodeCompletion) -> Option<NodeCompletionDto> {
    (!completion.is_empty()).then(|| NodeCompletionDto {
        require: completion.require.map(|require| match require {
            domain::CompletionRequirement::Approval => CompletionRequirementDto::Approval,
        }),
        delegate: completion
            .delegate
            .as_ref()
            .map(|delegate| SessionDelegateDto {
                child: delegate.child.clone(),
                inputs: delegate
                    .inputs
                    .iter()
                    .map(|(parameter, source)| ChildInputDto {
                        parameter: parameter.clone(),
                        source: source.raw().into(),
                    })
                    .collect(),
                when: predicate_to_dto(&delegate.when),
                max_iterations: delegate.max_iterations,
            }),
    })
}

fn facet_refs_to_dto(facets: &domain::FacetRefs) -> FacetRefsDto {
    FacetRefsDto {
        policy: facets.policy.clone(),
        knowledge: facets.knowledge.clone(),
        instruction: facets.instruction.clone(),
    }
}

fn predicate_to_dto(predicate: &domain::Predicate<String>) -> PredicateDto {
    match predicate {
        domain::Predicate::Ref(reference) => PredicateDto::Ref(reference.clone()),
        domain::Predicate::And(predicates) => PredicateDto::And {
            and: predicates.iter().map(predicate_to_dto).collect(),
        },
        domain::Predicate::Or(predicates) => PredicateDto::Or {
            or: predicates.iter().map(predicate_to_dto).collect(),
        },
    }
}

fn rule_to_dto(rule: &domain::Rule) -> RuleDto {
    match rule {
        domain::Rule::When { on, then, next } => RuleDto::When {
            on: predicate_to_dto(on),
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
        domain::ExecutionStatus::Completed => ExecutionStatusDto::Completed,
        domain::ExecutionStatus::Aborted => ExecutionStatusDto::Aborted,
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

#[cfg(test)]
#[path = "dto_test.rs"]
mod dto_tests;
