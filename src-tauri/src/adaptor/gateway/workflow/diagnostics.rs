use crate::adaptor::gateway::workflow::builtin;
use crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain;
use crate::adaptor::gateway::workflow::facet::{self, FacetKind};
use crate::adaptor::gateway::workflow::prompt_rendering;
use crate::adaptor::gateway::workflow::schema::{NodeDefinition, ReduceStrategy, Rule, Workflow};
use crate::adaptor::gateway::workflow::span_map::YamlSpanMap;
use crate::domain::workflow::validation;
use crate::domain::workflow::validation::{
    InvalidArtifactReferenceKind, InvalidRuleKind, InvalidSchemaKind,
};
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
    Warning,
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
    /// 対象の step 名
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_name: Option<String>,
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct DiagnosticSpan {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

impl DiagnosticSpan {
    fn from_location(location: serde_saphyr::Location) -> Self {
        let line = usize::try_from(location.line()).unwrap_or(usize::MAX);
        let col = usize::try_from(location.column()).unwrap_or(usize::MAX);
        Self {
            start_line: line,
            start_col: col,
            end_line: line,
            end_col: col.saturating_add(1),
        }
    }

    fn from_scan_error(error: &serde_saphyr::granit_parser::ScanError) -> Self {
        let marker = error.marker();
        Self {
            start_line: marker.line(),
            start_col: marker.col() + 1,
            end_line: marker.line(),
            end_col: marker.col() + 2,
        }
    }
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
            step_name: None,
            facet_key: None,
            facet_kind: None,
            field: None,
        }
    }

    fn workflow(mut self, name: impl Into<String>) -> Self {
        self.workflow_name = Some(name.into());
        self
    }

    fn step(mut self, name: impl Into<String>) -> Self {
        self.step_name = Some(name.into());
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
    pub warning_count: usize,
    pub info_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReport {
    pub items: Vec<DiagnosticItem>,
    /// workflow名 → そのworkflowの診断サマリ
    pub workflow_summaries: HashMap<String, DiagnosticSummary>,
    /// "kind/key" → そのファセットの診断サマリ
    pub facet_summaries: HashMap<String, DiagnosticSummary>,
    /// ファセットキー → 参照元workflow/step情報のリスト
    pub facet_usage: HashMap<String, Vec<FacetUsageEntry>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FacetUsageEntry {
    pub workflow_name: String,
    pub step_name: String,
    pub slot: String,
}

type LoadedWorkflowDiagnostics = Result<(Workflow, Vec<DiagnosticItem>), Vec<DiagnosticItem>>;
type NamedWorkflowDiagnostics = (String, LoadedWorkflowDiagnostics);

#[derive(Debug, Clone)]
pub(crate) struct WorkflowSourceDiagnostics {
    pub(crate) workflow: Option<Workflow>,
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

    let workflow = match serde_saphyr::from_str::<Workflow>(source) {
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
    wf: &Workflow,
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
        &["steps", "variables"],
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
        let step_name = node_obj
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("<unknown>");
        check_allowed_fields(
            node_obj,
            &node_path,
            &[
                "name", "command", "session", "fanout", "artifact", "input", "inputs", "collect",
                "rules",
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
                "cycle_guard",
                "resets_cycle_for",
            ],
            span_map,
            workflow_name,
            Some(step_name),
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
                        "node '{step_name}' must contain exactly one kind block: command, session, or fanout"
                    ),
                )
                .workflow(workflow_name)
                .step(step_name)
                .field("kind"),
            );
        }
        if step_name == "request" || step_name == "item" {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFR004",
                    Severity::Error,
                    DiagnosticStage::Resolve,
                    span_map.field_span(&format!("{node_path}.name")),
                    format!("node name '{step_name}' is reserved"),
                )
                .workflow(workflow_name)
                .step(step_name)
                .field("name"),
            );
        }
        if step_name != "<unknown>"
            && (validation::validate_name(step_name).is_err() || !names.insert(step_name))
        {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS006",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.field_span(&format!("{node_path}.name")),
                    format!("node name '{step_name}' is duplicated or invalid"),
                )
                .workflow(workflow_name)
                .step(step_name)
                .field("name"),
            );
        }
        if node_obj.contains_key("fanout") {
            for field in ["inputs", "collect"] {
                if node_obj.contains_key(field) {
                    diagnostics.push(kind_disallowed_diagnostic(
                        workflow_name,
                        step_name,
                        "fanout",
                        field,
                        span_map.field_span(&format!("{node_path}.{field}")),
                    ));
                }
            }
        }
        if node_obj.contains_key("command") && node_obj.contains_key("collect") {
            diagnostics.push(kind_disallowed_diagnostic(
                workflow_name,
                step_name,
                "command",
                "collect",
                span_map.field_span(&format!("{node_path}.collect")),
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
                Some(step_name),
                &mut diagnostics,
            );
            if let Some(facets) = session.get("facets").and_then(serde_json::Value::as_object) {
                check_allowed_fields(
                    facets,
                    &format!("{node_path}.session.facets"),
                    &["policy", "knowledge", "instruction"],
                    &[],
                    span_map,
                    workflow_name,
                    Some(step_name),
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
                &["parallel_children", "aggregate"],
                &[],
                span_map,
                workflow_name,
                Some(step_name),
                &mut diagnostics,
            );
            if let Some(children) = fanout
                .get("parallel_children")
                .and_then(serde_json::Value::as_array)
            {
                for (child_index, child) in children.iter().enumerate() {
                    let child_path = format!("{node_path}.fanout.parallel_children[{child_index}]");
                    if let Some(child_obj) = child.as_object() {
                        let child_name = child_obj
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("<unknown>");
                        check_allowed_fields(
                            child_obj,
                            &child_path,
                            &["name", "model", "permission", "facets", "artifact", "input"],
                            &["type", "mode", "prompt", "inline_prompt", "output_contract"],
                            span_map,
                            workflow_name,
                            Some(child_name),
                            &mut diagnostics,
                        );
                    }
                }
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
                    &["match", "cycle_guard", "resets_cycle_for"],
                    span_map,
                    workflow_name,
                    Some(step_name),
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
                        .step(step_name)
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
    step_name: Option<&str>,
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
        if let Some(step_name) = step_name {
            item = item.step(step_name);
        }
        diagnostics.push(item);
    }
}

fn kind_disallowed_diagnostic(
    workflow_name: &str,
    step_name: &str,
    kind: &str,
    field: &str,
    span: Option<DiagnosticSpan>,
) -> DiagnosticItem {
    DiagnosticItem::new(
        "WFS004",
        Severity::Error,
        DiagnosticStage::ParseShape,
        span,
        format!("node '{step_name}' ({kind}) cannot declare '{field}'"),
    )
    .workflow(workflow_name)
    .step(step_name)
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
    wf: &Workflow,
    error: &validation::ValidationError,
    span_map: Option<&YamlSpanMap>,
) -> DiagnosticItem {
    let (code, stage) = validation_error_code_stage(error);
    let (step_name, field) = validation_error_context(error);
    let span = span_map.and_then(|map| span_for_validation_error(wf, error, map));
    let mut item = DiagnosticItem::new(code, Severity::Error, stage, span, error.to_string())
        .workflow(wf.name.clone());
    if let Some(step_name) = step_name {
        item = item.step(step_name);
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
        | ValidationError::EmptySteps
        | ValidationError::DuplicateStep { .. }
        | ValidationError::ParallelChildNameConflict { .. }
        | ValidationError::EmptyCommand { .. }
        | ValidationError::DisallowedFieldForKind { .. }
        | ValidationError::TooManyNodes { .. }
        | ValidationError::TooManyParallelChildren { .. } => "WFS006",
        ValidationError::UnknownRuleTarget { .. }
        | ValidationError::AggregateUnknownTarget { .. }
        | ValidationError::UnknownCollectFrom { .. } => "WFR001",
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
        ValidationError::AggregateInvalidConfig { .. } => "WFC002",
        ValidationError::MissingFacet { .. }
        | ValidationError::ParallelChildMissingFacet { .. } => "WFR900",
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
    wf: &Workflow,
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
        ValidationError::InvalidRules { step, kind, .. } => invalid_rule_path(wf, step, *kind)
            .and_then(|path| span_map.field_span(&path))
            .or_else(|| {
                step_base_path(wf, step)
                    .and_then(|path| span_map.field_span(&format!("{path}.rules")))
            }),
        ValidationError::UnreachableNode { step } => {
            step_base_path(wf, step).and_then(|path| span_map.nearest_span(&path))
        }
        _ => {
            let (step_name, field) = validation_error_context(error);
            match (step_name.as_deref(), field.as_deref()) {
                (Some(step_name), Some(field)) => step_field_path(wf, step_name, field)
                    .and_then(|path| span_map.field_span(&path)),
                (Some(step_name), None) => {
                    step_base_path(wf, step_name).and_then(|path| span_map.nearest_span(&path))
                }
                (None, Some(field)) => span_map.field_span(field),
                (None, None) => span_map.nearest_span(""),
            }
        }
    }
}

fn input_reference_path(wf: &Workflow, reference: &str) -> Option<String> {
    for (index, node) in wf.nodes.iter().enumerate() {
        for (input_index, input) in node.inputs.iter().enumerate() {
            if input == reference {
                return Some(format!("nodes[{index}].inputs[{input_index}]"));
            }
        }
    }
    None
}

fn invalid_rule_path(wf: &Workflow, step_name: &str, kind: InvalidRuleKind) -> Option<String> {
    let (base, node) = step_base_path_with_node(wf, step_name)?;
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

fn step_base_path_with_node<'a>(
    wf: &'a Workflow,
    step_name: &str,
) -> Option<(String, &'a NodeDefinition)> {
    for (index, node) in wf.nodes.iter().enumerate() {
        if node.name == step_name {
            return Some((format!("nodes[{index}]"), node));
        }
    }
    None
}

fn rule_index(node: &NodeDefinition, matches_rule: impl Fn(&Rule) -> bool) -> Option<usize> {
    node.rules.iter().position(matches_rule)
}

fn step_base_path(wf: &Workflow, step_name: &str) -> Option<String> {
    for (index, node) in wf.nodes.iter().enumerate() {
        if node.name == step_name {
            return Some(format!("nodes[{index}]"));
        }
        if let Some(fanout) = node.fanout() {
            for (child_index, child) in fanout.parallel_children.iter().enumerate() {
                if child.name == step_name {
                    return Some(format!(
                        "nodes[{index}].fanout.parallel_children[{child_index}]"
                    ));
                }
            }
        }
    }
    None
}

fn step_field_path(wf: &Workflow, step_name: &str, field: &str) -> Option<String> {
    let base = step_base_path(wf, step_name)?;
    let suffix = if base.contains("parallel_children") {
        match field {
            "permission" | "model" | "artifact" | "input" | "facets" => field.to_string(),
            field => field.replace("parallel_children.", ""),
        }
    } else {
        match field {
            "permission" | "model" => format!("session.{field}"),
            "facets" => "session.facets".to_string(),
            "rules.next" => "rules".to_string(),
            "parallel_children" | "parallel_children.facets" => {
                "fanout.parallel_children".to_string()
            }
            field => field.to_string(),
        }
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
    let workflow_lookup: HashMap<&str, &Workflow> = workflows
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
            let facet_id = format!("{}/{}", kind.dir_name(), summary.key);

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
                .facet(summary.key.clone(), kind.dir_name().to_string())
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
                        kind.dir_name()
                    ),
                )
                .facet(summary.key.clone(), kind.dir_name().to_string());
                add_diagnostic(&mut items, &mut facet_summaries, &facet_id, item);
            }

            // テンプレート変数チェック
            if let Ok(content) = facet::load_facet(*kind, &summary.key, facets_base_dir) {
                check_template_variables(
                    &content,
                    &summary.key,
                    kind.dir_name(),
                    &facet_id,
                    &mut items,
                    &mut facet_summaries,
                );
                check_facet_template_references(
                    &content,
                    &summary.key,
                    kind.dir_name(),
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
                keys.insert(format!("{}/{}", kind.dir_name(), key));
            }
        }
    }
    keys
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

