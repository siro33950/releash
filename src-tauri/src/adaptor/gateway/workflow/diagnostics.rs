use crate::adaptor::gateway::workflow::builtin;
use crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain;
use crate::adaptor::gateway::workflow::facet::{self, FacetKind};
use crate::adaptor::gateway::workflow::schema::{NodeDefinition, Rule, WorkflowDefinitionYaml};
use crate::adaptor::gateway::workflow::span_map::{DiagnosticSpan, YamlSpanMap};
use crate::domain::workflow::validation;
use crate::domain::workflow::validation::{
    InvalidArtifactReferenceKind, InvalidRuleKind, InvalidSchemaKind,
};
use crate::infrastructure::runtime::workflow_host::prompt_rendering;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

const ALL_FACET_KINDS: [FacetKind; 3] = [
    FacetKind::Policy,
    FacetKind::Knowledge,
    FacetKind::Instruction,
];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticItem {
    pub code: String,
    pub severity: Severity,
    pub stage: DiagnosticStage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<DiagnosticSpan>,
    pub message: String,
    /// 対象の workflow 名（ファセット診断の場合は None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
    /// 対象の node 名
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// 対象のファセットキー（ファセット診断の場合）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_key: Option<String>,
    /// 対象のファセット種別
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_kind: Option<String>,
    /// 対象フィールド
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStage {
    ParseShape,
    Resolve,
    Typecheck,
    ControlFlow,
}

impl DiagnosticItem {
    pub(crate) fn new(
        code: impl Into<String>,
        severity: Severity,
        stage: DiagnosticStage,
        span: Option<DiagnosticSpan>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            stage,
            span,
            message: message.into(),
            workflow_name: None,
            node_name: None,
            facet_key: None,
            facet_kind: None,
            field: None,
        }
    }

    fn workflow(mut self, name: impl Into<String>) -> Self {
        self.workflow_name = Some(name.into());
        self
    }

    fn node(mut self, name: impl Into<String>) -> Self {
        self.node_name = Some(name.into());
        self
    }

    fn facet(mut self, key: impl Into<String>, kind: impl Into<String>) -> Self {
        self.facet_key = Some(key.into());
        self.facet_kind = Some(kind.into());
        self
    }

    fn field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DiagnosticSummary {
    pub error_count: usize,
    pub info_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReport {
    pub items: Vec<DiagnosticItem>,
    /// workflow名 → そのworkflowの診断サマリ
    pub workflow_summaries: HashMap<String, DiagnosticSummary>,
    /// "kind/key" → そのファセットの診断サマリ
    pub facet_summaries: HashMap<String, DiagnosticSummary>,
    /// ファセットキー → 参照元 workflow/node 情報のリスト
    pub facet_usage: HashMap<String, Vec<FacetUsageEntry>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FacetUsageEntry {
    pub workflow_name: String,
    pub node_name: String,
    pub slot: String,
}

type LoadedWorkflowDiagnostics =
    Result<(WorkflowDefinitionYaml, Vec<DiagnosticItem>), Vec<DiagnosticItem>>;
type NamedWorkflowDiagnostics = (String, LoadedWorkflowDiagnostics);

#[derive(Debug, Clone)]
pub(crate) struct WorkflowSourceDiagnostics {
    pub(crate) workflow: Option<WorkflowDefinitionYaml>,
    pub(crate) diagnostics: Vec<DiagnosticItem>,
}

impl WorkflowSourceDiagnostics {
    pub(crate) fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|item| item.severity == Severity::Error)
    }
}

pub(crate) fn diagnose_workflow_source(
    source: &str,
    workflow_name_hint: Option<&str>,
) -> WorkflowSourceDiagnostics {
    let span_map = match YamlSpanMap::parse(source) {
        Ok(span_map) => span_map,
        Err(error) => {
            return WorkflowSourceDiagnostics {
                workflow: None,
                diagnostics: vec![DiagnosticItem::new(
                    "WFS001",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    Some(DiagnosticSpan::from_scan_error(&error)),
                    format!("YAML syntax error: {error}"),
                )
                .workflow(workflow_name_hint.unwrap_or("<unknown>"))],
            };
        }
    };

    let raw_value = serde_saphyr::from_str::<serde_json::Value>(source).ok();
    let mut diagnostics = raw_value
        .as_ref()
        .map(|value| parse_shape_diagnostics(value, &span_map, workflow_name_hint))
        .unwrap_or_default();

    if diagnostics
        .iter()
        .any(|item| item.severity == Severity::Error)
    {
        normalize_invalid_source_workflow_name(&mut diagnostics, workflow_name_hint);
        return WorkflowSourceDiagnostics {
            workflow: None,
            diagnostics,
        };
    }

    let workflow = match serde_saphyr::from_str::<WorkflowDefinitionYaml>(source) {
        Ok(workflow) => workflow,
        Err(error) => {
            diagnostics.push(deserialize_error_diagnostic(
                &error,
                &span_map,
                workflow_name_hint,
            ));
            return WorkflowSourceDiagnostics {
                workflow: None,
                diagnostics,
            };
        }
    };

    diagnostics.extend(diagnose_workflow_definition(&workflow, Some(&span_map)));
    WorkflowSourceDiagnostics {
        workflow: Some(workflow),
        diagnostics,
    }
}

pub(crate) fn diagnose_workflow_definition(
    wf: &WorkflowDefinitionYaml,
    span_map: Option<&YamlSpanMap>,
) -> Vec<DiagnosticItem> {
    let mut items = Vec::new();
    let workflow = workflow_definition_to_domain(wf);
    for error in validation::validate_all(&workflow) {
        items.push(validation_error_to_diagnostic(wf, &error, span_map));
    }
    items
}

fn normalize_invalid_source_workflow_name(
    diagnostics: &mut [DiagnosticItem],
    workflow_name_hint: Option<&str>,
) {
    let Some(name) = workflow_name_hint else {
        return;
    };
    for item in diagnostics {
        item.workflow_name = Some(name.to_string());
    }
}

fn parse_shape_diagnostics(
    value: &serde_json::Value,
    span_map: &YamlSpanMap,
    workflow_name_hint: Option<&str>,
) -> Vec<DiagnosticItem> {
    let mut diagnostics = Vec::new();
    let workflow_name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .or(workflow_name_hint)
        .unwrap_or("<unknown>");
    let Some(root) = value.as_object() else {
        diagnostics.push(
            DiagnosticItem::new(
                "WFS002",
                Severity::Error,
                DiagnosticStage::ParseShape,
                span_map.nearest_span(""),
                "workflow YAML must be a mapping",
            )
            .workflow(workflow_name),
        );
        return diagnostics;
    };

    check_allowed_fields(
        root,
        "",
        &["name", "description", "builtin", "schemas", "nodes"],
        &["steps", "variables", "workflow_variables", "tasks"],
        span_map,
        workflow_name,
        None,
        &mut diagnostics,
    );

    if let Some(name) = root.get("name").and_then(serde_json::Value::as_str) {
        if validation::validate_name(name).is_err() {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS006",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.field_span("name"),
                    format!("workflow name '{name}' is not a safe identifier"),
                )
                .workflow(name)
                .field("name"),
            );
        }
    }

    let Some(nodes) = root.get("nodes").and_then(serde_json::Value::as_array) else {
        return diagnostics;
    };
    let mut names = BTreeSet::new();
    for (index, node) in nodes.iter().enumerate() {
        let node_path = format!("nodes[{index}]");
        let Some(node_obj) = node.as_object() else {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS002",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.nearest_span(&node_path),
                    "node must be a mapping",
                )
                .workflow(workflow_name)
                .field("nodes"),
            );
            continue;
        };
        let node_name = node_obj
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("<unknown>");
        check_allowed_fields(
            node_obj,
            &node_path,
            &[
                "name", "command", "session", "fanout", "artifact", "input", "inputs", "rules",
            ],
            &[
                "type",
                "mode",
                "facets",
                "prompt",
                "inline_prompt",
                "output_contract",
                "input_contracts",
                "pass_output_from",
                "pass_previous_response",
                "variables",
                "workflow_variables",
                "cycle_guard",
                "resets_cycle_for",
                "collect",
                "approval",
                "bash",
                "parallel",
                "tasks",
            ],
            span_map,
            workflow_name,
            Some(node_name),
            &mut diagnostics,
        );
        let kind_count = ["command", "session", "fanout"]
            .iter()
            .filter(|key| node_obj.contains_key(**key))
            .count();
        if kind_count != 1 {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS003",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.nearest_span(&node_path),
                    format!(
                        "node '{node_name}' must contain exactly one kind block: command, session, or fanout"
                    ),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("kind"),
            );
        }
        if node_name == "request" || node_name == "item" {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFR004",
                    Severity::Error,
                    DiagnosticStage::Resolve,
                    span_map.field_span(&format!("{node_path}.name")),
                    format!("node name '{node_name}' is reserved"),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("name"),
            );
        }
        if node_name != "<unknown>"
            && (validation::validate_name(node_name).is_err() || !names.insert(node_name))
        {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS006",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.field_span(&format!("{node_path}.name")),
                    format!("node name '{node_name}' is duplicated or invalid"),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("name"),
            );
        }
        if node_obj.contains_key("fanout") && node_obj.contains_key("inputs") {
            diagnostics.push(kind_disallowed_diagnostic(
                workflow_name,
                node_name,
                "fanout",
                "inputs",
                span_map.field_span(&format!("{node_path}.inputs")),
            ));
        }
        if let Some(session) = node_obj
            .get("session")
            .and_then(serde_json::Value::as_object)
        {
            check_allowed_fields(
                session,
                &format!("{node_path}.session"),
                &["model", "permission", "gate", "facets"],
                &["mode", "prompt", "inline_prompt"],
                span_map,
                workflow_name,
                Some(node_name),
                &mut diagnostics,
            );
            if !session.contains_key("gate") {
                diagnostics.push(
                    DiagnosticItem::new(
                        "WFS002",
                        Severity::Error,
                        DiagnosticStage::ParseShape,
                        span_map.nearest_span(&format!("{node_path}.session")),
                        format!("session node '{node_name}' requires gate: auto or gate: approval"),
                    )
                    .workflow(workflow_name)
                    .node(node_name)
                    .field("session.gate"),
                );
            }
            if let Some(facets) = session.get("facets").and_then(serde_json::Value::as_object) {
                check_allowed_fields(
                    facets,
                    &format!("{node_path}.session.facets"),
                    &["policy", "knowledge", "instruction"],
                    &[],
                    span_map,
                    workflow_name,
                    Some(node_name),
                    &mut diagnostics,
                );
            }
        }
        if let Some(fanout) = node_obj
            .get("fanout")
            .and_then(serde_json::Value::as_object)
        {
            check_allowed_fields(
                fanout,
                &format!("{node_path}.fanout"),
                &["child", "items"],
                &[
                    "parallel_children",
                    "aggregate",
                    "all_match",
                    "any_match",
                    "failure_policy",
                    "on_failure",
                    "fail_fast",
                ],
                span_map,
                workflow_name,
                Some(node_name),
                &mut diagnostics,
            );
            if let Some(aggregate) = fanout
                .get("aggregate")
                .and_then(serde_json::Value::as_object)
            {
                check_allowed_fields(
                    aggregate,
                    &format!("{node_path}.fanout.aggregate"),
                    &[],
                    &["all_match", "any_match", "then", "else"],
                    span_map,
                    workflow_name,
                    Some(node_name),
                    &mut diagnostics,
                );
            }
        }
        if let Some(rules) = node_obj.get("rules").and_then(serde_json::Value::as_array) {
            for (rule_index, rule) in rules.iter().enumerate() {
                let rule_path = format!("{node_path}.rules[{rule_index}]");
                let Some(rule_obj) = rule.as_object() else {
                    continue;
                };
                check_allowed_fields(
                    rule_obj,
                    &rule_path,
                    &["when", "switch", "loop_guard", "next"],
                    &[
                        "match",
                        "regex",
                        "expression",
                        "condition",
                        "cycle_guard",
                        "resets_cycle_for",
                        "reject",
                        "rerun",
                    ],
                    span_map,
                    workflow_name,
                    Some(node_name),
                    &mut diagnostics,
                );
                let discriminator_count = ["when", "switch", "loop_guard"]
                    .iter()
                    .filter(|key| rule_obj.contains_key(**key))
                    .count();
                if discriminator_count > 1 {
                    diagnostics.push(
                        DiagnosticItem::new(
                            "WFS003",
                            Severity::Error,
                            DiagnosticStage::ParseShape,
                            span_map.nearest_span(&rule_path),
                            "rule discriminator keys when, switch, and loop_guard are mutually exclusive",
                        )
                        .workflow(workflow_name)
                        .node(node_name)
                        .field("rules"),
                    );
                }
            }
        }
    }

    diagnostics
}

