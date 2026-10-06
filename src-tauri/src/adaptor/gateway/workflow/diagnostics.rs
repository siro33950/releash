use crate::adaptor::gateway::workflow::builtin;
use crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain;
use crate::adaptor::gateway::workflow::facet::{self, FacetKind};
use crate::adaptor::gateway::workflow::lua;
use crate::adaptor::gateway::workflow::schema::{NodeKind, Rule, WorkflowDefinitionYaml};
use crate::adaptor::gateway::workflow::span_map::YamlSpanMap;
use crate::adaptor::gateway::workflow::workflow_host::prompt_rendering;
use crate::domain::workflow::validation;
use crate::domain::workflow::validation::{
    InvalidArtifactReferenceKind, InvalidEnvironmentReferenceKind, InvalidRuleKind,
    InvalidSchemaKind,
};
use crate::domain::workflow::SessionPermission;
use crate::usecase::workflow::diagnostic_dto::{
    DiagnosticItem, DiagnosticReport, DiagnosticSpan, DiagnosticStage, DiagnosticSummary,
    FacetUsageEntry, Severity,
};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::path::Path;

const ALL_FACET_KINDS: [FacetKind; 3] = [
    FacetKind::Policy,
    FacetKind::Knowledge,
    FacetKind::Instruction,
];

/// 診断 DTO の構築は、外部世界（YAML parser / validation error）を診断語彙へ写す
/// gateway の責務であり、wire model 側には置かない。
impl DiagnosticSpan {
    pub(crate) fn from_location(location: serde_saphyr::Location) -> Self {
        let line = usize::try_from(location.line()).unwrap_or(usize::MAX);
        let col = usize::try_from(location.column()).unwrap_or(usize::MAX);
        Self {
            source: None,
            start_line: line,
            start_col: col,
            end_line: line,
            end_col: col.saturating_add(1),
        }
    }

    pub(crate) fn from_scan_error(error: &serde_saphyr::granit_parser::ScanError) -> Self {
        let marker = error.marker();
        Self {
            source: None,
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
            node_name: None,
            facet_key: None,
            facet_kind: None,
            field: None,
        }
    }

    pub(crate) fn workflow(mut self, name: impl Into<String>) -> Self {
        self.workflow_name = Some(name.into());
        self
    }

    pub(crate) fn node(mut self, name: impl Into<String>) -> Self {
        self.node_name = Some(name.into());
        self
    }

    pub(crate) fn facet(mut self, key: impl Into<String>, kind: impl Into<String>) -> Self {
        self.facet_key = Some(key.into());
        self.facet_kind = Some(kind.into());
        self
    }

    pub(crate) fn field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticScope {
    /// 適用済み config directory 用。disk + builtin の全 workflow / 全 Facet を列挙する。
    AllAvailable,
    /// 指定 directory 用。directory 配下の workflow を起点に、到達する Facet だけを列挙する。
    ReachableFromDirectory,
}

type LoadedWorkflowDiagnostics =
    Result<(WorkflowDefinitionYaml, Vec<DiagnosticItem>), Vec<DiagnosticItem>>;
type NamedWorkflowDiagnostics = (String, LoadedWorkflowDiagnostics);

#[derive(Debug, Clone)]
pub struct WorkflowSourceDiagnostics {
    pub workflow: Option<WorkflowDefinitionYaml>,
    pub diagnostics: Vec<DiagnosticItem>,
}

impl WorkflowSourceDiagnostics {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|item| item.severity == Severity::Error)
    }
}