/// ValidationError からステップ名とフィールド名を抽出
fn validation_error_context(e: &validation::ValidationError) -> (Option<String>, Option<String>) {
    use validation::ValidationError;
    match e {
        ValidationError::EmptyName | ValidationError::InvalidChars { .. } => {
            (None, Some("name".to_string()))
        }
        ValidationError::EmptySteps => (None, Some("steps".to_string())),
        ValidationError::DuplicateStep { name } => (Some(name.clone()), Some("name".to_string())),
        ValidationError::ParallelChildNameConflict { child } => (
            Some(child.clone()),
            Some("parallel_children.name".to_string()),
        ),
        ValidationError::AggregateInvalidConfig { step, .. } => {
            (Some(step.clone()), Some("aggregate".to_string()))
        }
        ValidationError::AggregateUnknownTarget { step, .. } => {
            (Some(step.clone()), Some("aggregate".to_string()))
        }
        ValidationError::ParallelChildMissingFacet { parent, .. } => (
            Some(parent.clone()),
            // 旧ラベル "parallel.facets" と同じく、policy/knowledge/instruction/artifact_contract
            // のいずれかの欠落を表す論理グループ名として "facets" を用いる
            // （新 schema の YAML キーは個別だが、`MissingFacet` 側も "facets" 表現）。
            Some("parallel_children.facets".to_string()),
        ),
        ValidationError::UnknownRuleTarget { step, .. } => {
            (Some(step.clone()), Some("rules.next".to_string()))
        }
        ValidationError::InvalidRules { step, kind, .. } => (
            Some(step.clone()),
            Some(invalid_rule_field_name(*kind).to_string()),
        ),
        ValidationError::UnreachableNode { step } => {
            (Some(step.clone()), Some("nodes".to_string()))
        }
        ValidationError::MissingFacet { step } => (Some(step.clone()), Some("facets".to_string())),
        ValidationError::UnknownCollectFrom { step, .. } => {
            (Some(step.clone()), Some("collect.from".to_string()))
        }
        ValidationError::InvalidArtifactReference { .. } => (None, Some("inputs".to_string())),
        ValidationError::InvalidPermissionMode { step, .. } => {
            (Some(step.clone()), Some("permission".to_string()))
        }
        ValidationError::MissingPermissionMode { step } => {
            (Some(step.clone()), Some("permission".to_string()))
        }
        ValidationError::UnknownModel { step, .. } => {
            (Some(step.clone()), Some("model".to_string()))
        }
        ValidationError::InvalidModelFormat { step, .. } => {
            (Some(step.clone()), Some("model".to_string()))
        }
        ValidationError::ModelResolutionFailed { step, .. } => {
            (Some(step.clone()), Some("model".to_string()))
        }
        ValidationError::EmptyCommand { step } => (Some(step.clone()), Some("command".to_string())),
        ValidationError::DisallowedFieldForKind { step, field, .. } => {
            (Some(step.clone()), Some(field.to_string()))
        }
        ValidationError::TooManyNodes { .. } => (None, Some("nodes".to_string())),
        ValidationError::TooManyParallelChildren { step, .. } => {
            (Some(step.clone()), Some("parallel_children".to_string()))
        }
        ValidationError::UnknownSchemaRef { step, slot, .. }
        | ValidationError::InvalidSchemaRef { step, slot, .. } => {
            (Some(step.clone()), Some((*slot).to_string()))
        }
        ValidationError::InvalidSchema { .. } => (None, Some("schemas".to_string())),
        ValidationError::InvalidArtifactSchema { step, .. }
        | ValidationError::ReservedArtifactField { step, .. } => {
            (Some(step.clone()), Some("artifact".to_string()))
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
    wf: &Workflow,
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

    // 各stepを診断
    for step in &wf.nodes {
        if let Some(ref collect) = step.collect {
            // collect元stepにrulesがないwarning
            if matches!(
                collect.reduce,
                ReduceStrategy::AnyNeedsFix | ReduceStrategy::AllPassed
            ) {
                for from in &collect.from {
                    if let Some(source_step) = wf.nodes.iter().find(|s| s.name == *from) {
                        if source_step.rules.is_empty() && !source_step.is_fanout() {
                            let item = DiagnosticItem::new(
                                "WFC900",
                                Severity::Warning,
                                DiagnosticStage::ControlFlow,
                                None,
                                format!(
                                    "collect元ステップ '{}' にrulesが未設定です（{:?}リデュースで結果がNoneになる可能性）",
                                    from, collect.reduce
                                ),
                            )
                            .workflow(name.clone())
                            .step(step.name.clone())
                            .field("collect.reduce");
                            add_diagnostic(items, workflow_summaries, name, item);
                        }
                    }
                }
            }
        }

        // ファセット参照の存在チェック + usage 記録
        FacetRefCheckContext::new(name, all_facet_keys, items, workflow_summaries, facet_usage)
            .check_step(
                &step.name,
                &FacetRefs {
                    policy: step
                        .session()
                        .and_then(|session| session.facets.policy.as_deref()),
                    knowledge: step
                        .session()
                        .and_then(|session| session.facets.knowledge.as_deref()),
                    instruction: step
                        .session()
                        .and_then(|session| session.facets.instruction.as_deref()),
                },
            );

        // parallel block の子step 診断
        if let Some(fanout) = step.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                FacetRefCheckContext::new(
                    name,
                    all_facet_keys,
                    items,
                    workflow_summaries,
                    facet_usage,
                )
                .check_step(
                    &child.name,
                    &FacetRefs {
                        policy: child.facets.policy.as_deref(),
                        knowledge: child.facets.knowledge.as_deref(),
                        instruction: child.facets.instruction.as_deref(),
                    },
                );
            }
        }
    }
}