#[allow(clippy::too_many_arguments)]
fn check_allowed_fields(
    map: &serde_json::Map<String, serde_json::Value>,
    path: &str,
    allowed: &[&str],
    old_fields: &[&str],
    span_map: &YamlSpanMap,
    workflow_name: &str,
    node_name: Option<&str>,
    diagnostics: &mut Vec<DiagnosticItem>,
) {
    for key in map.keys() {
        if allowed.contains(&key.as_str()) {
            continue;
        }
        let field_path = if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        };
        let (code, message) = if old_fields.contains(&key.as_str()) {
            (
                "WFS005",
                format!(
                    "field '{key}' belongs to the old workflow syntax and is no longer accepted"
                ),
            )
        } else {
            (
                "WFS002",
                format!("unknown workflow field '{key}' is not allowed here"),
            )
        };
        let mut item = DiagnosticItem::new(
            code,
            Severity::Error,
            DiagnosticStage::ParseShape,
            span_map.field_span(&field_path),
            message,
        )
        .workflow(workflow_name)
        .field(key);
        if let Some(node_name) = node_name {
            item = item.node(node_name);
        }
        diagnostics.push(item);
    }
}

fn kind_disallowed_diagnostic(
    workflow_name: &str,
    node_name: &str,
    kind: &str,
    field: &str,
    span: Option<DiagnosticSpan>,
) -> DiagnosticItem {
    DiagnosticItem::new(
        "WFS004",
        Severity::Error,
        DiagnosticStage::ParseShape,
        span,
        format!("node '{node_name}' ({kind}) cannot declare '{field}'"),
    )
    .workflow(workflow_name)
    .node(node_name)
    .field(field)
}

fn deserialize_error_diagnostic(
    error: &serde_saphyr::Error,
    span_map: &YamlSpanMap,
    workflow_name_hint: Option<&str>,
) -> DiagnosticItem {
    let message = error.to_string();
    let code = if message.contains("old workflow syntax") {
        "WFS005"
    } else if message.contains("unknown field") || message.contains("unknown variant") {
        "WFS002"
    } else if message.contains("kind block")
        || message.contains("requires sibling next")
        || message.contains("rule discriminator")
        || message.contains("invalid rule shape")
    {
        "WFS003"
    } else if message.contains("YAML") || message.contains("syntax") {
        "WFS001"
    } else {
        "WFS002"
    };
    let span = error
        .location()
        .map(DiagnosticSpan::from_location)
        .or_else(|| span_map.nearest_span(""));
    DiagnosticItem::new(
        code,
        Severity::Error,
        DiagnosticStage::ParseShape,
        span,
        format!("workflow shape error: {message}"),
    )
    .workflow(workflow_name_hint.unwrap_or("<unknown>"))
}

fn validation_error_to_diagnostic(
    wf: &WorkflowDefinitionYaml,
    error: &validation::ValidationError,
    span_map: Option<&YamlSpanMap>,
) -> DiagnosticItem {
    let (code, stage) = validation_error_code_stage(error);
    let (node_name, field) = validation_error_context(error);
    let span = span_map.and_then(|map| span_for_validation_error(wf, error, map));
    let mut item = DiagnosticItem::new(code, Severity::Error, stage, span, error.to_string())
        .workflow(wf.name.clone());
    if let Some(node_name) = node_name {
        item = item.node(node_name);
    }
    if let Some(field) = field {
        item = item.field(field);
    }
    item
}

fn validation_error_code_stage(
    error: &validation::ValidationError,
) -> (&'static str, DiagnosticStage) {
    use validation::ValidationError;
    let code = match error {
        ValidationError::EmptyName
        | ValidationError::InvalidChars { .. }
        | ValidationError::EmptyNodes
        | ValidationError::DuplicateNode { .. }
        | ValidationError::EmptyFanoutChildren { .. }
        | ValidationError::EmptyCommand { .. }
        | ValidationError::TooManyNodes { .. }
        | ValidationError::TooManyFanoutChildren { .. } => "WFS006",
        ValidationError::UnknownRuleTarget { .. }
        | ValidationError::UnknownLoopGuardResetNode { .. }
        | ValidationError::UnknownFanoutChild { .. } => "WFR001",
        ValidationError::InvalidFanoutItemsReference { .. } => "WFR003",
        ValidationError::FanoutInputMismatch { .. } => "WFT003",
        ValidationError::FanoutChildLeafViolation { .. } => "WFC006",
        ValidationError::UnknownSchemaRef { .. } => "WFR002",
        ValidationError::InvalidSchemaRef { .. } => "WFR002",
        ValidationError::InvalidSchema { kind, .. } => match kind {
            InvalidSchemaKind::UnknownSchemaReference => "WFR002",
            InvalidSchemaKind::InvalidDeclaration => "WFS002",
        },
        ValidationError::InvalidArtifactReference { kind, .. } => match kind {
            InvalidArtifactReferenceKind::ReservedArtifactName => "WFR004",
            InvalidArtifactReferenceKind::ItemOutOfScope => "WFR005",
            InvalidArtifactReferenceKind::InputsNotAllowedOnFanout => "WFS004",
            InvalidArtifactReferenceKind::UnknownNode
            | InvalidArtifactReferenceKind::UnavailableArtifact
            | InvalidArtifactReferenceKind::UnknownField
            | InvalidArtifactReferenceKind::InvalidInputRef => "WFR003",
        },
        ValidationError::InvalidArtifactSchema { .. } => "WFT004",
        ValidationError::ReservedArtifactField { .. } => "WFT005",
        ValidationError::InvalidRules { kind, .. } => match kind {
            InvalidRuleKind::WhenFieldNotBoolean => "WFT001",
            InvalidRuleKind::SwitchFieldNotEnum | InvalidRuleKind::SwitchUnknownCase => "WFT002",
            InvalidRuleKind::DiscriminatorOnFanout
            | InvalidRuleKind::DiscriminatorWithoutArtifact => "WFT006",
            InvalidRuleKind::SwitchMissingCases => "WFC004",
            InvalidRuleKind::LoopGuardMaxIterations | InvalidRuleKind::CycleWithoutLoopGuard => {
                "WFC005"
            }
            InvalidRuleKind::MultipleNextCatchAll
            | InvalidRuleKind::SwitchExhaustiveHasNext
            | InvalidRuleKind::SwitchRequiresNext => "WFC003",
            InvalidRuleKind::MultipleDiscriminators
            | InvalidRuleKind::MultipleLoopGuards
            | InvalidRuleKind::StandaloneNextWithDiscriminator => "WFC002",
        },
        ValidationError::UnreachableNode { .. } => "WFC001",
        ValidationError::MissingFacet { .. } => "WFR900",
        ValidationError::InvalidPermissionMode { .. }
        | ValidationError::MissingPermissionMode { .. }
        | ValidationError::UnknownModel { .. }
        | ValidationError::InvalidModelFormat { .. }
        | ValidationError::ModelResolutionFailed { .. } => "WFT900",
    };
    (code, stage_for_code(code))
}

fn stage_for_code(code: &str) -> DiagnosticStage {
    if code.starts_with("WFR") {
        DiagnosticStage::Resolve
    } else if code.starts_with("WFT") {
        DiagnosticStage::Typecheck
    } else if code.starts_with("WFC") {
        DiagnosticStage::ControlFlow
    } else {
        DiagnosticStage::ParseShape
    }
}