pub fn diagnose_workflow_source(
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

pub fn diagnose_lua_workflow_source(
    source_name: &str,
    source: &str,
    workflows_dir: &Path,
    facets_base_dir: &Path,
    workflow_name_hint: Option<&str>,
) -> WorkflowSourceDiagnostics {
    let catalog = match lua::facet_catalog(facets_base_dir) {
        Ok(catalog) => catalog,
        Err(error) => {
            return WorkflowSourceDiagnostics {
                workflow: None,
                diagnostics: vec![DiagnosticItem::new(
                    "WFS010",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    Some(lua_span(source_name, 1)),
                    format!("facet index could not be loaded: {error}"),
                )
                .workflow(workflow_name_hint.unwrap_or("<unknown>"))],
            };
        }
    };
    let loaded = match lua::load_lua_workflow(source_name, source, workflows_dir, catalog) {
        Ok(loaded) => loaded,
        Err(error) => {
            let stage = stage_for_code(&error.code);
            let span = error
                .location
                .as_ref()
                .map(|location| lua_location_span(location, workflows_dir));
            let mut item =
                DiagnosticItem::new(error.code, Severity::Error, stage, span, error.message)
                    .workflow(workflow_name_hint.unwrap_or("<unknown>"));
            if let Some(field) = error.field {
                item = item.field(field);
            }
            return WorkflowSourceDiagnostics {
                workflow: None,
                diagnostics: vec![item],
            };
        }
    };
    let mut diagnostics = diagnose_workflow_definition(&loaded.workflow, None);
    for item in &mut diagnostics {
        let location = item
            .node_name
            .as_ref()
            .and_then(|node| loaded.node_locations.get(node))
            .or_else(|| loaded.node_locations.get("main"));
        if let Some(location) = location {
            item.span = Some(lua_location_span(location, workflows_dir));
            if item.code == "WFS008" && item.field.as_deref() == Some("artifact") {
                if let Some(mut span) = item
                    .node_name
                    .as_ref()
                    .and_then(|node| loaded.node_artifact_spans.get(node))
                    .cloned()
                {
                    span.source = lua_location_span(location, workflows_dir).source;
                    item.span = Some(span);
                }
            }
        }
    }
    if workflow_name_hint.is_some_and(|name| name != loaded.workflow.name) {
        let location = loaded.node_locations.get("main");
        diagnostics.push(
            DiagnosticItem::new(
                "WFS006",
                Severity::Error,
                DiagnosticStage::ParseShape,
                location.map(|location| lua_location_span(location, workflows_dir)),
                format!(
                    "Lua workflow name '{}' must match file name '{}'",
                    loaded.workflow.name,
                    workflow_name_hint.unwrap_or("<unknown>")
                ),
            )
            .workflow(workflow_name_hint.unwrap_or("<unknown>"))
            .field("name"),
        );
    }
    WorkflowSourceDiagnostics {
        workflow: Some(loaded.workflow),
        diagnostics,
    }
}

fn lua_span(source: &str, line: usize) -> DiagnosticSpan {
    DiagnosticSpan {
        source: Some(source.to_string()),
        start_line: line,
        start_col: 1,
        end_line: line,
        end_col: 2,
    }
}

fn lua_location_span(
    location: &crate::infrastructure::lua::LuaSourceLocation,
    workflows_dir: &Path,
) -> DiagnosticSpan {
    let source = Path::new(&location.source);
    let display_source = if source.is_absolute() {
        std::fs::canonicalize(workflows_dir)
            .ok()
            .and_then(|base| source.strip_prefix(base).ok())
            .map(|relative| relative.to_string_lossy().into_owned())
            .unwrap_or_else(|| location.source.clone())
    } else {
        location.source.clone()
    };
    lua_span(&display_source, location.line)
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

    let Some(nodes_value) = root.get("nodes") else {
        return diagnostics;
    };
    let Some(nodes) = nodes_value.as_object() else {
        diagnostics.push(
            DiagnosticItem::new(
                "WFS002",
                Severity::Error,
                DiagnosticStage::ParseShape,
                span_map.field_span("nodes"),
                "nodes must be a mapping of node name to node definition",
            )
            .workflow(workflow_name)
            .field("nodes"),
        );
        return diagnostics;
    };
    // nodes マップの重複キーは serde-saphyr が raw parse 時点で拒否し、
    // deserialize エラー分類（WFS006）として報告される。
    for (node_name, node) in nodes {
        let node_name = node_name.as_str();
        let node_path = format!("nodes.{node_name}");
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
                .node(node_name)
                .field("nodes"),
            );
            continue;
        };
        check_allowed_fields(
            node_obj,
            &node_path,
            &[
                "command",
                "session",
                "fanout",
                "sequence",
                "env",
                "artifact",
                "input",
                "completion",
                "worktree",
                // rules / inputs は下の WFS007（移設案内）で報告する。
                "inputs",
                "rules",
            ],
            span_map,
            workflow_name,
            Some(node_name),
            &mut diagnostics,
        );
        for moved_field in ["rules", "inputs"] {
            if node_obj.contains_key(moved_field) {
                diagnostics.push(
                    DiagnosticItem::new(
                        "WFS007",
                        Severity::Error,
                        DiagnosticStage::ParseShape,
                        span_map.field_span(&format!("{node_path}.{moved_field}")),
                        format!(
                            "node '{node_name}' cannot declare '{moved_field}': wiring moved to the children entries of the owning composite (sequence / fanout)"
                        ),
                    )
                    .workflow(workflow_name)
                    .node(node_name)
                    .field(moved_field),
                );
            }
        }
        let kind_count = ["command", "session", "fanout", "sequence"]
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
                        "node '{node_name}' must contain exactly one kind block: command, session, fanout, or sequence"
                    ),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("kind"),
            );
        }
        check_command_env_shape(
            node_obj,
            &node_path,
            node_obj.contains_key("command"),
            span_map,
            workflow_name,
            node_name,
            &mut diagnostics,
        );
        if node_name == "request" || crate::domain::workflow::is_reserved_node_name(node_name) {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFR004",
                    Severity::Error,
                    DiagnosticStage::Resolve,
                    span_map.field_span(&node_path),
                    format!("node name '{node_name}' is reserved"),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("nodes"),
            );
        }
        if validation::validate_name(node_name).is_err() {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS006",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.field_span(&node_path),
                    format!("node name '{node_name}' is not a safe identifier"),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("nodes"),
            );
        }
        check_completion_shape(
            node_obj,
            &node_path,
            span_map,
            workflow_name,
            node_name,
            &mut diagnostics,
        );
        if let Some(input) = node_obj.get("input") {
            if !input.is_array() {
                diagnostics.push(
                    DiagnosticItem::new(
                        "WFS002",
                        Severity::Error,
                        DiagnosticStage::ParseShape,
                        span_map.field_span(&format!("{node_path}.input")),
                        format!("node '{node_name}' input must be a list of parameters"),
                    )
                    .workflow(workflow_name)
                    .node(node_name)
                    .field("input"),
                );
            }
        }
        if let Some(session) = node_obj
            .get("session")
            .and_then(serde_json::Value::as_object)
        {
            let session_path = format!("{node_path}.session");
            check_allowed_fields(
                session,
                &session_path,
                &["provider", "model", "permission", "facets"],
                span_map,
                workflow_name,
                Some(node_name),
                &mut diagnostics,
            );
            check_session_permission(
                session,
                &session_path,
                span_map,
                workflow_name,
                node_name,
                &mut diagnostics,
            );
            if let Some(facets) = session.get("facets").and_then(serde_json::Value::as_object) {
                check_allowed_fields(
                    facets,
                    &format!("{node_path}.session.facets"),
                    &["policy", "knowledge", "instruction"],
                    span_map,
                    workflow_name,
                    Some(node_name),
                    &mut diagnostics,
                );
            }
        }
        check_composite_shape(
            node_obj,
            &node_path,
            span_map,
            workflow_name,
            node_name,
            &mut diagnostics,
        );
    }

    diagnostics
}