struct FacetRefs<'a> {
    policy: Option<&'a str>,
    knowledge: Option<&'a str>,
    instruction: Option<&'a str>,
}

/// 複数の facet 参照を 1 つの step スコープで一括検査するためのコンテキスト。
///
/// 旧 `check_single_facet_ref` は 9 引数で都度 sink / 一覧 / workflow 名を渡していたが、
/// それらは「`diagnose_workflow` の 1 走行を通じて共有される」性質のもの。
/// このコンテキストにまとめて `ctx.check(step, slot, kind, key)` の形で呼び出すことで、
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
    fn check(&mut self, step_name: &str, slot: &str, kind: FacetKind, key: &str) {
        let facet_id = format!("{}/{}", kind.dir_name(), key);

        self.facet_usage
            .entry(facet_id.clone())
            .or_default()
            .push(FacetUsageEntry {
                workflow_name: self.workflow_name.to_string(),
                step_name: step_name.to_string(),
                slot: slot.to_string(),
            });

        if !self.all_facet_keys.contains(&facet_id) {
            let item = DiagnosticItem::new(
                "FAC002",
                Severity::Error,
                DiagnosticStage::Resolve,
                None,
                format!(
                    "ステップ '{}' が存在しないファセット '{}' ({}) を参照しています",
                    step_name,
                    key,
                    kind.dir_name()
                ),
            )
            .workflow(self.workflow_name.to_string())
            .step(step_name.to_string())
            .facet(key.to_string(), kind.dir_name().to_string())
            .field(slot.to_string());
            add_diagnostic(
                self.items,
                self.workflow_summaries,
                self.workflow_name,
                item,
            );
        }
    }

    /// 1 つの step が持つ全 facet ref を一括検査する。
    fn check_step(&mut self, step_name: &str, facet_refs: &FacetRefs<'_>) {
        let singles: &[(&str, FacetKind, Option<&str>)] = &[
            ("policy", FacetKind::Policy, facet_refs.policy),
            ("knowledge", FacetKind::Knowledge, facet_refs.knowledge),
            (
                "instruction",
                FacetKind::Instruction,
                facet_refs.instruction,
            ),
        ];
        for (slot, kind, key_opt) in singles {
            if let Some(key) = key_opt {
                self.check(step_name, slot, *kind, key);
            }
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
    workflow_lookup: &HashMap<&str, &Workflow>,
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
        let allow_item = facet_usage_allows_item(workflow, &usage.step_name);
        for error in validation::validate_template_references(&domain_workflow, content, allow_item)
        {
            let span = facet_template_error_span(content, &error);
            let mut item = validation_error_to_diagnostic(workflow, &error, None)
                .facet(facet_key.to_string(), facet_kind_name.to_string())
                .step(usage.step_name.clone())
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

fn facet_usage_allows_item(workflow: &Workflow, step_name: &str) -> bool {
    workflow.nodes.iter().any(|node| {
        node.fanout().is_some_and(|fanout| {
            fanout
                .parallel_children
                .iter()
                .any(|child| child.name == step_name)
        })
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
        Severity::Warning => summary.warning_count += 1,
        Severity::Info => summary.info_count += 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        CollectConfig, CommandSpec, FacetRefs, FanoutSpec, InterimChild, NodeKind, ReduceStrategy,
        Rule, SchemaDef, SessionSpec, Workflow,
    };
    use std::fs;
    use tempfile::TempDir;

    fn make_step(name: &str, instruction: Option<&str>) -> NodeDefinition {
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

    fn make_child(name: &str, instruction: Option<&str>) -> InterimChild {
        InterimChild {
            name: name.to_string(),
            permission: Some("edit".to_string()),
            facets: FacetRefs {
                instruction: instruction.map(str::to_string),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn make_fanout(name: &str, children: Vec<InterimChild>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                parallel_children: children,
                aggregate: None,
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

    fn save_workflow_yaml(dir: &Path, wf: &Workflow) {
        fs::create_dir_all(dir).unwrap();
        let content = serde_saphyr::to_string(wf).unwrap();
        fs::write(dir.join(format!("{}.yml", wf.name)), content).unwrap();
    }

    fn fixture_dir(kind: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/adaptor/gateway/workflow/fixtures")
            .join(kind)
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
    fn workflow_fixture_suite_valid_has_no_error_diagnostics() {
        for entry in fs::read_dir(fixture_dir("valid")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yml") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap();
            let diagnosis =
                diagnose_workflow_source(&source, path.file_stem().and_then(|stem| stem.to_str()));
            let errors = diagnosis
                .diagnostics
                .iter()
                .filter(|item| item.severity == Severity::Error)
                .collect::<Vec<_>>();
            assert!(
                errors.is_empty(),
                "valid fixture {} produced errors: {errors:?}",
                path.display()
            );
        }
    }

    #[test]
    fn workflow_fixture_suite_invalid_fixes_expected_diagnostic_code() {
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
        }
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
        let wf = Workflow {
            name: "semantic-template".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_step("step1", Some("bad"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        for code in ["WFR003", "WFR005"] {
            assert!(
                report.items.iter().any(|item| item.code == code
                    && item.workflow_name.as_deref() == Some("semantic-template")
                    && item.step_name.as_deref() == Some("step1")
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
            .get("instructions/bad")
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
        for step_name in ["orphan", "target"] {
            assert!(
                diagnosis
                    .diagnostics
                    .iter()
                    .any(|item| item.code == "WFC001"
                        && item.stage == DiagnosticStage::ControlFlow
                        && item.step_name.as_deref() == Some(step_name)),
                "expected WFC001 for {step_name}, got: {:?}",
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
                    step: "route".to_string(),
                    kind: InvalidRuleKind::WhenFieldNotBoolean,
                    reason: "renamed wording".to_string(),
                },
                "WFT001",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::SwitchFieldNotEnum,
                    reason: "renamed wording".to_string(),
                },
                "WFT002",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::SwitchUnknownCase,
                    reason: "renamed wording".to_string(),
                },
                "WFT002",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::DiscriminatorOnFanout,
                    reason: "renamed wording".to_string(),
                },
                "WFT006",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::DiscriminatorWithoutArtifact,
                    reason: "renamed wording".to_string(),
                },
                "WFT006",
                DiagnosticStage::Typecheck,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::SwitchMissingCases,
                    reason: "renamed wording".to_string(),
                },
                "WFC004",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::CycleWithoutLoopGuard,
                    reason: "renamed wording".to_string(),
                },
                "WFC005",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::LoopGuardMaxIterations,
                    reason: "renamed wording".to_string(),
                },
                "WFC005",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::SwitchRequiresNext,
                    reason: "renamed wording".to_string(),
                },
                "WFC003",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::SwitchExhaustiveHasNext,
                    reason: "renamed wording".to_string(),
                },
                "WFC003",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::MultipleNextCatchAll,
                    reason: "renamed wording".to_string(),
                },
                "WFC003",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::MultipleDiscriminators,
                    reason: "renamed wording".to_string(),
                },
                "WFC002",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::MultipleLoopGuards,
                    reason: "renamed wording".to_string(),
                },
                "WFC002",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidRules {
                    step: "route".to_string(),
                    kind: InvalidRuleKind::StandaloneNextWithDiscriminator,
                    reason: "renamed wording".to_string(),
                },
                "WFC002",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::UnreachableNode {
                    step: "orphan".to_string(),
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
    fn builtin_workflows_have_zero_validation_diagnostics() {
        for summary in builtin::list_builtin_workflows() {
            let workflow = builtin::load_builtin_workflow_resolved(&summary.name)
                .unwrap_or_else(|error| panic!("builtin {} failed to load: {error}", summary.name))
                .unwrap_or_else(|| panic!("builtin {} not found", summary.name));
            let diagnostics = diagnose_workflow_definition(&workflow, None);
            let errors = diagnostics
                .iter()
                .filter(|item| item.severity == Severity::Error)
                .collect::<Vec<_>>();
            assert!(
                errors.is_empty(),
                "builtin {} produced diagnostics: {errors:?}",
                summary.name
            );
        }
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

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_step("step1", Some("nonexistent-instruction"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Error && i.message.contains("存在しないファセット")));
    }

    #[test]
    fn diagnose_missing_input_schema_ref() {
        // Scenario: input が存在しない schemas Contract キーを参照していれば
        // workflow validation 経由でエラーになる
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                input: Some("nonexistent-contract".to_string()),
                ..make_step("step1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.step_name.as_deref() == Some("step1")
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

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                artifact: Some("nonexistent-contract".to_string()),
                ..make_step("step1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.step_name.as_deref() == Some("step1")
                && i.field.as_deref() == Some("artifact")),
            "Expected missing-artifact-schema error on step1, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_array_items_unknown_schema_ref_is_schema_scoped() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
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
            nodes: vec![make_step("step1", Some("impl"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("schemas.review-list")
                && i.message
                    .contains("array.items references unknown schemas 'missing-item'")
                && i.step_name.is_none()
                && i.field.as_deref() == Some("schemas")),
            "Expected schema-scoped array.items error, got: {:?}",
            report.items
        );
        assert!(
            !report
                .items
                .iter()
                .any(|i| i.step_name.as_deref() == Some("review-list")),
            "array.items diagnostics must not be attached to a schema name as a step: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_schema_refs_do_not_record_facet_usage() {
        // Scenario: schemas Contract はファセットではないため facet_usage に記録されない
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "input-contract".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                    additional_properties: false,
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinition {
                input: Some("input-contract".to_string()),
                ..make_step("step1", Some("impl"))
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
    fn diagnose_missing_input_schema_ref_in_parallel_child() {
        // Scenario: parallel child の input でも存在しない schemas Contract
        // 参照を検出する
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_fanout(
                "parent",
                vec![InterimChild {
                    name: "child1".to_string(),
                    facets: FacetRefs {
                        instruction: Some("impl".to_string()),
                        ..Default::default()
                    },
                    input: Some("nonexistent-contract".to_string()),
                    ..Default::default()
                }],
            )],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.step_name.as_deref() == Some("child1")
                && i.field.as_deref() == Some("input")),
            "Expected missing-input-schema error on child, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_missing_step_ref() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                rules: vec![Rule::Next("nonexistent".to_string())],
                ..make_step("step1", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        let rule_target_errors = report
            .items
            .iter()
            .filter(|i| {
                i.severity == Severity::Error
                    && i.step_name.as_deref() == Some("step1")
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
    fn diagnose_collect_warning() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "review", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    // source step without rules
                    ..make_step("review-step", Some("review"))
                },
                NodeDefinition {
                    collect: Some(CollectConfig {
                        from: vec!["review-step".to_string()],
                        reduce: ReduceStrategy::AnyNeedsFix,
                    }),
                    ..make_step("collect-step", None)
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Warning && i.message.contains("rulesが未設定")));
    }

    #[test]
    fn diagnose_unreachable_step() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        // step1 が Auto + rules で step3 へ遷移 → orphan は到達不能
        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    rules: vec![Rule::Next("step3".to_string())],
                    ..make_step("step1", Some("impl"))
                },
                make_step("orphan", Some("impl")),
                make_step("step3", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.code == "WFC001"
                && i.severity == Severity::Error
                && i.stage == DiagnosticStage::ControlFlow
                && i.step_name.as_deref() == Some("orphan")),
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
        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_step("step1", Some("impl")),
                make_step("step2", Some("impl")),
                make_step("step3", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        for step_name in ["step2", "step3"] {
            assert!(
                report.items.iter().any(|i| i.code == "WFC001"
                    && i.severity == Severity::Error
                    && i.stage == DiagnosticStage::ControlFlow
                    && i.step_name.as_deref() == Some(step_name)),
                "Expected WFC001 for {step_name}, got: {:?}",
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
        let wf = Workflow {
            name: "bad-template".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_step("step1", Some("bad"))],
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

        let wf = Workflow {
            name: "bash-wf".to_string(),
            description: "bash test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_command("build", "cargo build")],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.step_name.as_deref() == Some("build")
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

        let wf = Workflow {
            name: "bash-wf".to_string(),
            description: "bash test".to_string(),
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
            "bash node without command must report a command-related error: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_facet_usage_tracked() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_step("step1", Some("impl"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        let usage = report.facet_usage.get("instructions/impl");
        assert!(usage.is_some());
        assert_eq!(usage.unwrap().len(), 1);
        assert_eq!(usage.unwrap()[0].workflow_name, "test-wf");
    }

    #[test]
    fn diagnose_workflow_name_invalid() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        // ファイル名が不正な文字を含むworkflowを作成
        // load_workflow内のvalidation::validateで名前が拒否されるため、
        // diagnose_allでは「読み込みに失敗」エラーとして報告される
        fs::create_dir_all(wf_dir).unwrap();
        let wf = Workflow {
            name: "bad workflow".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_step("step1", Some("impl"))],
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
    fn diagnose_collect_warning_all_passed() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "review", "content");

        let wf = Workflow {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    ..make_step("review-step", Some("review"))
                },
                NodeDefinition {
                    collect: Some(CollectConfig {
                        from: vec!["review-step".to_string()],
                        reduce: ReduceStrategy::AllPassed,
                    }),
                    ..make_step("collect-step", None)
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Warning && i.message.contains("rulesが未設定")));
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
    fn diagnose_duplicate_step_via_validation() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = Workflow {
            name: "dup-step".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_step("same-name", Some("task")),
                make_step("same-name", Some("task")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.code == "WFS006"
                && i.severity == Severity::Error
                && i.stage == DiagnosticStage::ParseShape
                && i.workflow_name.as_deref() == Some("dup-step")
                && i.field.as_deref() == Some("name")),
            "Expected WFS006 duplicate step error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_node_input_reference_passes() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = Workflow {
            name: "input-ref".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: [(
                "artifact".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                    additional_properties: false,
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![
                NodeDefinition {
                    artifact: Some("artifact".to_string()),
                    ..make_step("step1", Some("task"))
                },
                NodeDefinition {
                    inputs: vec!["step1".to_string()],
                    artifact: Some("artifact".to_string()),
                    ..make_step("step2", Some("task"))
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
    fn diagnose_collect_from_subsequent_step() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        // step1 が後続の step2 を collect.from で参照 → エラーになるべき
        let wf = Workflow {
            name: "subsequent-collect".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    collect: Some(CollectConfig {
                        from: vec!["step2".to_string()],
                        reduce: ReduceStrategy::Concat,
                    }),
                    ..make_step("step1", None)
                },
                make_step("step2", Some("task")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.code == "WFR001"
                && i.severity == Severity::Error
                && i.stage == DiagnosticStage::Resolve
                && i.step_name.as_deref() == Some("step1")
                && i.field.as_deref() == Some("collect.from")),
            "Expected WFR001 for collect.from, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_parallel_child_item_reference_passes() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "{{ item.path }}");

        let wf = Workflow {
            name: "par-item".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![make_fanout(
                "par",
                vec![InterimChild {
                    name: "child1".to_string(),
                    facets: FacetRefs {
                        instruction: Some("task".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
            )],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.step_name.as_deref() == Some("child1")
                && i.field.as_deref() == Some("inputs")),
            "item reference inside parallel child should not be an error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_fanout_inputs_rejected() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let mut fanout = make_fanout(
            "par",
            vec![
                make_child("child1", Some("task")),
                make_child("child2", Some("task")),
            ],
        );
        fanout.inputs = vec!["request".to_string()];
        let wf = Workflow {
            name: "fanout-inputs".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![fanout],
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