fn span_for_validation_error(
    wf: &WorkflowDefinitionYaml,
    error: &validation::ValidationError,
    span_map: &YamlSpanMap,
) -> Option<DiagnosticSpan> {
    use validation::ValidationError;
    match error {
        ValidationError::InvalidSchema { schema, .. } => span_map
            .field_span(&format!("schemas.{schema}"))
            .or_else(|| span_map.field_span("schemas")),
        ValidationError::InvalidArtifactReference { reference, .. } => {
            input_reference_path(wf, reference)
                .and_then(|path| span_map.field_span(&path))
                .or_else(|| span_map.field_span("nodes"))
        }
        ValidationError::InvalidRules { node, kind, .. } => invalid_rule_path(wf, node, *kind)
            .and_then(|path| span_map.field_span(&path))
            .or_else(|| {
                node_base_path(wf, node)
                    .and_then(|path| span_map.field_span(&format!("{path}.rules")))
            }),
        ValidationError::UnknownLoopGuardResetNode { node, .. } => {
            loop_guard_reset_on_path(wf, node)
                .and_then(|path| span_map.field_span(&path))
                .or_else(|| {
                    node_base_path(wf, node)
                        .and_then(|path| span_map.field_span(&format!("{path}.rules")))
                })
        }
        ValidationError::UnreachableNode { node } => {
            node_base_path(wf, node).and_then(|path| span_map.nearest_span(&path))
        }
        _ => {
            let (node_name, field) = validation_error_context(error);
            match (node_name.as_deref(), field.as_deref()) {
                (Some(node_name), Some(field)) => node_field_path(wf, node_name, field)
                    .and_then(|path| span_map.field_span(&path))
                    .or_else(|| {
                        node_base_path(wf, node_name).and_then(|path| span_map.nearest_span(&path))
                    }),
                (Some(node_name), None) => {
                    node_base_path(wf, node_name).and_then(|path| span_map.nearest_span(&path))
                }
                (None, Some(field)) => span_map.field_span(field),
                (None, None) => span_map.nearest_span(""),
            }
        }
    }
}

fn input_reference_path(wf: &WorkflowDefinitionYaml, reference: &str) -> Option<String> {
    for (index, node) in wf.nodes.iter().enumerate() {
        for (input_index, input) in node.inputs.iter().enumerate() {
            if input == reference {
                return Some(format!("nodes[{index}].inputs[{input_index}]"));
            }
        }
    }
    None
}

fn invalid_rule_path(
    wf: &WorkflowDefinitionYaml,
    node_name: &str,
    kind: InvalidRuleKind,
) -> Option<String> {
    let (base, node) = node_base_path_with_node(wf, node_name)?;
    let suffix = match kind {
        InvalidRuleKind::WhenFieldNotBoolean => {
            rule_index(node, |rule| matches!(rule, Rule::When { .. }))
                .map(|index| format!("rules[{index}].when.on"))
        }
        InvalidRuleKind::SwitchFieldNotEnum
        | InvalidRuleKind::SwitchUnknownCase
        | InvalidRuleKind::SwitchMissingCases => {
            rule_index(node, |rule| matches!(rule, Rule::Switch { .. }))
                .map(|index| format!("rules[{index}].switch.on"))
        }
        InvalidRuleKind::SwitchExhaustiveHasNext | InvalidRuleKind::SwitchRequiresNext => {
            rule_index(node, |rule| matches!(rule, Rule::Switch { .. }))
                .map(|index| format!("rules[{index}].next"))
        }
        InvalidRuleKind::LoopGuardMaxIterations => {
            rule_index(node, |rule| matches!(rule, Rule::LoopGuard { .. }))
                .map(|index| format!("rules[{index}].loop_guard.max_iterations"))
        }
        InvalidRuleKind::DiscriminatorOnFanout | InvalidRuleKind::DiscriminatorWithoutArtifact => {
            rule_index(node, |rule| {
                matches!(rule, Rule::When { .. } | Rule::Switch { .. })
            })
            .map(|index| format!("rules[{index}]"))
        }
        InvalidRuleKind::MultipleDiscriminators
        | InvalidRuleKind::MultipleLoopGuards
        | InvalidRuleKind::MultipleNextCatchAll
        | InvalidRuleKind::StandaloneNextWithDiscriminator
        | InvalidRuleKind::CycleWithoutLoopGuard => Some("rules".to_string()),
    }?;
    Some(format!("{base}.{suffix}"))
}

fn loop_guard_reset_on_path(wf: &WorkflowDefinitionYaml, node_name: &str) -> Option<String> {
    let (base, node) = node_base_path_with_node(wf, node_name)?;
    rule_index(node, |rule| matches!(rule, Rule::LoopGuard { .. }))
        .map(|index| format!("{base}.rules[{index}].loop_guard.reset_on"))
}

fn node_base_path_with_node<'a>(
    wf: &'a WorkflowDefinitionYaml,
    node_name: &str,
) -> Option<(String, &'a NodeDefinition)> {
    for (index, node) in wf.nodes.iter().enumerate() {
        if node.name == node_name {
            return Some((format!("nodes[{index}]"), node));
        }
    }
    None
}

fn rule_index(node: &NodeDefinition, matches_rule: impl Fn(&Rule) -> bool) -> Option<usize> {
    node.rules.iter().position(matches_rule)
}

fn node_base_path(wf: &WorkflowDefinitionYaml, node_name: &str) -> Option<String> {
    for (index, node) in wf.nodes.iter().enumerate() {
        if node.name == node_name {
            return Some(format!("nodes[{index}]"));
        }
    }
    None
}

fn node_field_path(wf: &WorkflowDefinitionYaml, node_name: &str, field: &str) -> Option<String> {
    let base = node_base_path(wf, node_name)?;
    let suffix = match field {
        "permission" | "model" => format!("session.{field}"),
        "facets" => "session.facets".to_string(),
        "rules.next" => "rules".to_string(),
        field => field.to_string(),
    };
    Some(format!("{base}.{suffix}"))
}

/// 全ワークフロー・全ファセットを走査し診断結果を返す
pub fn diagnose_all(workflows_dir: &Path, facets_base_dir: &Path) -> DiagnosticReport {
    let mut items = Vec::new();
    let mut workflow_summaries: HashMap<String, DiagnosticSummary> = HashMap::new();
    let mut facet_summaries: HashMap<String, DiagnosticSummary> = HashMap::new();
    let mut facet_usage: HashMap<String, Vec<FacetUsageEntry>> = HashMap::new();

    // --- 全ファセットキーのセットを構築（参照存在チェック用） ---
    let all_facet_keys = collect_all_facet_keys(facets_base_dir);

    // --- ワークフロー診断 ---
    let workflows = load_all_workflows(workflows_dir, facets_base_dir);
    for (name, wf_result) in &workflows {
        match wf_result {
            Err(diagnostics) => {
                for item in diagnostics {
                    add_diagnostic(&mut items, &mut workflow_summaries, name, item.clone());
                }
            }
            Ok((wf, source_diagnostics)) => {
                for item in source_diagnostics {
                    add_diagnostic(&mut items, &mut workflow_summaries, name, item.clone());
                }
                diagnose_workflow(
                    wf,
                    &all_facet_keys,
                    &mut items,
                    &mut workflow_summaries,
                    &mut facet_usage,
                );
            }
        }
    }
    let workflow_lookup: HashMap<&str, &WorkflowDefinitionYaml> = workflows
        .iter()
        .filter_map(|(_, result)| {
            result
                .as_ref()
                .ok()
                .map(|(workflow, _)| (workflow.name.as_str(), workflow))
        })
        .collect();

    // --- ファセット診断 ---
    for kind in &ALL_FACET_KINDS {
        let summaries = facet::list_facet_summaries(*kind, facets_base_dir).unwrap_or_default();
        for summary in &summaries {
            let facet_id = format!("{}/{}", kind.canonical_name(), summary.key);

            // ファセットキー命名規則チェック
            if facet::validate_facet_key(&summary.key).is_err() {
                let item = DiagnosticItem::new(
                    "FAC001",
                    Severity::Error,
                    DiagnosticStage::Resolve,
                    None,
                    format!(
                        "ファセットキー '{}' が命名規則に違反しています",
                        summary.key
                    ),
                )
                .facet(summary.key.clone(), kind.canonical_name().to_string())
                .field("key");
                add_diagnostic(&mut items, &mut facet_summaries, &facet_id, item);
            }

            // ビルトイン info
            if summary.builtin {
                let item = DiagnosticItem::new(
                    "FAC000",
                    Severity::Info,
                    DiagnosticStage::Resolve,
                    None,
                    format!(
                        "ビルトインファセット '{}' ({})",
                        summary.key,
                        kind.canonical_name()
                    ),
                )
                .facet(summary.key.clone(), kind.canonical_name().to_string());
                add_diagnostic(&mut items, &mut facet_summaries, &facet_id, item);
            }

            // テンプレート変数チェック
            if let Ok(content) = facet::load_facet(*kind, &summary.key, facets_base_dir) {
                check_template_variables(
                    &content,
                    &summary.key,
                    kind.canonical_name(),
                    &facet_id,
                    &mut items,
                    &mut facet_summaries,
                );
                check_facet_template_references(
                    &content,
                    &summary.key,
                    kind.canonical_name(),
                    &facet_id,
                    &workflow_lookup,
                    &facet_usage,
                    &mut items,
                    &mut workflow_summaries,
                    &mut facet_summaries,
                );
            }
        }
    }

    DiagnosticReport {
        items,
        workflow_summaries,
        facet_summaries,
        facet_usage,
    }
}

/// 全ファセットキーを収集（"kind/key" 形式）
fn collect_all_facet_keys(base_dir: &Path) -> HashSet<String> {
    let mut keys = HashSet::new();
    for kind in &ALL_FACET_KINDS {
        if let Ok(list) = facet::list_facets(*kind, base_dir) {
            for key in list {
                keys.insert(format!("{}/{}", kind.canonical_name(), key));
            }
        }
    }
    keys
}