/// 合成子（sequence / fanout）ブロックと children エントリ（4形式）の形状を
/// span 付き多エラーで検査する。意味判定（供給元解決・ネスト検出等）は
/// domain の validate_all が担う。
fn check_composite_shape(
    node_obj: &serde_json::Map<String, serde_json::Value>,
    node_path: &str,
    span_map: &YamlSpanMap,
    workflow_name: &str,
    node_name: &str,
    diagnostics: &mut Vec<DiagnosticItem>,
) {
    if let Some(sequence) = node_obj
        .get("sequence")
        .and_then(serde_json::Value::as_object)
    {
        let sequence_path = format!("{node_path}.sequence");
        check_allowed_fields(
            sequence,
            &sequence_path,
            &["entry", "children"],
            span_map,
            workflow_name,
            Some(node_name),
            diagnostics,
        );
        check_children_shape(
            sequence.get("children"),
            &sequence_path,
            span_map,
            workflow_name,
            node_name,
            diagnostics,
        );
    }
    if let Some(fanout) = node_obj
        .get("fanout")
        .and_then(serde_json::Value::as_object)
    {
        let fanout_path = format!("{node_path}.fanout");
        check_allowed_fields(
            fanout,
            &fanout_path,
            &["children", "items"],
            span_map,
            workflow_name,
            Some(node_name),
            diagnostics,
        );
        check_children_shape(
            fanout.get("children"),
            &fanout_path,
            span_map,
            workflow_name,
            node_name,
            diagnostics,
        );
    }
}

const CHILD_ENTRY_BODY_FIELDS: &[&str] = &[
    "command",
    "session",
    "fanout",
    "sequence",
    "env",
    "artifact",
    "input",
    "completion",
    "worktree",
    "inputs",
    "rules",
];

fn check_children_shape(
    children: Option<&serde_json::Value>,
    owner_path: &str,
    span_map: &YamlSpanMap,
    workflow_name: &str,
    node_name: &str,
    diagnostics: &mut Vec<DiagnosticItem>,
) {
    let Some(children) = children else {
        return;
    };
    let Some(elements) = children.as_array() else {
        diagnostics.push(
            DiagnosticItem::new(
                "WFS008",
                Severity::Error,
                DiagnosticStage::ParseShape,
                span_map.field_span(&format!("{owner_path}.children")),
                "children must be a list of entries",
            )
            .workflow(workflow_name)
            .node(node_name)
            .field("children"),
        );
        return;
    };
    for (index, element) in elements.iter().enumerate() {
        let element_path = format!("{owner_path}.children[{index}]");
        if element.is_string() {
            continue;
        }
        let Some(element_obj) = element.as_object() else {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS008",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.nearest_span(&element_path),
                    "children entry must be a node name or a mapping",
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("children"),
            );
            continue;
        };
        if element_obj.is_empty() {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS008",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.nearest_span(&element_path),
                    "children entry must not be empty",
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("children"),
            );
            continue;
        }
        // 4形式の判別はキー集合で行う: 全キーが予約語なら④（無名エントリ）、
        // 非予約語キーがちょうど1つだけならそれが名前（②③）。
        let non_reserved: Vec<&String> = element_obj
            .keys()
            .filter(|key| !crate::domain::workflow::is_reserved_node_name(key))
            .collect();
        if non_reserved.is_empty() {
            // ④ 無名エントリ: 要素マップ全体が本体。
            check_child_body_shape(
                element_obj,
                &element_path,
                span_map,
                workflow_name,
                node_name,
                diagnostics,
            );
            continue;
        }
        let [first_key] = non_reserved.as_slice() else {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS008",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.nearest_span(&element_path),
                    "children entry must be a single named key or an unnamed entry of reserved fields",
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("children"),
            );
            continue;
        };
        let first_key = first_key.as_str();
        // ②③ 名前付きエントリ: 単一の名前キーの値が本体。
        if element_obj.len() != 1 {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS008",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.nearest_span(&element_path),
                    format!("children entry '{first_key}' must be the only key in its mapping"),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("children"),
            );
            continue;
        }
        let entry_path = format!("{element_path}.{first_key}");
        let Some(body) = element_obj
            .get(first_key)
            .and_then(serde_json::Value::as_object)
        else {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS008",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.nearest_span(&entry_path),
                    format!("children entry '{first_key}' must map to an entry body"),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("children"),
            );
            continue;
        };
        check_child_body_shape(
            body,
            &entry_path,
            span_map,
            workflow_name,
            node_name,
            diagnostics,
        );
    }
}

fn check_child_body_shape(
    body: &serde_json::Map<String, serde_json::Value>,
    body_path: &str,
    span_map: &YamlSpanMap,
    workflow_name: &str,
    node_name: &str,
    diagnostics: &mut Vec<DiagnosticItem>,
) {
    check_completion_shape(
        body,
        body_path,
        span_map,
        workflow_name,
        node_name,
        diagnostics,
    );
    check_allowed_fields(
        body,
        body_path,
        CHILD_ENTRY_BODY_FIELDS,
        span_map,
        workflow_name,
        Some(node_name),
        diagnostics,
    );
    check_command_env_shape(
        body,
        body_path,
        body.contains_key("command"),
        span_map,
        workflow_name,
        node_name,
        diagnostics,
    );
    if let Some(rules) = body.get("rules").and_then(serde_json::Value::as_array) {
        check_rules_shape(
            rules,
            body_path,
            span_map,
            workflow_name,
            node_name,
            diagnostics,
        );
    }
    if let Some(inputs) = body.get("inputs") {
        if !inputs.is_object() {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS008",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.field_span(&format!("{body_path}.inputs")),
                    "inputs must be a mapping of parameter name to source",
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("inputs"),
            );
        }
    }
    if let Some(session) = body.get("session").and_then(serde_json::Value::as_object) {
        let session_path = format!("{body_path}.session");
        check_session_permission(
            session,
            &session_path,
            span_map,
            workflow_name,
            node_name,
            diagnostics,
        );
    }
    // インライン宣言（③④）の合成子はネストした children も形状検査する。
    check_composite_shape(
        body,
        body_path,
        span_map,
        workflow_name,
        node_name,
        diagnostics,
    );
}