fn collect_referenced_facet_keys(
    workflow: &WorkflowDefinitionYaml,
    base_dir: &Path,
) -> Result<HashSet<String>, facet::FacetError> {
    let mut checked = HashSet::new();
    let mut existing = HashSet::new();
    for node in &workflow.nodes {
        let Some(session) = node.session() else {
            continue;
        };
        if let Some(key) = session.facets.policy.as_deref() {
            collect_existing_facet_key(
                &mut checked,
                &mut existing,
                FacetKind::Policy,
                key,
                base_dir,
            )?;
        }
        for key in &session.facets.knowledge {
            collect_existing_facet_key(
                &mut checked,
                &mut existing,
                FacetKind::Knowledge,
                key,
                base_dir,
            )?;
        }
        if let Some(key) = session.facets.instruction.as_deref() {
            collect_existing_facet_key(
                &mut checked,
                &mut existing,
                FacetKind::Instruction,
                key,
                base_dir,
            )?;
        }
    }
    Ok(existing)
}

fn collect_existing_facet_key(
    checked: &mut HashSet<String>,
    existing: &mut HashSet<String>,
    kind: FacetKind,
    key: &str,
    base_dir: &Path,
) -> Result<(), facet::FacetError> {
    let facet_id = format!("{}/{}", kind.canonical_name(), key);
    if checked.insert(facet_id.clone()) && facet::facet_exists(kind, key, base_dir)? {
        existing.insert(facet_id);
    }
    Ok(())
}

/// Load/save の解決前に、workflow が参照する facet の存在を構造化 Diagnostic として検査する。
///
/// `diagnose_all` と同じ FAC002 shape を返すことで、source editor と runtime loader の
/// どちらでも欠損した参照名・node・slot を失わない。facet inventory 自体を読めない場合は
/// I/O error を欠損参照へ誤分類せず、そのまま caller へ伝搬する。
pub(crate) fn diagnose_workflow_facet_references(
    workflow: &WorkflowDefinitionYaml,
    facets_base_dir: &Path,
) -> Result<Vec<DiagnosticItem>, facet::FacetError> {
    let all_facet_keys = collect_referenced_facet_keys(workflow, facets_base_dir)?;
    let mut items = Vec::new();
    let mut workflow_summaries = HashMap::new();
    let mut facet_usage = HashMap::new();
    check_workflow_facet_references(
        workflow,
        &all_facet_keys,
        &mut items,
        &mut workflow_summaries,
        &mut facet_usage,
    );
    Ok(items)
}

/// ディスク + builtin のワークフロー一覧を読み込み
fn load_all_workflows(dir: &Path, facets_base_dir: &Path) -> Vec<NamedWorkflowDiagnostics> {
    let mut results = Vec::new();

    // ディスク上のカスタムワークフロー（validate() をスキップし全件走査）
    let mut seen = HashSet::new();
    if dir.exists() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("yml") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let name = stem.to_string();
                        let result = match std::fs::read_to_string(&path) {
                            Ok(content) => {
                                let diagnosis = diagnose_workflow_source(&content, Some(&name));
                                if let Some(workflow) = diagnosis.workflow {
                                    let _ =
                                        facet::resolve_workflow_facets(&workflow, facets_base_dir);
                                    Ok((workflow, diagnosis.diagnostics))
                                } else {
                                    Err(diagnosis.diagnostics)
                                }
                            }
                            Err(error) => Err(vec![DiagnosticItem::new(
                                "WFS001",
                                Severity::Error,
                                DiagnosticStage::ParseShape,
                                None,
                                format!("ワークフロー '{name}' の読み込みに失敗: {error}"),
                            )
                            .workflow(name.clone())]),
                        };
                        seen.insert(name.clone());
                        results.push((name, result));
                    }
                }
            }
        }
    }

    // ビルトインワークフロー
    for summary in builtin::list_builtin_workflows() {
        if !seen.contains(&summary.name) {
            match builtin::load_builtin_workflow_resolved(&summary.name) {
                Ok(Some(wf)) => results.push((summary.name, Ok((wf, Vec::new())))),
                Ok(None) => results.push((
                    summary.name.clone(),
                    Err(vec![DiagnosticItem::new(
                        "WFS001",
                        Severity::Error,
                        DiagnosticStage::ParseShape,
                        None,
                        format!("ビルトインワークフロー '{}' の読み込みに失敗", summary.name),
                    )
                    .workflow(summary.name.clone())]),
                )),
                Err(err) => results.push((
                    summary.name.clone(),
                    Err(vec![DiagnosticItem::new(
                        "WFS001",
                        Severity::Error,
                        DiagnosticStage::ParseShape,
                        None,
                        format!(
                            "ビルトインワークフロー '{}' の読み込みに失敗: {err}",
                            summary.name
                        ),
                    )
                    .workflow(summary.name.clone())]),
                )),
            }
        }
    }

    results
}

/// ValidationError から node 名とフィールド名を抽出
fn validation_error_context(e: &validation::ValidationError) -> (Option<String>, Option<String>) {
    use validation::ValidationError;
    match e {
        ValidationError::EmptyName | ValidationError::InvalidChars { .. } => {
            (None, Some("name".to_string()))
        }
        ValidationError::EmptyNodes => (None, Some("nodes".to_string())),
        ValidationError::DuplicateNode { name } => (Some(name.clone()), Some("name".to_string())),
        ValidationError::EmptyFanoutChildren { node } => {
            (Some(node.clone()), Some("fanout.child".to_string()))
        }
        ValidationError::UnknownFanoutChild { node, .. } => {
            (Some(node.clone()), Some("fanout.child".to_string()))
        }
        ValidationError::InvalidFanoutItemsReference { node, .. }
        | ValidationError::FanoutInputMismatch { node, .. } => {
            (Some(node.clone()), Some("fanout.items".to_string()))
        }
        ValidationError::FanoutChildLeafViolation { node, .. } => {
            (Some(node.clone()), Some("fanout.child".to_string()))
        }
        ValidationError::UnknownRuleTarget { node, .. } => {
            (Some(node.clone()), Some("rules.next".to_string()))
        }
        ValidationError::UnknownLoopGuardResetNode { node, .. } => (
            Some(node.clone()),
            Some("rules.loop_guard.reset_on".to_string()),
        ),
        ValidationError::InvalidRules { node, kind, .. } => (
            Some(node.clone()),
            Some(invalid_rule_field_name(*kind).to_string()),
        ),
        ValidationError::UnreachableNode { node } => {
            (Some(node.clone()), Some("nodes".to_string()))
        }
        ValidationError::MissingFacet { node } => (Some(node.clone()), Some("facets".to_string())),
        ValidationError::InvalidArtifactReference { .. } => (None, Some("inputs".to_string())),
        ValidationError::InvalidPermissionMode { node, .. } => {
            (Some(node.clone()), Some("permission".to_string()))
        }
        ValidationError::MissingPermissionMode { node } => {
            (Some(node.clone()), Some("permission".to_string()))
        }
        ValidationError::UnknownModel { node, .. } => {
            (Some(node.clone()), Some("model".to_string()))
        }
        ValidationError::InvalidModelFormat { node, .. } => {
            (Some(node.clone()), Some("model".to_string()))
        }
        ValidationError::ModelResolutionFailed { node, .. } => {
            (Some(node.clone()), Some("model".to_string()))
        }
        ValidationError::EmptyCommand { node } => (Some(node.clone()), Some("command".to_string())),
        ValidationError::TooManyNodes { .. } => (None, Some("nodes".to_string())),
        ValidationError::TooManyFanoutChildren { node, .. } => {
            (Some(node.clone()), Some("fanout.child".to_string()))
        }
        ValidationError::UnknownSchemaRef { node, slot, .. }
        | ValidationError::InvalidSchemaRef { node, slot, .. } => {
            (Some(node.clone()), Some((*slot).to_string()))
        }
        ValidationError::InvalidSchema { .. } => (None, Some("schemas".to_string())),
        ValidationError::InvalidArtifactSchema { node, .. }
        | ValidationError::ReservedArtifactField { node, .. } => {
            (Some(node.clone()), Some("artifact".to_string()))
        }
    }
}

fn invalid_rule_field_name(kind: InvalidRuleKind) -> &'static str {
    match kind {
        InvalidRuleKind::WhenFieldNotBoolean => "rules.when.on",
        InvalidRuleKind::SwitchFieldNotEnum
        | InvalidRuleKind::SwitchUnknownCase
        | InvalidRuleKind::SwitchMissingCases => "rules.switch.on",
        InvalidRuleKind::SwitchExhaustiveHasNext
        | InvalidRuleKind::SwitchRequiresNext
        | InvalidRuleKind::MultipleNextCatchAll => "rules.next",
        InvalidRuleKind::LoopGuardMaxIterations => "rules.loop_guard.max_iterations",
        InvalidRuleKind::DiscriminatorOnFanout
        | InvalidRuleKind::DiscriminatorWithoutArtifact
        | InvalidRuleKind::MultipleDiscriminators
        | InvalidRuleKind::MultipleLoopGuards
        | InvalidRuleKind::StandaloneNextWithDiscriminator
        | InvalidRuleKind::CycleWithoutLoopGuard => "rules",
    }
}

fn diagnose_workflow(
    wf: &WorkflowDefinitionYaml,
    all_facet_keys: &HashSet<String>,
    items: &mut Vec<DiagnosticItem>,
    workflow_summaries: &mut HashMap<String, DiagnosticSummary>,
    facet_usage: &mut HashMap<String, Vec<FacetUsageEntry>>,
) {
    let name = &wf.name;

    // ビルトイン info
    if wf.builtin {
        let item = DiagnosticItem::new(
            "WFI000",
            Severity::Info,
            DiagnosticStage::Resolve,
            None,
            format!("ビルトインワークフロー '{name}'"),
        )
        .workflow(name.clone());
        add_diagnostic(items, workflow_summaries, name, item);
    }

    check_workflow_facet_references(wf, all_facet_keys, items, workflow_summaries, facet_usage);
}

fn check_workflow_facet_references(
    wf: &WorkflowDefinitionYaml,
    all_facet_keys: &HashSet<String>,
    items: &mut Vec<DiagnosticItem>,
    workflow_summaries: &mut HashMap<String, DiagnosticSummary>,
    facet_usage: &mut HashMap<String, Vec<FacetUsageEntry>>,
) {
    let name = &wf.name;
    for node in &wf.nodes {
        // ファセット参照の存在チェック + usage 記録
        FacetRefCheckContext::new(name, all_facet_keys, items, workflow_summaries, facet_usage)
            .check_node(
                &node.name,
                &FacetRefs {
                    policy: node
                        .session()
                        .and_then(|session| session.facets.policy.as_deref()),
                    knowledge: node
                        .session()
                        .map(|session| session.facets.knowledge.as_slice())
                        .unwrap_or_default(),
                    instruction: node
                        .session()
                        .and_then(|session| session.facets.instruction.as_deref()),
                },
            );
    }
}

struct FacetRefs<'a> {
    policy: Option<&'a str>,
    knowledge: &'a [String],
    instruction: Option<&'a str>,
}

/// 複数の facet 参照を 1 つの node スコープで一括検査するためのコンテキスト。
///
/// 旧 `check_single_facet_ref` は 9 引数で都度 sink / 一覧 / workflow 名を渡していたが、
/// それらは「`diagnose_workflow` の 1 走行を通じて共有される」性質のもの。
/// このコンテキストにまとめて `ctx.check(node, slot, kind, key)` の形で呼び出すことで、
/// 凝集度を上げ `#[allow(clippy::too_many_arguments)]` を不要にする。
struct FacetRefCheckContext<'a> {
    workflow_name: &'a str,
    all_facet_keys: &'a HashSet<String>,
    items: &'a mut Vec<DiagnosticItem>,
    workflow_summaries: &'a mut HashMap<String, DiagnosticSummary>,
    facet_usage: &'a mut HashMap<String, Vec<FacetUsageEntry>>,
}

impl<'a> FacetRefCheckContext<'a> {
    fn new(
        workflow_name: &'a str,
        all_facet_keys: &'a HashSet<String>,
        items: &'a mut Vec<DiagnosticItem>,
        workflow_summaries: &'a mut HashMap<String, DiagnosticSummary>,
        facet_usage: &'a mut HashMap<String, Vec<FacetUsageEntry>>,
    ) -> Self {
        Self {
            workflow_name,
            all_facet_keys,
            items,
            workflow_summaries,
            facet_usage,
        }
    }

    /// 単一の facet 参照について usage 記録と存在チェックを行う。
    fn check(&mut self, node_name: &str, slot: &str, kind: FacetKind, key: &str) {
        let facet_id = format!("{}/{}", kind.canonical_name(), key);

        self.facet_usage
            .entry(facet_id.clone())
            .or_default()
            .push(FacetUsageEntry {
                workflow_name: self.workflow_name.to_string(),
                node_name: node_name.to_string(),
                slot: slot.to_string(),
            });

        if !self.all_facet_keys.contains(&facet_id) {
            let item = DiagnosticItem::new(
                "FAC002",
                Severity::Error,
                DiagnosticStage::Resolve,
                None,
                format!(
                    "node '{}' が存在しないファセット '{}' ({}) を参照しています",
                    node_name,
                    key,
                    kind.canonical_name()
                ),
            )
            .workflow(self.workflow_name.to_string())
            .node(node_name.to_string())
            .facet(key.to_string(), kind.canonical_name().to_string())
            .field(slot.to_string());
            add_diagnostic(
                self.items,
                self.workflow_summaries,
                self.workflow_name,
                item,
            );
        }
    }

    /// 1 つの node が持つ全 facet ref を一括検査する。
    fn check_node(&mut self, node_name: &str, facet_refs: &FacetRefs<'_>) {
        if let Some(key) = facet_refs.policy {
            self.check(node_name, "policy", FacetKind::Policy, key);
        }
        for key in facet_refs.knowledge {
            self.check(node_name, "knowledge", FacetKind::Knowledge, key);
        }
        if let Some(key) = facet_refs.instruction {
            self.check(node_name, "instruction", FacetKind::Instruction, key);
        }
    }
}

fn check_template_variables(
    content: &str,
    facet_key: &str,
    facet_kind_name: &str,
    facet_id: &str,
    items: &mut Vec<DiagnosticItem>,
    facet_summaries: &mut HashMap<String, DiagnosticSummary>,
) {
    for var_name in prompt_rendering::find_undefined_template_variables(content) {
        let item = DiagnosticItem::new(
            "FAC003",
            Severity::Error,
            DiagnosticStage::Resolve,
            None,
            format!(
                "ファセット '{}' に未定義のテンプレート変数 '{{{{{}}}}}' が含まれています",
                facet_key, var_name
            ),
        )
        .facet(facet_key.to_string(), facet_kind_name.to_string())
        .field("content");
        add_diagnostic(items, facet_summaries, facet_id, item);
    }
}

#[allow(clippy::too_many_arguments)]
fn check_facet_template_references(
    content: &str,
    facet_key: &str,
    facet_kind_name: &str,
    facet_id: &str,
    workflow_lookup: &HashMap<&str, &WorkflowDefinitionYaml>,
    facet_usage: &HashMap<String, Vec<FacetUsageEntry>>,
    items: &mut Vec<DiagnosticItem>,
    workflow_summaries: &mut HashMap<String, DiagnosticSummary>,
    facet_summaries: &mut HashMap<String, DiagnosticSummary>,
) {
    let Some(usages) = facet_usage.get(facet_id) else {
        return;
    };
    for usage in usages {
        let Some(workflow) = workflow_lookup.get(usage.workflow_name.as_str()).copied() else {
            continue;
        };
        let domain_workflow = workflow_definition_to_domain(workflow);
        let allow_item = facet_usage_allows_item(workflow, &usage.node_name);
        for error in validation::validate_template_references(&domain_workflow, content, allow_item)
        {
            let span = facet_template_error_span(content, &error);
            let mut item = validation_error_to_diagnostic(workflow, &error, None)
                .facet(facet_key.to_string(), facet_kind_name.to_string())
                .node(usage.node_name.clone())
                .field("content");
            item.span = span;
            add_diagnostic_to_workflow_and_facet(
                items,
                workflow_summaries,
                &usage.workflow_name,
                facet_summaries,
                facet_id,
                item,
            );
        }
    }
}

fn facet_usage_allows_item(workflow: &WorkflowDefinitionYaml, node_name: &str) -> bool {
    workflow.nodes.iter().any(|node| {
        node.fanout()
            .is_some_and(|fanout| fanout.child.iter().any(|child| child == node_name))
    })
}

fn facet_template_error_span(
    content: &str,
    error: &validation::ValidationError,
) -> Option<DiagnosticSpan> {
    let validation::ValidationError::InvalidArtifactReference { reference, .. } = error else {
        return None;
    };
    template_reference_span(content, reference)
}

fn template_reference_span(content: &str, reference: &str) -> Option<DiagnosticSpan> {
    for (line_index, line) in content.lines().enumerate() {
        let mut search_start = 0usize;
        while let Some(open_rel) = line[search_start..].find("{{") {
            let open = search_start + open_rel;
            let inner_start = open + 2;
            let Some(close_rel) = line[inner_start..].find("}}") else {
                break;
            };
            let close = inner_start + close_rel;
            let template_reference = line[inner_start..close].trim();
            if template_reference == reference
                || template_reference
                    .strip_prefix(reference)
                    .is_some_and(|rest| rest.starts_with('.'))
            {
                let end = close + 2;
                return Some(DiagnosticSpan {
                    start_line: line_index + 1,
                    start_col: line[..open].chars().count() + 1,
                    end_line: line_index + 1,
                    end_col: line[..end].chars().count() + 1,
                });
            }
            search_start = close + 2;
        }
    }
    None
}

fn add_diagnostic(
    items: &mut Vec<DiagnosticItem>,
    summaries: &mut HashMap<String, DiagnosticSummary>,
    key: &str,
    item: DiagnosticItem,
) {
    increment_summary(summaries, key, item.severity);
    items.push(item);
}

fn add_diagnostic_to_workflow_and_facet(
    items: &mut Vec<DiagnosticItem>,
    workflow_summaries: &mut HashMap<String, DiagnosticSummary>,
    workflow_key: &str,
    facet_summaries: &mut HashMap<String, DiagnosticSummary>,
    facet_key: &str,
    item: DiagnosticItem,
) {
    increment_summary(workflow_summaries, workflow_key, item.severity);
    increment_summary(facet_summaries, facet_key, item.severity);
    items.push(item);
}