fn check_completion_shape(
    body: &serde_json::Map<String, serde_json::Value>,
    body_path: &str,
    span_map: &YamlSpanMap,
    workflow_name: &str,
    node_name: &str,
    diagnostics: &mut Vec<DiagnosticItem>,
) {
    let Some(completion) = body.get("completion") else {
        return;
    };
    if let Err(error) = super::completion_wire::parse_completion(completion) {
        diagnostics.push(
            DiagnosticItem::new(
                "WFS002",
                Severity::Error,
                DiagnosticStage::ParseShape,
                span_map.field_span(&format!("{body_path}.completion")),
                error.to_string(),
            )
            .workflow(workflow_name)
            .node(node_name)
            .field("completion"),
        );
    }
}

fn check_session_permission(
    session: &serde_json::Map<String, serde_json::Value>,
    session_path: &str,
    span_map: &YamlSpanMap,
    workflow_name: &str,
    node_name: &str,
    diagnostics: &mut Vec<DiagnosticItem>,
) {
    let Some(permission) = session.get("permission") else {
        return;
    };
    let invalid = permission
        .as_str()
        .ok_or_else(|| "field 'permission' must be string".to_string())
        .and_then(|value| {
            value
                .parse::<SessionPermission>()
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .err();
    if let Some(message) = invalid {
        diagnostics.push(
            DiagnosticItem::new(
                "WFS002",
                Severity::Error,
                DiagnosticStage::ParseShape,
                span_map.field_span(&format!("{session_path}.permission")),
                message,
            )
            .workflow(workflow_name)
            .node(node_name)
            .field("permission"),
        );
    }
}

fn check_command_env_shape(
    node: &serde_json::Map<String, serde_json::Value>,
    node_path: &str,
    is_command: bool,
    span_map: &YamlSpanMap,
    workflow_name: &str,
    node_name: &str,
    diagnostics: &mut Vec<DiagnosticItem>,
) {
    let Some(env) = node.get("env") else {
        return;
    };
    let env_path = format!("{node_path}.env");
    if !is_command {
        diagnostics.push(
            DiagnosticItem::new(
                "WFS002",
                Severity::Error,
                DiagnosticStage::ParseShape,
                span_map.field_span(&env_path),
                "env can only be declared by command nodes",
            )
            .workflow(workflow_name)
            .node(node_name)
            .field("env"),
        );
        return;
    }
    let Some(entries) = env.as_object() else {
        diagnostics.push(
            DiagnosticItem::new(
                "WFS002",
                Severity::Error,
                DiagnosticStage::ParseShape,
                span_map.field_span(&env_path),
                "command env must be a mapping of environment variable names to input parameter references",
            )
            .workflow(workflow_name)
            .node(node_name)
            .field("env"),
        );
        return;
    };
    for (name, reference) in entries {
        let entry_path = format!("{env_path}.{name}");
        if let Err(error) = crate::domain::workflow::EnvironmentVariableName::new(name) {
            let (code, stage) = match &error {
                crate::domain::workflow::EnvironmentVariableNameError::Invalid(_) => {
                    ("WFS006", DiagnosticStage::ParseShape)
                }
                crate::domain::workflow::EnvironmentVariableNameError::Reserved(_) => {
                    ("WFR004", DiagnosticStage::Resolve)
                }
            };
            diagnostics.push(
                DiagnosticItem::new(
                    code,
                    Severity::Error,
                    stage,
                    span_map.field_span(&entry_path),
                    error.to_string(),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("env"),
            );
        }
        let Some(reference) = reference.as_str() else {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS002",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.field_span(&entry_path),
                    format!(
                        "command env '{name}' value must be an input parameter reference string"
                    ),
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("env"),
            );
            continue;
        };
        if let Err(error) = crate::domain::workflow::InputParameterRef::new(reference) {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFR003",
                    Severity::Error,
                    DiagnosticStage::Resolve,
                    span_map.field_span(&entry_path),
                    error,
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("env"),
            );
        }
    }
}

fn check_rules_shape(
    rules: &[serde_json::Value],
    base_path: &str,
    span_map: &YamlSpanMap,
    workflow_name: &str,
    node_name: &str,
    diagnostics: &mut Vec<DiagnosticItem>,
) {
    for (rule_index, rule) in rules.iter().enumerate() {
        let rule_path = format!("{base_path}.rules[{rule_index}]");
        let Some(rule_obj) = rule.as_object() else {
            continue;
        };
        check_allowed_fields(
            rule_obj,
            &rule_path,
            &["when", "switch", "loop_guard", "next"],
            span_map,
            workflow_name,
            Some(node_name),
            diagnostics,
        );
        if let Some(on) = rule_obj.get("when").and_then(|when| when.get("on")) {
            if let Err(error) = super::predicate_wire::parse_predicate(on) {
                diagnostics.push(
                    DiagnosticItem::new(
                        "WFS002",
                        Severity::Error,
                        DiagnosticStage::ParseShape,
                        span_map.nearest_span(&format!("{rule_path}.when.on")),
                        error.to_string(),
                    )
                    .workflow(workflow_name)
                    .node(node_name)
                    .field("rules.when.on"),
                );
            }
        }
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

fn check_allowed_fields(
    map: &serde_json::Map<String, serde_json::Value>,
    path: &str,
    allowed: &[&str],
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
        let mut item = DiagnosticItem::new(
            "WFS002",
            Severity::Error,
            DiagnosticStage::ParseShape,
            span_map.field_span(&field_path),
            format!("unknown workflow field '{key}' is not allowed here"),
        )
        .workflow(workflow_name)
        .field(key);
        if let Some(node_name) = node_name {
            item = item.node(node_name);
        }
        diagnostics.push(item);
    }
}

fn deserialize_error_diagnostic(
    error: &serde_saphyr::Error,
    span_map: &YamlSpanMap,
    workflow_name_hint: Option<&str>,
) -> DiagnosticItem {
    let message = error.to_string();
    let code = if message.contains("duplicate") || message.contains("is duplicated") {
        "WFS006"
    } else if message.contains("children entry") || message.contains("inputs source") {
        "WFS008"
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
        ValidationError::InvalidDelegate { kind, .. } => match kind {
            validation::InvalidDelegateKind::UnsupportedNodeKind => "WFC011",
            validation::InvalidDelegateKind::MissingArtifactContract => "WFT006",
            validation::InvalidDelegateKind::ChildWithoutArtifact => "WFT006",
            validation::InvalidDelegateKind::MaxIterations => "WFC005",
            validation::InvalidDelegateKind::WhenFieldNotBoolean => "WFT001",
        },
        ValidationError::EmptyName
        | ValidationError::InvalidChars { .. }
        | ValidationError::EmptyNodes
        | ValidationError::DuplicateNode { .. }
        | ValidationError::EmptyCommand { .. }
        | ValidationError::TooManyNodes { .. }
        | ValidationError::TooManyFanoutChildren { .. } => "WFS006",
        ValidationError::EmptyChildren { .. }
        | ValidationError::SequenceArtifactDeclaration { .. } => "WFS008",
        ValidationError::MissingEntryNode { .. } => "WFR006",
        ValidationError::ReservedNodeName { .. } => "WFR004",
        ValidationError::UnknownRuleTarget { .. }
        | ValidationError::UnknownChildNode { .. }
        | ValidationError::SequenceEntryNotChild { .. } => "WFR001",
        ValidationError::InvalidFanoutItemsReference { .. } => "WFR003",
        ValidationError::FanoutInputMismatch { .. } => "WFT003",
        ValidationError::ChildReferenceViolation { .. } => "WFC006",
        ValidationError::DuplicateChildReference { .. }
        | ValidationError::RulesOnFanoutChildEntry { .. } => "WFC007",
        ValidationError::CompositeInclusionCycle { .. } => "WFC008",
        ValidationError::InvalidInputWiring(violation) => match violation.kind {
            validation::InputWiringKind::AmbiguousSource => "WFR008",
            _ => "WFR007",
        },
        ValidationError::ReservedInputParameterName { .. } => "WFR008",
        ValidationError::UnknownSchemaRef { .. } => "WFR002",
        ValidationError::InvalidSchemaRef { .. } => "WFR002",
        ValidationError::InvalidSchema { kind, .. } => match kind {
            InvalidSchemaKind::UnknownSchemaReference => "WFR002",
            InvalidSchemaKind::InvalidDeclaration => "WFS002",
        },
        ValidationError::InvalidArtifactReference { kind, .. } => match kind {
            InvalidArtifactReferenceKind::ReservedArtifactName => "WFR004",
            InvalidArtifactReferenceKind::UnknownParameter
            | InvalidArtifactReferenceKind::UnknownField
            | InvalidArtifactReferenceKind::InvalidInputRef => "WFR003",
        },
        ValidationError::InvalidEnvironmentReference { kind, .. } => match kind {
            InvalidEnvironmentReferenceKind::UnknownParameter
            | InvalidEnvironmentReferenceKind::UnknownField
            | InvalidEnvironmentReferenceKind::InvalidInputRef => "WFR003",
        },
        ValidationError::InvalidArtifactSchema { .. } => "WFT004",
        ValidationError::ReservedArtifactField { .. } => "WFT005",
        ValidationError::InvalidRules { kind, .. } => match kind {
            InvalidRuleKind::WhenFieldNotBoolean => "WFT001",
            InvalidRuleKind::SwitchFieldNotEnum | InvalidRuleKind::SwitchUnknownCase => "WFT002",
            InvalidRuleKind::DiscriminatorWithoutArtifact => "WFT006",
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
    };
    (code, stage_for_code(code))
}

/// Diagnostic code の接頭辞から段を決める。接頭辞と段の対応はこの関数だけが持つ。
fn stage_for_code(code: &str) -> DiagnosticStage {
    if code.starts_with("WFR") || code.starts_with("WFU") || code.starts_with("FAC") {
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
        ValidationError::InvalidArtifactReference { .. } => span_map.field_span("nodes"),
        ValidationError::InvalidRules { node, kind, .. } => {
            entry_rule_span(wf, node, span_map, invalid_rule_suffix(wf, node, *kind))
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

/// children エントリ（entry_name はエントリの参照名）の YAML パス。
/// base = `nodes.<合成子>.<kind>.children[<index>]`。名前付き（②③）は
/// `<base>.<名前>` が本体のパスになる。
fn child_entry_base_paths(wf: &WorkflowDefinitionYaml, entry_name: &str) -> Vec<String> {
    let mut bases = Vec::new();
    for node in &wf.nodes {
        let (kind_key, children) = match &node.kind {
            NodeKind::Sequence(sequence) => ("sequence", &sequence.children),
            NodeKind::Fanout(fanout) => ("fanout", &fanout.children),
            _ => continue,
        };
        for (index, entry) in children.iter().enumerate() {
            if entry.name == entry_name {
                let element = format!("nodes.{}.{kind_key}.children[{index}]", node.name);
                bases.push(format!("{element}.{entry_name}"));
                bases.push(element);
            }
        }
    }
    bases
}

fn entry_rule_span(
    wf: &WorkflowDefinitionYaml,
    entry_name: &str,
    span_map: &YamlSpanMap,
    suffix: Option<String>,
) -> Option<DiagnosticSpan> {
    let bases = child_entry_base_paths(wf, entry_name);
    if let Some(suffix) = &suffix {
        for base in &bases {
            if let Some(span) = span_map.field_span(&format!("{base}.{suffix}")) {
                return Some(span);
            }
        }
    }
    for base in &bases {
        if let Some(span) = span_map.field_span(&format!("{base}.rules")) {
            return Some(span);
        }
    }
    bases
        .first()
        .and_then(|base| span_map.nearest_span(base))
        .or_else(|| node_base_path(wf, entry_name).and_then(|path| span_map.nearest_span(&path)))
}

fn invalid_rule_suffix(
    wf: &WorkflowDefinitionYaml,
    entry_name: &str,
    kind: InvalidRuleKind,
) -> Option<String> {
    match kind {
        InvalidRuleKind::WhenFieldNotBoolean => {
            entry_rule_index(wf, entry_name, |rule| matches!(rule, Rule::When { .. }))
                .map(|index| format!("rules[{index}].when.on"))
        }
        InvalidRuleKind::SwitchFieldNotEnum
        | InvalidRuleKind::SwitchUnknownCase
        | InvalidRuleKind::SwitchMissingCases => {
            entry_rule_index(wf, entry_name, |rule| matches!(rule, Rule::Switch { .. }))
                .map(|index| format!("rules[{index}].switch.on"))
        }
        InvalidRuleKind::SwitchExhaustiveHasNext | InvalidRuleKind::SwitchRequiresNext => {
            entry_rule_index(wf, entry_name, |rule| matches!(rule, Rule::Switch { .. }))
                .map(|index| format!("rules[{index}].next"))
        }
        InvalidRuleKind::LoopGuardMaxIterations => entry_rule_index(wf, entry_name, |rule| {
            matches!(rule, Rule::LoopGuard { .. })
        })
        .map(|index| format!("rules[{index}].loop_guard.max_iterations")),
        InvalidRuleKind::DiscriminatorWithoutArtifact => entry_rule_index(wf, entry_name, |rule| {
            matches!(rule, Rule::When { .. } | Rule::Switch { .. })
        })
        .map(|index| format!("rules[{index}]")),
        InvalidRuleKind::MultipleDiscriminators
        | InvalidRuleKind::MultipleLoopGuards
        | InvalidRuleKind::MultipleNextCatchAll
        | InvalidRuleKind::StandaloneNextWithDiscriminator
        | InvalidRuleKind::CycleWithoutLoopGuard => Some("rules".to_string()),
    }
}

fn entry_rule_index(
    wf: &WorkflowDefinitionYaml,
    entry_name: &str,
    matches_rule: impl Fn(&Rule) -> bool,
) -> Option<usize> {
    for node in &wf.nodes {
        let children = match &node.kind {
            NodeKind::Sequence(sequence) => &sequence.children,
            NodeKind::Fanout(fanout) => &fanout.children,
            _ => continue,
        };
        for entry in children {
            if entry.name != entry_name {
                continue;
            }
            if let Some(index) = entry
                .rules
                .as_ref()
                .and_then(|rules| rules.iter().position(&matches_rule))
            {
                return Some(index);
            }
        }
    }
    None
}

fn node_base_path(wf: &WorkflowDefinitionYaml, node_name: &str) -> Option<String> {
    wf.nodes
        .iter()
        .find(|node| node.name == node_name)
        .map(|node| format!("nodes.{}", node.name))
}

fn node_field_path(wf: &WorkflowDefinitionYaml, node_name: &str, field: &str) -> Option<String> {
    let base = node_base_path(wf, node_name)?;
    let suffix = match field {
        "provider" => format!("session.{field}"),
        "facets" => "session.facets".to_string(),
        "rules.next" => "rules".to_string(),
        field => field.to_string(),
    };
    Some(format!("{base}.{suffix}"))
}

/// 全ワークフロー・全ファセットを走査し診断結果を返す
pub fn diagnose_all(
    workflows_dir: &Path,
    facets_base_dir: &Path,
) -> Result<DiagnosticReport, super::storage::StorageError> {
    diagnose_with_scope(
        workflows_dir,
        facets_base_dir,
        DiagnosticScope::AllAvailable,
    )
}

/// 指定 directory を workflow source directory として扱い、正本 layout から解決した
/// Facet base に対して workflow から到達する範囲だけを診断する。
pub fn diagnose_directory(dir: &Path) -> Result<DiagnosticReport, super::storage::StorageError> {
    let facets_base_dir = facet::resolve_facets_base_dir(dir);
    diagnose_with_scope(
        dir,
        &facets_base_dir,
        DiagnosticScope::ReachableFromDirectory,
    )
}

fn diagnose_with_scope(
    workflows_dir: &Path,
    facets_base_dir: &Path,
    scope: DiagnosticScope,
) -> Result<DiagnosticReport, super::storage::StorageError> {
    let mut items = Vec::new();
    let mut workflow_summaries: HashMap<String, DiagnosticSummary> = HashMap::new();
    let mut facet_summaries: HashMap<String, DiagnosticSummary> = HashMap::new();
    let mut facet_usage: HashMap<String, Vec<FacetUsageEntry>> = HashMap::new();

    let workflows = load_workflows_in_scope(workflows_dir, facets_base_dir, scope)?;

    // --- 全ファセットキーのセットを構築（参照存在チェック用） ---
    let all_facet_keys = match scope {
        DiagnosticScope::AllAvailable => collect_all_facet_keys(facets_base_dir)?,
        DiagnosticScope::ReachableFromDirectory => workflows
            .iter()
            .filter_map(|(_, result)| result.as_ref().ok())
            .flat_map(|(workflow, _)| collect_reachable_facet_keys(workflow, facets_base_dir))
            .collect(),
    };

    // --- ワークフロー診断 ---
    for (name, wf_result) in &workflows {
        workflow_summaries.entry(name.clone()).or_default();
        match wf_result {
            Err(diagnostics) => {
                for item in diagnostics {
                    add_diagnostic(
                        &mut items,
                        &mut workflow_summaries,
                        name,
                        item.clone().workflow(name.clone()),
                    );
                }
            }
            Ok((wf, source_diagnostics)) => {
                for item in source_diagnostics {
                    add_diagnostic(
                        &mut items,
                        &mut workflow_summaries,
                        name,
                        item.clone().workflow(name.clone()),
                    );
                }
                diagnose_workflow(
                    wf,
                    name,
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
        let summaries = facet::list_facet_summaries(*kind, facets_base_dir)?;
        for summary in &summaries {
            let facet_id = format!("{}/{}", kind.canonical_name(), summary.key);
            if scope == DiagnosticScope::ReachableFromDirectory {
                if !all_facet_keys.contains(&facet_id) {
                    continue;
                }
                // 到達した Facet は diagnostic が 0 件でも判定対象だったことを summary に残す。
                facet_summaries.entry(facet_id.clone()).or_default();
            }

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
                continue;
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
            {
                let content = facet::load_facet(*kind, &summary.key, facets_base_dir)?;
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

    Ok(DiagnosticReport {
        items,
        workflow_summaries,
        facet_summaries,
        facet_usage,
    })
}

/// 全ファセットキーを収集（"kind/key" 形式）
pub fn collect_all_facet_keys(
    base_dir: &Path,
) -> Result<HashSet<String>, super::storage::StorageError> {
    let mut keys = HashSet::new();
    for kind in &ALL_FACET_KINDS {
        for key in facet::list_facets(*kind, base_dir)? {
            keys.insert(format!("{}/{}", kind.canonical_name(), key));
        }
    }
    Ok(keys)
}

fn collect_referenced_facet_keys(
    workflow: &WorkflowDefinitionYaml,
    base_dir: &Path,
) -> Result<HashSet<String>, facet::FacetError> {
    let mut checked = HashSet::new();
    let mut existing = HashSet::new();
    try_for_each_workflow_facet_ref(workflow, |kind, key| {
        collect_existing_facet_key(&mut checked, &mut existing, kind, key, base_dir)
    })?;
    Ok(existing)
}

fn collect_reachable_facet_keys(
    workflow: &WorkflowDefinitionYaml,
    base_dir: &Path,
) -> HashSet<String> {
    let mut checked = HashSet::new();
    let mut existing = HashSet::new();
    for_each_workflow_facet_ref(workflow, |kind, key| {
        let facet_id = format!("{}/{}", kind.canonical_name(), key);
        if checked.insert(facet_id.clone())
            && matches!(facet::facet_exists(kind, key, base_dir), Ok(true))
        {
            existing.insert(facet_id);
        }
    });
    existing
}

/// 失敗しない走査。`try_for_each_workflow_facet_ref` と同じ順序で参照を訪れる。
fn for_each_workflow_facet_ref(
    workflow: &WorkflowDefinitionYaml,
    mut visit: impl FnMut(FacetKind, &str),
) {
    try_for_each_workflow_facet_ref(workflow, |kind, key| {
        visit(kind, key);
        Ok::<(), Infallible>(())
    })
    .unwrap();
}

fn try_for_each_workflow_facet_ref<E>(
    workflow: &WorkflowDefinitionYaml,
    mut visit: impl FnMut(FacetKind, &str) -> Result<(), E>,
) -> Result<(), E> {
    for node in &workflow.nodes {
        let Some(session) = node.session() else {
            continue;
        };
        if let Some(key) = session.facets.policy.as_deref() {
            visit(FacetKind::Policy, key)?;
        }
        for key in &session.facets.knowledge {
            visit(FacetKind::Knowledge, key)?;
        }
        if let Some(key) = session.facets.instruction.as_deref() {
            visit(FacetKind::Instruction, key)?;
        }
    }
    Ok(())
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
pub fn diagnose_workflow_facet_references(
    workflow: &WorkflowDefinitionYaml,
    facets_base_dir: &Path,
) -> Result<Vec<DiagnosticItem>, facet::FacetError> {
    let all_facet_keys = collect_referenced_facet_keys(workflow, facets_base_dir)?;
    let mut items = Vec::new();
    let mut workflow_summaries = HashMap::new();
    let mut facet_usage = HashMap::new();
    check_workflow_facet_references(
        workflow,
        &workflow.name,
        &all_facet_keys,
        &mut items,
        &mut workflow_summaries,
        &mut facet_usage,
    );
    Ok(items)
}

/// scope に応じたワークフロー一覧を読み込む。
/// `DiagnosticScope::AllAvailable` の場合だけ、ディスク上の定義と名前が衝突しない
/// builtin workflow を追加する。
pub fn load_workflows_in_scope(
    dir: &Path,
    facets_base_dir: &Path,
    scope: DiagnosticScope,
) -> Result<Vec<NamedWorkflowDiagnostics>, super::storage::StorageError> {
    let mut results = Vec::new();

    // ディスク上のカスタムワークフロー（validate() をスキップし全件走査）
    let mut seen = HashSet::new();
    for (name, path) in super::storage::workflow_files(dir)? {
        let result = match std::fs::read_to_string(&path) {
            Ok(content) => {
                let diagnosis =
                    super::storage::diagnose_workflow_file(&path, &content, dir, facets_base_dir);
                if let Some(workflow) = diagnosis.workflow {
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

    if scope == DiagnosticScope::AllAvailable {
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
    }

    Ok(results)
}

/// ValidationError から node 名とフィールド名を抽出
fn validation_error_context(e: &validation::ValidationError) -> (Option<String>, Option<String>) {
    use validation::ValidationError;
    match e {
        ValidationError::InvalidDelegate { node, kind, .. } => {
            let field = kind.field_path();
            (Some(node.clone()), Some(field.into()))
        }
        ValidationError::EmptyName | ValidationError::InvalidChars { .. } => {
            (None, Some("name".to_string()))
        }
        ValidationError::EmptyNodes => (None, Some("nodes".to_string())),
        ValidationError::MissingEntryNode { .. } => (None, Some("nodes".to_string())),
        ValidationError::ReservedNodeName { name } => {
            (Some(name.clone()), Some("nodes".to_string()))
        }
        ValidationError::DuplicateNode { name } => (Some(name.clone()), Some("name".to_string())),
        ValidationError::EmptyChildren { node } => {
            (Some(node.clone()), Some("children".to_string()))
        }
        ValidationError::UnknownChildNode { node, .. }
        | ValidationError::DuplicateChildReference { node, .. }
        | ValidationError::RulesOnFanoutChildEntry { node, .. }
        | ValidationError::ChildReferenceViolation { node, .. }
        | ValidationError::CompositeInclusionCycle { node, .. } => {
            (Some(node.clone()), Some("children".to_string()))
        }
        ValidationError::SequenceEntryNotChild { node, .. } => {
            (Some(node.clone()), Some("sequence.entry".to_string()))
        }
        ValidationError::SequenceArtifactDeclaration { node } => {
            (Some(node.clone()), Some("artifact".to_string()))
        }
        ValidationError::InvalidFanoutItemsReference { node, .. }
        | ValidationError::FanoutInputMismatch { node, .. } => {
            (Some(node.clone()), Some("fanout.items".to_string()))
        }
        ValidationError::InvalidInputWiring(violation) => {
            (Some(violation.node.clone()), Some("inputs".to_string()))
        }
        ValidationError::ReservedInputParameterName { node, .. } => {
            (Some(node.clone()), Some("input".to_string()))
        }
        ValidationError::UnknownRuleTarget { node, .. } => {
            (Some(node.clone()), Some("rules.next".to_string()))
        }
        ValidationError::InvalidRules { node, kind, .. } => (
            Some(node.clone()),
            Some(invalid_rule_field_name(*kind).to_string()),
        ),
        ValidationError::UnreachableNode { node } => {
            (Some(node.clone()), Some("nodes".to_string()))
        }
        ValidationError::MissingFacet { node } => (Some(node.clone()), Some("facets".to_string())),
        ValidationError::InvalidArtifactReference { .. } => (None, Some("nodes".to_string())),
        ValidationError::InvalidEnvironmentReference { node, .. } => {
            (Some(node.clone()), Some("env".to_string()))
        }
        ValidationError::EmptyCommand { node } => (Some(node.clone()), Some("command".to_string())),
        ValidationError::TooManyNodes { .. } => (None, Some("nodes".to_string())),
        ValidationError::TooManyFanoutChildren { node, .. } => {
            (Some(node.clone()), Some("children".to_string()))
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
        InvalidRuleKind::DiscriminatorWithoutArtifact
        | InvalidRuleKind::MultipleDiscriminators
        | InvalidRuleKind::MultipleLoopGuards
        | InvalidRuleKind::StandaloneNextWithDiscriminator
        | InvalidRuleKind::CycleWithoutLoopGuard => "rules",
    }
}

fn diagnose_workflow(
    wf: &WorkflowDefinitionYaml,
    name: &str,
    all_facet_keys: &HashSet<String>,
    items: &mut Vec<DiagnosticItem>,
    workflow_summaries: &mut HashMap<String, DiagnosticSummary>,
    facet_usage: &mut HashMap<String, Vec<FacetUsageEntry>>,
) {
    // ビルトイン info
    if wf.builtin {
        let item = DiagnosticItem::new(
            "WFI000",
            Severity::Info,
            DiagnosticStage::Resolve,
            None,
            format!("ビルトインワークフロー '{name}'"),
        )
        .workflow(name.to_owned());
        add_diagnostic(items, workflow_summaries, name, item);
    }

    check_workflow_facet_references(
        wf,
        name,
        all_facet_keys,
        items,
        workflow_summaries,
        facet_usage,
    );
}

fn check_workflow_facet_references(
    wf: &WorkflowDefinitionYaml,
    name: &str,
    all_facet_keys: &HashSet<String>,
    items: &mut Vec<DiagnosticItem>,
    workflow_summaries: &mut HashMap<String, DiagnosticSummary>,
    facet_usage: &mut HashMap<String, Vec<FacetUsageEntry>>,
) {
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

pub struct FacetRefs<'a> {
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
        let Some(usage_node) = domain_workflow.node_by_name(&usage.node_name) else {
            continue;
        };
        for error in
            validation::validate_template_references_for_node(&domain_workflow, usage_node, content)
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
                    source: None,
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
#[path = "diagnostics_test.rs"]
mod diagnostics_tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "test_helpers_diagnostics.rs"]
pub(crate) mod shared_test_helpers;