fn increment_summary(
    summaries: &mut HashMap<String, DiagnosticSummary>,
    key: &str,
    severity: Severity,
) {
    let summary = summaries.entry(key.to_string()).or_default();
    match severity {
        Severity::Error => summary.error_count += 1,
        Severity::Info => summary.info_count += 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, FacetRefs, FanoutSpec, ItemsSource, NodeKind, Rule, SchemaDef, SessionSpec,
        WorkflowDefinitionYaml,
    };
    use std::fs;
    use tempfile::TempDir;

    fn make_node(name: &str, instruction: Option<&str>) -> NodeDefinition {
        let facets = FacetRefs {
            instruction: instruction.map(str::to_string),
            ..Default::default()
        };
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets,
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_child(name: &str, instruction: Option<&str>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                permission: Some("edit".to_string()),
                facets: FacetRefs {
                    instruction: instruction.map(str::to_string),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_fanout(name: &str, children: Vec<&str>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: children.into_iter().map(str::to_string).collect(),
                items: None,
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_command(name: &str, command: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: command.to_string(),
            }),
            ..NodeDefinition::default()
        }
    }

    fn setup_facet(dir: &Path, kind: &str, key: &str, content: &str) {
        let facet_dir = dir.join(kind);
        fs::create_dir_all(&facet_dir).unwrap();
        fs::write(facet_dir.join(format!("{key}.md")), content).unwrap();
    }

    fn save_workflow_yaml(dir: &Path, wf: &WorkflowDefinitionYaml) {
        fs::create_dir_all(dir).unwrap();
        let content = serde_saphyr::to_string(wf).unwrap();
        fs::write(dir.join(format!("{}.yml", wf.name)), content).unwrap();
    }

    #[test]
    fn collect_all_facet_keys_preserves_healthy_kinds_when_one_inventory_fails() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("policies"), "not a directory").unwrap();
        setup_facet(tmp.path(), "knowledge", "known", "known content");

        let keys = collect_all_facet_keys(tmp.path());

        assert!(keys.contains("knowledge/known"));
    }

    #[test]
    fn reference_diagnostics_ignore_unreferenced_broken_facet_inventory() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("policies"), "not a directory").unwrap();
        setup_facet(tmp.path(), "knowledge", "known", "known content");
        let workflow = WorkflowDefinitionYaml {
            name: "knowledge-only".to_string(),
            description: "knowledge-only diagnostic".to_string(),
            nodes: vec![NodeDefinition {
                name: "node".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        knowledge: vec!["known".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let diagnostics = diagnose_workflow_facet_references(&workflow, tmp.path()).unwrap();

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reference_diagnostics_propagate_referenced_broken_facet_inventory_error() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("policies"), "not a directory").unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "custom-policy".to_string(),
            description: "custom policy diagnostic".to_string(),
            nodes: vec![NodeDefinition {
                name: "node".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        policy: Some("custom-policy".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let error = diagnose_workflow_facet_references(&workflow, tmp.path()).unwrap_err();

        assert!(matches!(error, facet::FacetError::Io(_)));
    }

    #[test]
    fn reference_diagnostics_short_circuit_broken_inventory_for_builtin_facet() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("knowledge"), "not a directory").unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "builtin-knowledge".to_string(),
            description: "builtin knowledge diagnostic".to_string(),
            nodes: vec![NodeDefinition {
                name: "node".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        knowledge: vec!["releash-thread-cli".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let diagnostics = diagnose_workflow_facet_references(&workflow, tmp.path()).unwrap();

        assert!(diagnostics.is_empty());
    }

    fn fixture_dir(kind: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/adaptor/gateway/workflow/fixtures")
            .join(kind)
    }

    fn full_pipeline_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/examples/full-pipeline.yml")
    }

    fn expected_stage_for_code(code: &str) -> DiagnosticStage {
        match &code[..3] {
            "WFR" => DiagnosticStage::Resolve,
            "WFT" => DiagnosticStage::Typecheck,
            "WFC" => DiagnosticStage::ControlFlow,
            _ => DiagnosticStage::ParseShape,
        }
    }

    #[test]
    fn workflow_fixture_suite_valid_has_zero_diagnostics() {
        for entry in fs::read_dir(fixture_dir("valid")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yml") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap();
            let diagnosis =
                diagnose_workflow_source(&source, path.file_stem().and_then(|stem| stem.to_str()));
            assert!(
                diagnosis.diagnostics.is_empty(),
                "valid fixture {} produced diagnostics: {:?}",
                path.display(),
                diagnosis.diagnostics,
            );
        }
    }

    #[test]
    fn full_pipeline_canonical_example_loads_with_zero_diagnostics() {
        let source_path = full_pipeline_path();
        let source = fs::read_to_string(&source_path).unwrap();
        let diagnosis = diagnose_workflow_source(&source, Some("full-pipeline"));
        assert!(
            diagnosis.diagnostics.is_empty(),
            "canonical full-pipeline example produced diagnostics: {:?}",
            diagnosis.diagnostics
        );
        assert_eq!(
            diagnosis
                .workflow
                .as_ref()
                .expect("zero diagnostics must yield a workflow")
                .name,
            "full-pipeline"
        );

        let tmp = TempDir::new().unwrap();
        let workflow_path = tmp.path().join("full-pipeline.yml");
        fs::write(&workflow_path, source).unwrap();
        for policy in ["implementing", "reviewing", "triage"] {
            setup_facet(tmp.path(), "policies", policy, "test policy");
        }
        for knowledge in ["releash-review", "releash-thread"] {
            setup_facet(tmp.path(), "knowledge", knowledge, "test knowledge");
        }
        for instruction in [
            "fix-failing-tests",
            "review-diff",
            "apply-review-fixes",
            "decide-thread-fix",
            "triage-ship-decision",
            "summarize-ship",
            "summarize-failure",
            "summarize-escalation",
        ] {
            setup_facet(tmp.path(), "instructions", instruction, "test instruction");
        }

        let workflow =
            crate::adaptor::gateway::workflow::storage::load_workflow(&workflow_path, tmp.path())
                .expect("canonical full-pipeline example must pass the real loader");
        assert_eq!(workflow.name, "full-pipeline");
        assert_eq!(workflow.nodes.len(), 14);
    }

    #[test]
    fn legacy_cleanup_regression_fixture_manifest_is_complete() {
        let fixture_names = fs::read_dir(fixture_dir("invalid"))
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .path()
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::to_string)
            })
            .collect::<std::collections::BTreeSet<_>>();
        let required = [
            "WFS002_out-of-scope-external-event",
            "WFS002_out-of-scope-timer",
            "WFS002_out-of-scope-trigger",
            "WFS005_legacy-aggregate-all-match",
            "WFS005_legacy-aggregate-any-match",
            "WFS005_legacy-approval-node",
            "WFS005_legacy-bash-node",
            "WFS005_legacy-collect",
            "WFS005_legacy-fanout-failure-policy",
            "WFS005_legacy-global-tasks",
            "WFS005_legacy-input-contracts",
            "WFS005_legacy-node-cycle-guard",
            "WFS005_legacy-output-contract",
            "WFS005_legacy-parallel-children",
            "WFS005_legacy-parallel-node",
            "WFS005_legacy-pass-output-from",
            "WFS005_legacy-pass-previous-response",
            "WFS005_legacy-rule-expression",
            "WFS005_legacy-rule-reject",
            "WFS005_legacy-rule-rerun",
            "WFS005_legacy-rules-match-regex",
            "WFS005_legacy-type-approval",
            "WFS005_legacy-type-bash",
            "WFS005_legacy-type-field",
            "WFS005_legacy-type-parallel",
            "WFS005_legacy-workflow-variables",
        ];

        for name in required {
            assert!(
                fixture_names.contains(name),
                "legacy cleanup regression fixture is missing: {name}"
            );
        }
    }

    #[test]
    fn workflow_fixture_suite_invalid_fixtures_have_expected_diagnostic_code() {
        for entry in fs::read_dir(fixture_dir("invalid")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yml") {
                continue;
            }
            let filename = path.file_name().and_then(|name| name.to_str()).unwrap();
            let expected_code = filename.split('_').next().unwrap();
            let source = fs::read_to_string(&path).unwrap();
            let diagnosis = diagnose_workflow_source(&source, Some(filename));
            let matching = diagnosis
                .diagnostics
                .iter()
                .filter(|item| item.code == expected_code)
                .collect::<Vec<_>>();
            assert!(
                !matching.is_empty(),
                "invalid fixture {} did not produce expected code {expected_code}: {:?}",
                path.display(),
                diagnosis.diagnostics
            );
            assert!(
                matching
                    .iter()
                    .any(|item| item.stage == expected_stage_for_code(expected_code)),
                "fixture {} produced {expected_code} with wrong stage: {matching:?}",
                path.display()
            );
            assert!(
                matching.iter().any(|item| item.span.is_some()),
                "fixture {} expected {expected_code} to carry a span: {matching:?}",
                path.display()
            );

            let load_error = crate::adaptor::gateway::workflow::storage::load_workflow(
                &path,
                path.parent().expect("fixture path must have a parent"),
            )
            .expect_err("every invalid fixture must be rejected by the real loader");
            assert!(
                matches!(
                    load_error,
                    crate::adaptor::gateway::workflow::storage::StorageError::Diagnostics(ref items)
                        if items.iter().any(|item| item.code == expected_code)
                ),
                "loader rejection for {} did not preserve expected code {expected_code}: {load_error:?}",
                path.display()
            );
        }
    }

    #[test]
    fn unknown_loop_guard_reset_node_diagnostic_identifies_reference() {
        let source = fs::read_to_string(
            fixture_dir("invalid").join("WFR001_unknown-loop-guard-reset-node.yml"),
        )
        .unwrap();

        let diagnosis = diagnose_workflow_source(&source, Some("unknown-loop-guard-reset-node"));
        let diagnostic = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFR001")
            .expect("unknown reset_on node must produce WFR001");

        assert_eq!(diagnostic.node_name.as_deref(), Some("fix"));
        assert_eq!(
            diagnostic.field.as_deref(),
            Some("rules.loop_guard.reset_on")
        );
        assert!(diagnostic.message.contains("missing-boundary"));
        assert!(diagnostic.span.is_some());
    }

    #[test]
    fn invalid_source_diagnostics_use_file_stem_workflow_key() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        fs::write(
            wf_dir.join("file-stem.yml"),
            r#"
name: yaml-name
description: invalid workflow with mismatched name
nodes:
  - name: implement
    type: agent
    session:
      permission: edit
      gate: auto
      facets:
        instruction: implement
"#,
        )
        .unwrap();

        let report = diagnose_all(wf_dir, wf_dir);
        let summary = report
            .workflow_summaries
            .get("file-stem")
            .expect("invalid workflow summary must use file stem");
        let items = report
            .items
            .iter()
            .filter(|item| item.workflow_name.as_deref() == Some("file-stem"))
            .collect::<Vec<_>>();
        assert_eq!(summary.error_count, items.len());
        assert!(
            !report
                .items
                .iter()
                .any(|item| item.workflow_name.as_deref() == Some("yaml-name")),
            "invalid source diagnostics must not be keyed by YAML name: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_all_validates_facet_templates_with_workflow_context() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(
            wf_dir,
            "instructions",
            "bad",
            "Use {{ missing_node }} and {{ item.path }}",
        );
        let wf = WorkflowDefinitionYaml {
            name: "semantic-template".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_node("node1", Some("bad"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        for code in ["WFR003", "WFR005"] {
            assert!(
                report.items.iter().any(|item| item.code == code
                    && item.workflow_name.as_deref() == Some("semantic-template")
                    && item.node_name.as_deref() == Some("node1")
                    && item.facet_key.as_deref() == Some("bad")
                    && item.field.as_deref() == Some("content")
                    && item.span.is_some()),
                "expected semantic facet diagnostic {code}, got: {:?}",
                report.items
            );
        }
        let workflow_summary = report
            .workflow_summaries
            .get("semantic-template")
            .expect("semantic facet errors must count toward workflow summary");
        assert!(workflow_summary.error_count >= 2);
        let facet_summary = report
            .facet_summaries
            .get("instruction/bad")
            .expect("semantic facet errors must count toward facet summary");
        assert!(facet_summary.error_count >= 2);
    }

    #[test]
    fn invalid_rule_span_points_to_specific_rule_field() {
        let source =
            fs::read_to_string(fixture_dir("invalid").join("WFT001_when-on-enum.yml")).unwrap();
        let span_map = YamlSpanMap::parse(&source).unwrap();
        let expected = span_map
            .field_span("nodes[0].rules[0].when.on")
            .expect("fixture must have when.on span");
        let diagnosis = diagnose_workflow_source(&source, Some("WFT001_when-on-enum"));
        let item = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFT001")
            .expect("fixture must produce WFT001");
        assert_eq!(item.field.as_deref(), Some("rules.when.on"));
        assert_eq!(item.span, Some(expected));
    }

    #[test]
    fn unreachable_subgraph_targets_are_not_marked_reachable() {
        let source =
            fs::read_to_string(fixture_dir("invalid").join("WFC001_unreachable-subgraph.yml"))
                .unwrap();
        let diagnosis = diagnose_workflow_source(&source, Some("unreachable-subgraph"));
        for node_name in ["orphan", "target"] {
            assert!(
                diagnosis
                    .diagnostics
                    .iter()
                    .any(|item| item.code == "WFC001"
                        && item.stage == DiagnosticStage::ControlFlow
                        && item.node_name.as_deref() == Some(node_name)),
                "expected WFC001 for {node_name}, got: {:?}",
                diagnosis.diagnostics
            );
        }
    }

    #[test]
    fn validation_error_code_stage_uses_typed_variants() {
        let cases = vec![
            (
                validation::ValidationError::InvalidSchema {
                    schema: "list".to_string(),
                    kind: InvalidSchemaKind::UnknownSchemaReference,
                    reason: "renamed wording".to_string(),
                },
                "WFR002",
                DiagnosticStage::Resolve,
            ),
            (
                validation::ValidationError::InvalidSchema {
                    schema: "bad".to_string(),
                    kind: InvalidSchemaKind::InvalidDeclaration,
                    reason: "renamed wording".to_string(),
                },
                "WFS002",
                DiagnosticStage::ParseShape,
            ),
            (
                validation::ValidationError::InvalidArtifactReference {
                    reference: "request".to_string(),
                    kind: InvalidArtifactReferenceKind::ReservedArtifactName,
                    reason: "renamed wording".to_string(),
                },
                "WFR004",
                DiagnosticStage::Resolve,
            ),
            (
                validation::ValidationError::InvalidArtifactReference {
                    reference: "item".to_string(),
                    kind: InvalidArtifactReferenceKind::ItemOutOfScope,
                    reason: "renamed wording".to_string(),
                },
                "WFR005",
                DiagnosticStage::Resolve,
            ),
            (
                validation::ValidationError::InvalidArtifactReference {
                    reference: "fanout".to_string(),
                    kind: InvalidArtifactReferenceKind::InputsNotAllowedOnFanout,
                    reason: "renamed wording".to_string(),
                },
                "WFS004",
                DiagnosticStage::ParseShape,
            ),
            (
                validation::ValidationError::InvalidArtifactReference {
                    reference: "missing".to_string(),
                    kind: InvalidArtifactReferenceKind::UnknownNode,
                    reason: "renamed wording".to_string(),
                },
                "WFR003",
                DiagnosticStage::Resolve,
            ),
            (
                validation::ValidationError::InvalidArtifactReference {
                    reference: "plan".to_string(),
                    kind: InvalidArtifactReferenceKind::UnavailableArtifact,
                    reason: "renamed wording".to_string(),
                },
                "WFR003",
                DiagnosticStage::Resolve,
            ),
            (
                validation::ValidationError::InvalidArtifactReference {
                    reference: "plan.field".to_string(),
                    kind: InvalidArtifactReferenceKind::UnknownField,
                    reason: "renamed wording".to_string(),
                },
                "WFR003",
                DiagnosticStage::Resolve,
            ),
            (
                validation::ValidationError::InvalidArtifactReference {
                    reference: "bad ref".to_string(),
                    kind: InvalidArtifactReferenceKind::InvalidInputRef,
                    reason: "renamed wording".to_string(),
                },
                "WFR003",
                DiagnosticStage::Resolve,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::WhenFieldNotBoolean,
                    reason: "renamed wording".to_string(),
                },
                "WFT001",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::SwitchFieldNotEnum,
                    reason: "renamed wording".to_string(),
                },
                "WFT002",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::SwitchUnknownCase,
                    reason: "renamed wording".to_string(),
                },
                "WFT002",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::DiscriminatorOnFanout,
                    reason: "renamed wording".to_string(),
                },
                "WFT006",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::DiscriminatorWithoutArtifact,
                    reason: "renamed wording".to_string(),
                },
                "WFT006",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::SwitchMissingCases,
                    reason: "renamed wording".to_string(),
                },
                "WFC004",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::CycleWithoutLoopGuard,
                    reason: "renamed wording".to_string(),
                },
                "WFC005",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::LoopGuardMaxIterations,
                    reason: "renamed wording".to_string(),
                },
                "WFC005",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::SwitchRequiresNext,
                    reason: "renamed wording".to_string(),
                },
                "WFC003",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::SwitchExhaustiveHasNext,
                    reason: "renamed wording".to_string(),
                },
                "WFC003",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::MultipleNextCatchAll,
                    reason: "renamed wording".to_string(),
                },
                "WFC003",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::MultipleDiscriminators,
                    reason: "renamed wording".to_string(),
                },
                "WFC002",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::MultipleLoopGuards,
                    reason: "renamed wording".to_string(),
                },
                "WFC002",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    node: "route".to_string(),
                    kind: InvalidRuleKind::StandaloneNextWithDiscriminator,
                    reason: "renamed wording".to_string(),
                },
                "WFC002",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::UnreachableNode {
                    node: "orphan".to_string(),
                },
                "WFC001",
                DiagnosticStage::ControlFlow,
            ),
        ];

        for (error, expected_code, expected_stage) in cases {
            let (code, stage) = validation_error_code_stage(&error);
            assert_eq!(code, expected_code, "wrong code for {error:?}");
            assert_eq!(stage, expected_stage, "wrong stage for {error:?}");
        }
    }

    #[test]
    fn deserialize_error_diagnostic_classifies_error_messages() {
        let span_map = YamlSpanMap::parse("name: sample\nnodes: []\n").unwrap();
        let cases = [
            ("old workflow syntax field", "WFS005"),
            ("unknown field `type`", "WFS002"),
            ("when rule requires sibling next", "WFS003"),
            ("YAML syntax problem", "WFS001"),
            ("unclassified deserialize problem", "WFS002"),
        ];

        for (message, expected_code) in cases {
            let error = <serde_saphyr::Error as serde::de::Error>::custom(message);
            let item = deserialize_error_diagnostic(&error, &span_map, Some("sample"));
            assert_eq!(item.code, expected_code, "wrong code for {message}");
            assert_eq!(item.stage, DiagnosticStage::ParseShape);
        }
    }

    #[test]
    fn empty_workflow_nodes_diagnostic_uses_node_vocabulary_and_targets_nodes_field() {
        assert_eq!(
            validation_error_context(&validation::ValidationError::EmptyNodes),
            (None, Some("nodes".to_string()))
        );
    }

    #[test]
    fn diagnose_broken_yaml() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path().join("workflows");
        fs::create_dir_all(&wf_dir).unwrap();
        fs::write(wf_dir.join("broken.yml"), "invalid: yaml: [[[").unwrap();

        let report = diagnose_all(&wf_dir, &wf_dir);
        assert!(
            report
                .items
                .iter()
                .any(|i| i.severity == Severity::Error
                    && i.workflow_name.as_deref() == Some("broken"))
        );
    }

    #[test]
    fn diagnose_missing_facet_ref() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "knowledge", "known", "known content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "node1".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    permission: Some("edit".to_string()),
                    facets: FacetRefs {
                        knowledge: vec!["known".to_string(), "missing-knowledge".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        let missing = report
            .items
            .iter()
            .find(|item| item.code == "FAC002")
            .expect("missing knowledge FAC002");
        assert_eq!(missing.workflow_name.as_deref(), Some("test-wf"));
        assert_eq!(missing.node_name.as_deref(), Some("node1"));
        assert_eq!(missing.facet_key.as_deref(), Some("missing-knowledge"));
        assert_eq!(missing.facet_kind.as_deref(), Some("knowledge"));
        assert_eq!(missing.field.as_deref(), Some("knowledge"));
        assert!(missing.message.contains("missing-knowledge"));
    }

    #[test]
    fn diagnose_missing_input_schema_ref() {
        // Scenario: input が存在しない schemas Contract キーを参照していれば
        // workflow validation 経由でエラーになる
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                input: Some("nonexistent-contract".to_string()),
                ..make_node("node1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.node_name.as_deref() == Some("node1")
                && i.field.as_deref() == Some("input")),
            "Expected missing-input-schema error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_missing_artifact_schema_ref_remains_node_scoped() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                artifact: Some("nonexistent-contract".to_string()),
                ..make_node("node1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.node_name.as_deref() == Some("node1")
                && i.field.as_deref() == Some("artifact")),
            "Expected missing-artifact-schema error on node1, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_array_items_unknown_schema_ref_is_schema_scoped() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "review-list".to_string(),
                SchemaDef::Array {
                    items: "missing-item".to_string(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![make_node("node1", Some("impl"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("schemas.review-list")
                && i.message
                    .contains("array.items references unknown schemas 'missing-item'")
                && i.node_name.is_none()
                && i.field.as_deref() == Some("schemas")),
            "Expected schema-scoped array.items error, got: {:?}",
            report.items
        );
        assert!(
            !report
                .items
                .iter()
                .any(|i| i.node_name.as_deref() == Some("review-list")),
            "array.items diagnostics must not be attached to a schema name as a node: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_schema_refs_do_not_record_facet_usage() {
        // Scenario: schemas Contract はファセットではないため facet_usage に記録されない
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "input-contract".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinition {
                input: Some("input-contract".to_string()),
                ..make_node("node1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.facet_usage.contains_key("contracts/input-contract"),
            "schemas Contract must not be tracked as facet usage: {:?}",
            report.facet_usage
        );
    }

    #[test]
    fn diagnose_missing_input_schema_ref_in_fanout_child() {
        // Scenario: fanout child の input でも存在しない schemas Contract
        // 参照を検出する
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_fanout("parent", vec!["child1"]),
                NodeDefinition {
                    input: Some("nonexistent-contract".to_string()),
                    ..make_child("child1", Some("impl"))
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.node_name.as_deref() == Some("child1")
                && i.field.as_deref() == Some("input")),
            "Expected missing-input-schema error on child, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_missing_node_ref() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                rules: vec![Rule::Next("nonexistent".to_string())],
                ..make_node("node1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        let rule_target_errors = report
            .items
            .iter()
            .filter(|i| {
                i.severity == Severity::Error
                    && i.node_name.as_deref() == Some("node1")
                    && i.field.as_deref() == Some("rules.next")
                    && i.message.contains("存在しないnode")
            })
            .count();
        assert_eq!(
            rule_target_errors, 1,
            "expected one rules target diagnostic from validate_all, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_unreachable_node() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        // node1 が Auto + rules で node3 へ遷移 → orphan は到達不能
        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    rules: vec![Rule::Next("node3".to_string())],
                    ..make_node("node1", Some("impl"))
                },
                make_node("orphan", Some("impl")),
                make_node("node3", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.code == "WFC001"
                && i.severity == Severity::Error
                && i.stage == DiagnosticStage::ControlFlow
                && i.node_name.as_deref() == Some("orphan")),
            "Expected WFC001 for orphan, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_rules_without_fallthrough_marks_later_nodes_unreachable() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        // rules なしの node は終端なので、定義順の暗黙到達はない。
        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_node("node1", Some("impl")),
                make_node("node2", Some("impl")),
                make_node("node3", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        for node_name in ["node2", "node3"] {
            assert!(
                report.items.iter().any(|i| i.code == "WFC001"
                    && i.severity == Severity::Error
                    && i.stage == DiagnosticStage::ControlFlow
                    && i.node_name.as_deref() == Some(node_name)),
                "Expected WFC001 for {node_name}, got: {:?}",
                report.items
            );
        }
    }

    #[test]
    fn diagnose_builtin_workflow_info() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Info && i.message.contains("ビルトインワークフロー")));
    }

    #[test]
    fn diagnose_builtin_facet_info() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Info && i.message.contains("ビルトインファセット")));
    }

    #[test]
    fn diagnose_template_variable_error() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "bad", "Use {{request.field}} here");
        let wf = WorkflowDefinitionYaml {
            name: "bad-template".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_node("node1", Some("bad"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.facet_key.as_deref() == Some("bad")
            && i.message
                .contains("未定義のテンプレート変数 '{{request.field}}'")));
    }

    #[test]
    fn diagnose_request_reference_ok() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "good", "Request: {{ request }}");

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(!report.items.iter().any(|i| i.severity == Severity::Error
            && i.facet_key.as_deref() == Some("good")
            && i.message.contains("未定義のテンプレート変数")));
    }

    /// command node は command を持ち facet は不要。
    /// diagnose_all 経路で valid な command node が誤って「ファセット参照が必要」
    /// エラーにならないことを担保する（validation.rs と同じ整合性が diagnostics 側にも必要）。
    #[test]
    fn diagnose_command_node_with_command_has_no_facet_required_error() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let wf = WorkflowDefinitionYaml {
            name: "command-wf".to_string(),
            description: "command test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_command("build", "cargo build")],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.node_name.as_deref() == Some("build")
                && i.message.contains("ファセット参照")),
            "command node with command must not trigger facet requirement error: {:?}",
            report.items
        );
    }

    /// command node の command が空なら validation 経路で command field のエラーになる。
    #[test]
    fn diagnose_command_node_with_empty_command_reports_command_error() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let wf = WorkflowDefinitionYaml {
            name: "command-wf".to_string(),
            description: "command test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_command("build", "   ")],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report
                .items
                .iter()
                .any(|i| i.severity == Severity::Error && i.message.contains("command")),
            "command node without command must report a command-related error: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_facet_usage_tracked() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_node("node1", Some("impl"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        let usage = report.facet_usage.get("instruction/impl");
        assert!(usage.is_some());
        assert_eq!(usage.unwrap().len(), 1);
        assert_eq!(usage.unwrap()[0].workflow_name, "test-wf");
    }

    #[test]
    fn diagnose_tracks_each_knowledge_reference_usage() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "knowledge", "first", "first content");
        setup_facet(wf_dir, "knowledge", "second", "second content");

        let wf = WorkflowDefinitionYaml {
            name: "knowledge-usage".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "node1".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    permission: Some("edit".to_string()),
                    facets: FacetRefs {
                        knowledge: vec!["first".to_string(), "second".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        for facet_id in ["knowledge/first", "knowledge/second"] {
            let usages = report
                .facet_usage
                .get(facet_id)
                .unwrap_or_else(|| panic!("missing usage for {facet_id}"));
            assert_eq!(usages.len(), 1);
            assert_eq!(usages[0].workflow_name, "knowledge-usage");
            assert_eq!(usages[0].node_name, "node1");
            assert_eq!(usages[0].slot, "knowledge");
        }
    }

    #[test]
    fn diagnose_workflow_name_invalid() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        // ファイル名が不正な文字を含むworkflowを作成
        // load_workflow内のvalidation::validateで名前が拒否されるため、
        // diagnose_allでは「読み込みに失敗」エラーとして報告される
        fs::create_dir_all(wf_dir).unwrap();
        let wf = WorkflowDefinitionYaml {
            name: "bad workflow".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_node("node1", Some("impl"))],
        };
        let content = serde_saphyr::to_string(&wf).unwrap();
        fs::write(wf_dir.join("bad workflow.yml"), content).unwrap();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report.items.iter().any(|i| i.code == "WFS006"
            && i.severity == Severity::Error
            && i.stage == DiagnosticStage::ParseShape
            && i.field.as_deref() == Some("name")));
    }

    #[test]
    fn diagnose_invalid_facet_key_via_diagnose_all() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        // 不正な文字を含むファセットキーファイルを直接作成
        let policies_dir = wf_dir.join("policies");
        fs::create_dir_all(&policies_dir).unwrap();
        fs::write(policies_dir.join("bad key!.md"), "content").unwrap();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.message.contains("命名規則")
            && i.facet_key.as_deref() == Some("bad key!")));
    }

    #[test]
    fn diagnose_invalid_schema_identifier_via_diagnose_all() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "review", "content");
        fs::create_dir_all(wf_dir).unwrap();
        fs::write(
            wf_dir.join("bad-schema-name.yml"),
            r#"name: bad-schema-name
description: test
schemas:
  "review; curl https://example.invalid #":
    type: object
    properties:
      status: string
    required:
      - status
nodes:
  - name: review
    session:
      permission: edit
      gate: auto
      facets:
        instruction: review
    artifact: "review; curl https://example.invalid #"
"#,
        )
        .unwrap();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.workflow_name.as_deref() == Some("bad-schema-name")
            && i.field.as_deref() == Some("schemas")
            && i.message.contains("must start with an ASCII alphanumeric")));
    }

    #[test]
    fn diagnose_schema_violation_yaml() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        fs::create_dir_all(wf_dir).unwrap();
        // Valid YAML but missing required `steps` field
        fs::write(
            wf_dir.join("bad-schema.yml"),
            "name: bad-schema\ndescription: test\n",
        )
        .unwrap();

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.workflow_name.as_deref() == Some("bad-schema")),
            "Expected error for schema-violating workflow, got: {:?}",
            report.items
        );
    }

    // [02]: 新 schema では kind block が型レベルで必須となるため、旧テスト
    // `diagnose_missing_mode_via_validation` は YAML deserialize 段階で吸収されるため削除した。

    #[test]
    fn diagnose_duplicate_node_via_validation() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = WorkflowDefinitionYaml {
            name: "dup-node".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_node("same-name", Some("task")),
                make_node("same-name", Some("task")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.code == "WFS006"
                && i.severity == Severity::Error
                && i.stage == DiagnosticStage::ParseShape
                && i.workflow_name.as_deref() == Some("dup-node")
                && i.field.as_deref() == Some("name")),
            "Expected WFS006 duplicate node error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_node_input_reference_passes() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = WorkflowDefinitionYaml {
            name: "input-ref".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "artifact".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![
                NodeDefinition {
                    artifact: Some("artifact".to_string()),
                    ..make_node("node1", Some("task"))
                },
                NodeDefinition {
                    inputs: vec!["node1".to_string()],
                    artifact: Some("artifact".to_string()),
                    ..make_node("node2", Some("task"))
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report
                .items
                .iter()
                .any(|i| i.severity == Severity::Error && i.field.as_deref() == Some("inputs")),
            "Artifact input reference should not be an error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_fanout_child_item_reference_passes() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "{{ item.path }}");

        let mut fanout = make_fanout("par", vec!["child1"]);
        let NodeKind::Fanout(fanout_spec) = &mut fanout.kind else {
            unreachable!();
        };
        fanout_spec.items = Some(ItemsSource::Literal(vec![serde_json::json!({
            "path": "src/lib.rs"
        })]));
        let wf = WorkflowDefinitionYaml {
            name: "par-item".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "item-contract".to_string(),
                SchemaDef::Object {
                    properties: [("path".to_string(), SchemaDef::String { r#enum: None })]
                        .into_iter()
                        .collect(),
                    required: ["path".to_string()].into_iter().collect(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![
                fanout,
                NodeDefinition {
                    input: Some("item-contract".to_string()),
                    ..make_child("child1", Some("task"))
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.node_name.as_deref() == Some("child1")
                && i.field.as_deref() == Some("inputs")),
            "item reference inside fanout child should not be an error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_fanout_inputs_rejected() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let mut fanout = make_fanout("par", vec!["child1", "child2"]);
        fanout.inputs = vec!["request".to_string()];
        let wf = WorkflowDefinitionYaml {
            name: "fanout-inputs".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                fanout,
                make_child("child1", Some("task")),
                make_child("child2", Some("task")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.workflow_name.as_deref() == Some("fanout-inputs")
                && i.field.as_deref() == Some("inputs")
                && i.message.contains("fanout")),
            "Expected fanout inputs error, got: {:?}",
            report.items
        );
    }
}
