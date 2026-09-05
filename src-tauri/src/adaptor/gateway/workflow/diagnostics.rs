use crate::adaptor::gateway::workflow::builtin;
use crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain;
use crate::adaptor::gateway::workflow::facet::{self, FacetKind};
use crate::adaptor::gateway::workflow::lua;
#[cfg(test)]
use crate::adaptor::gateway::workflow::schema::NodeDefinition;
use crate::adaptor::gateway::workflow::schema::{NodeKind, Rule, WorkflowDefinitionYaml};
use crate::adaptor::gateway::workflow::span_map::YamlSpanMap;
use crate::adaptor::gateway::workflow::workflow_host::prompt_rendering;
use crate::adaptor::protocol::workflow::{
    DiagnosticItem, DiagnosticReport, DiagnosticSpan, DiagnosticStage, DiagnosticSummary,
    FacetUsageEntry, Severity,
};
use crate::domain::workflow::validation;
use crate::domain::workflow::validation::{
    InvalidArtifactReferenceKind, InvalidEnvironmentReferenceKind, InvalidRuleKind,
    InvalidSchemaKind,
};
use crate::domain::workflow::SessionPermission;
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
enum DiagnosticScope {
    /// 適用済み config directory 用。disk + builtin の全 workflow / 全 Facet を列挙する。
    AllAvailable,
    /// 指定 directory 用。directory 配下の workflow を起点に、到達する Facet だけを列挙する。
    ReachableFromDirectory,
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

pub(crate) fn diagnose_lua_workflow_source(
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
            &["entry", "output", "children"],
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
    "on_failure",
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
    if let Some(on_failure) = body.get("on_failure") {
        if !on_failure_shape_is_valid(on_failure) {
            diagnostics.push(
                DiagnosticItem::new(
                    "WFS008",
                    Severity::Error,
                    DiagnosticStage::ParseShape,
                    span_map.field_span(&format!("{body_path}.on_failure")),
                    "on_failure must be `ignore` or a mapping with a single `retry: <n>` (n >= 1)",
                )
                .workflow(workflow_name)
                .node(node_name)
                .field("on_failure"),
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

fn on_failure_shape_is_valid(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => text == "ignore",
        serde_json::Value::Object(map) => {
            map.len() == 1
                && map
                    .get("retry")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|count| (1..=u64::from(u32::MAX)).contains(&count))
        }
        _ => false,
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
        ValidationError::EmptyName
        | ValidationError::InvalidChars { .. }
        | ValidationError::EmptyNodes
        | ValidationError::DuplicateNode { .. }
        | ValidationError::EmptyCommand { .. }
        | ValidationError::TooManyNodes { .. }
        | ValidationError::TooManyFanoutChildren { .. } => "WFS006",
        ValidationError::EmptyChildren { .. }
        | ValidationError::SequenceArtifactRequiresOutput { .. } => "WFS008",
        ValidationError::MissingEntryNode { .. } => "WFR006",
        ValidationError::ReservedNodeName { .. } => "WFR004",
        ValidationError::UnknownRuleTarget { .. }
        | ValidationError::UnknownChildNode { .. }
        | ValidationError::SequenceEntryNotChild { .. }
        | ValidationError::SequenceOutputNotChild { .. } => "WFR001",
        ValidationError::InvalidFanoutItemsReference { .. } => "WFR003",
        ValidationError::FanoutInputMismatch { .. } => "WFT003",
        ValidationError::ChildReferenceViolation { .. } => "WFC006",
        ValidationError::DuplicateChildReference { .. }
        | ValidationError::RulesOnFanoutChildEntry { .. } => "WFC007",
        ValidationError::CompositeInclusionCycle { .. } => "WFC008",
        ValidationError::IgnoredChildDependency { .. } => "WFC009",
        ValidationError::RetryOnCompositeChild { .. } => "WFC010",
        ValidationError::InvalidInputWiring(violation) => match violation.kind {
            validation::InputWiringKind::AmbiguousSource => "WFR008",
            _ => "WFR007",
        },
        ValidationError::ReservedInputParameterName { .. } => "WFR008",
        ValidationError::UnsupportedWorktreeField { .. } => "WFU002",
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
        ValidationError::IgnoredChildDependency { child, .. }
        | ValidationError::RetryOnCompositeChild { child, .. } => {
            let bases = child_entry_base_paths(wf, child);
            bases
                .iter()
                .find_map(|base| span_map.field_span(&format!("{base}.on_failure")))
                .or_else(|| bases.first().and_then(|base| span_map.nearest_span(base)))
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
        InvalidRuleKind::DiscriminatorOnFanout | InvalidRuleKind::DiscriminatorWithoutArtifact => {
            entry_rule_index(wf, entry_name, |rule| {
                matches!(rule, Rule::When { .. } | Rule::Switch { .. })
            })
            .map(|index| format!("rules[{index}]"))
        }
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
pub fn diagnose_all(workflows_dir: &Path, facets_base_dir: &Path) -> DiagnosticReport {
    diagnose_with_scope(
        workflows_dir,
        facets_base_dir,
        DiagnosticScope::AllAvailable,
    )
}

/// 指定 directory を workflow source directory として扱い、正本 layout から解決した
/// Facet base に対して workflow から到達する範囲だけを診断する。
pub fn diagnose_directory(dir: &Path) -> DiagnosticReport {
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
) -> DiagnosticReport {
    let mut items = Vec::new();
    let mut workflow_summaries: HashMap<String, DiagnosticSummary> = HashMap::new();
    let mut facet_summaries: HashMap<String, DiagnosticSummary> = HashMap::new();
    let mut facet_usage: HashMap<String, Vec<FacetUsageEntry>> = HashMap::new();

    let workflows = load_workflows_in_scope(workflows_dir, facets_base_dir, scope);

    // --- 全ファセットキーのセットを構築（参照存在チェック用） ---
    let all_facet_keys = match scope {
        DiagnosticScope::AllAvailable => collect_all_facet_keys(facets_base_dir),
        DiagnosticScope::ReachableFromDirectory => workflows
            .iter()
            .filter_map(|(_, result)| result.as_ref().ok())
            .flat_map(|(workflow, _)| collect_reachable_facet_keys(workflow, facets_base_dir))
            .collect(),
    };

    // --- ワークフロー診断 ---
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

/// scope に応じたワークフロー一覧を読み込む。
/// `DiagnosticScope::AllAvailable` の場合だけ、ディスク上の定義と名前が衝突しない
/// builtin workflow を追加する。
fn load_workflows_in_scope(
    dir: &Path,
    facets_base_dir: &Path,
    scope: DiagnosticScope,
) -> Vec<NamedWorkflowDiagnostics> {
    let mut results = Vec::new();

    // ディスク上のカスタムワークフロー（validate() をスキップし全件走査）
    let mut seen = HashSet::new();
    if dir.exists() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if super::storage::workflow_source_format(&path).is_some() {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        let name = stem.to_string();
                        let result = match std::fs::read_to_string(&path) {
                            Ok(content) => {
                                let diagnosis = super::storage::diagnose_workflow_file(
                                    &path,
                                    &content,
                                    dir,
                                    facets_base_dir,
                                );
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
                }
            }
        }
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
        ValidationError::IgnoredChildDependency { node, .. }
        | ValidationError::RetryOnCompositeChild { node, .. } => {
            (Some(node.clone()), Some("on_failure".to_string()))
        }
        ValidationError::SequenceEntryNotChild { node, .. } => {
            (Some(node.clone()), Some("sequence.entry".to_string()))
        }
        ValidationError::SequenceOutputNotChild { node, .. }
        | ValidationError::SequenceArtifactRequiresOutput { node } => {
            (Some(node.clone()), Some("sequence.output".to_string()))
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
        ValidationError::UnsupportedWorktreeField { node } => {
            (Some(node.clone()), Some("worktree".to_string()))
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
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, FacetRefs, FanoutSpec, ItemsSource, NodeKind, Rule, SchemaDef, SessionSpec,
        WorkflowDefinitionYaml,
    };
    use crate::domain::workflow::InputParam;
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

    fn make_session_node(
        name: &str,
        policy: Option<&str>,
        knowledge: &[&str],
        instruction: Option<&str>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    policy: policy.map(str::to_string),
                    knowledge: knowledge.iter().map(|key| (*key).to_string()).collect(),
                    instruction: instruction.map(str::to_string),
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_child(name: &str, instruction: Option<&str>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
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
                children: children
                    .into_iter()
                    .map(crate::domain::workflow::ChildEntry::reference)
                    .collect(),
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
                env: Default::default(),
            }),
            ..NodeDefinition::default()
        }
    }

    fn setup_facet(dir: &Path, kind: &str, key: &str, content: &str) {
        let facet_dir = dir.join(kind);
        fs::create_dir_all(&facet_dir).unwrap();
        fs::write(facet_dir.join(format!("{key}.md")), content).unwrap();
    }

    #[test]
    fn test_診断reportはserializeとdeserializeをround_tripできる() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("round-trip.yml"),
            r#"name: round-trip
description: round trip
nodes:
  main:
    command: printf ok
"#,
        )
        .unwrap();

        // When
        let report = diagnose_all(tmp.path(), tmp.path());
        let value = serde_json::to_value(report).unwrap();
        let decoded = serde_json::from_value::<DiagnosticReport>(value.clone()).unwrap();

        // Then
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }

    #[test]
    fn test_診断itemは省略されたoptional_fieldをnoneとしてdeserializeする() {
        // Given
        let value = serde_json::json!({
            "code": "X",
            "severity": "error",
            "stage": "parse_shape",
            "message": "m"
        });

        // When
        let item = serde_json::from_value::<DiagnosticItem>(value).unwrap();

        // Then
        assert!(item.span.is_none());
        assert!(item.workflow_name.is_none());
        assert!(item.node_name.is_none());
        assert!(item.facet_key.is_none());
        assert!(item.facet_kind.is_none());
        assert!(item.field.is_none());
    }

    #[test]
    fn test_診断spanは省略されたsourceをnoneとしてdeserializeする() {
        // Given
        let value = serde_json::json!({
            "start_line": 7,
            "start_col": 5,
            "end_line": 7,
            "end_col": 6
        });

        // When
        let span = serde_json::from_value::<DiagnosticSpan>(value).unwrap();

        // Then
        assert!(span.source.is_none());
    }

    fn permission_yaml(permission: &str) -> String {
        format!(
            r#"name: permission
description: permission contract
nodes:
  main:
    session:
      provider: claude
      permission: {permission}
      facets:
        instruction: test
"#
        )
    }

    fn permission_lua(permission: &str) -> String {
        format!(
            r#"local r = require("releash")
local f = require("facets")
return r.workflow{{
  name = "permission", description = "permission contract",
  main = r.session{{ provider = r.provider.claude, permission = "{permission}", facets = {{ instruction = f.instruction.test }} }},
}}
"#
        )
    }

    fn permission_lua_value(permission: &str) -> String {
        format!(
            r#"local r = require("releash")
local f = require("facets")
return r.workflow{{
  name = "permission", description = "permission contract",
  main = r.session{{ provider = r.provider.claude, permission = {permission}, facets = {{ instruction = f.instruction.test }} }},
}}
"#
        )
    }

    fn inline_child_permission_yaml(permission: &str) -> String {
        format!(
            r#"name: permission
description: permission contract
nodes:
  main:
    sequence:
      children:
      - session:
          provider: claude
          permission: {permission}
          facets:
            instruction: test
"#
        )
    }

    fn named_inline_child_permission_yaml(permission: &str) -> String {
        format!(
            r#"name: permission
description: permission contract
nodes:
  main:
    sequence:
      children:
      - review:
          session:
            provider: claude
            permission: {permission}
            facets:
              instruction: test
"#
        )
    }

    fn inline_child_permission_lua(permission: &str) -> String {
        format!(
            r#"local r = require("releash")
local f = require("facets")
local child = r.session{{ provider = r.provider.claude, permission = "{permission}", facets = {{ instruction = f.instruction.test }} }}
return r.workflow{{
  name = "permission", description = "permission contract",
  main = r.sequence{{ children = {{ r.child{{ node = child }} }} }},
}}
"#
        )
    }

    #[test]
    fn test_診断_yamlとluaのsession_permission_4値をerrorなしで受理する() {
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "instructions", "test", "test instruction");

        for permission in ["manual", "auto", "bypass", "read-only"] {
            let yaml = diagnose_workflow_source(&permission_yaml(permission), Some("permission"));
            assert!(
                !yaml.has_errors(),
                "YAML {permission} produced diagnostics: {:?}",
                yaml.diagnostics
            );
            let lua = diagnose_lua_workflow_source(
                "permission.lua",
                &permission_lua(permission),
                tmp.path(),
                tmp.path(),
                Some("permission"),
            );
            assert!(
                !lua.has_errors(),
                "Lua {permission} produced diagnostics: {:?}",
                lua.diagnostics
            );
        }
    }

    #[test]
    fn test_診断_yamlとluaの不正permissionは同じwfs002でfield位置を示す() {
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "instructions", "test", "test instruction");

        for invalid in ["unknown", "acceptEdits", "danger-full-access"] {
            let yaml = diagnose_workflow_source(&permission_yaml(invalid), Some("permission"));
            let lua = diagnose_lua_workflow_source(
                "permission.lua",
                &permission_lua(invalid),
                tmp.path(),
                tmp.path(),
                Some("permission"),
            );
            let yaml_item = yaml.diagnostics.first().unwrap();
            let lua_item = lua.diagnostics.first().unwrap();

            assert_eq!(yaml_item.code, "WFS002");
            assert_eq!(lua_item.code, yaml_item.code);
            assert_eq!(yaml_item.stage, DiagnosticStage::ParseShape);
            assert_eq!(lua_item.stage, yaml_item.stage);
            assert_eq!(yaml_item.message, lua_item.message);
            assert_eq!(yaml_item.field.as_deref(), Some("permission"));
            assert_eq!(lua_item.field, yaml_item.field);
            assert_eq!(yaml_item.span.as_ref().unwrap().start_line, 7);
            assert_eq!(lua_item.span.as_ref().unwrap().start_line, 5);
            assert!(yaml.workflow.is_none());
            assert!(lua.workflow.is_none());
        }
    }

    #[test]
    fn test_診断_yamlとluaの非文字列permissionは同じwfs002でfield位置を示す() {
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "instructions", "test", "test instruction");

        let yaml = diagnose_workflow_source(&permission_yaml("5"), Some("permission"));
        let lua = diagnose_lua_workflow_source(
            "permission.lua",
            &permission_lua_value("5"),
            tmp.path(),
            tmp.path(),
            Some("permission"),
        );
        let yaml_item = yaml.diagnostics.first().unwrap();
        let lua_item = lua.diagnostics.first().unwrap();

        assert_eq!(yaml_item.code, "WFS002");
        assert_eq!(lua_item.code, yaml_item.code);
        assert_eq!(yaml_item.stage, DiagnosticStage::ParseShape);
        assert_eq!(lua_item.stage, yaml_item.stage);
        assert_eq!(yaml_item.message, "field 'permission' must be string");
        assert_eq!(lua_item.message, yaml_item.message);
        assert_eq!(yaml_item.field.as_deref(), Some("permission"));
        assert_eq!(lua_item.field, yaml_item.field);
        assert!(yaml.workflow.is_none());
        assert!(lua.workflow.is_none());
    }

    #[test]
    fn test_診断_childrenインラインsessionの不正permissionはtop_levelとluaに一致する() {
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "instructions", "test", "test instruction");

        let top_level =
            diagnose_workflow_source(&permission_yaml("acceptEdits"), Some("permission"));
        let inline_child = diagnose_workflow_source(
            &inline_child_permission_yaml("acceptEdits"),
            Some("permission"),
        );
        let named_inline_child = diagnose_workflow_source(
            &named_inline_child_permission_yaml("acceptEdits"),
            Some("permission"),
        );
        let lua = diagnose_lua_workflow_source(
            "permission.lua",
            &inline_child_permission_lua("acceptEdits"),
            tmp.path(),
            tmp.path(),
            Some("permission"),
        );

        let top_level_item = top_level.diagnostics.first().unwrap();
        let inline_child_item = inline_child.diagnostics.first().unwrap();
        let named_inline_child_item = named_inline_child.diagnostics.first().unwrap();
        let lua_item = lua.diagnostics.first().unwrap();
        for item in [inline_child_item, named_inline_child_item, lua_item] {
            assert_eq!(item.code, top_level_item.code);
            assert_eq!(item.stage, top_level_item.stage);
            assert_eq!(item.message, top_level_item.message);
            assert_eq!(item.field, top_level_item.field);
        }
        assert_eq!(inline_child_item.node_name.as_deref(), Some("main"));
        assert_eq!(inline_child_item.span.as_ref().unwrap().start_line, 9);
        assert_eq!(named_inline_child_item.node_name.as_deref(), Some("main"));
        assert_eq!(
            named_inline_child_item.span.as_ref().unwrap().start_line,
            10
        );
        assert_eq!(lua_item.span.as_ref().unwrap().start_line, 3);
        assert!(top_level.workflow.is_none());
        assert!(inline_child.workflow.is_none());
        assert!(named_inline_child.workflow.is_none());
        assert!(lua.workflow.is_none());
    }

    fn save_workflow_yaml(dir: &Path, wf: &WorkflowDefinitionYaml) {
        fs::create_dir_all(dir).unwrap();
        let content = serde_saphyr::to_string(wf).unwrap();
        fs::write(dir.join(format!("{}.yml", wf.name)), content).unwrap();
    }

    #[test]
    fn test_診断_指定directory経路はbuiltin_workflowを列挙しない() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("custom.yml"),
            r#"name: custom
description: custom workflow
nodes:
  main:
    command: printf ok
"#,
        )
        .unwrap();
        let builtin_names: HashSet<_> = builtin::list_builtin_workflows()
            .into_iter()
            .map(|summary| summary.name)
            .collect();

        // When
        let directory_report = diagnose_directory(tmp.path());
        let all_report = diagnose_all(tmp.path(), tmp.path());

        // Then
        assert!(directory_report
            .workflow_summaries
            .keys()
            .all(|name| !builtin_names.contains(name)));
        assert!(directory_report.items.iter().all(|item| item
            .workflow_name
            .as_ref()
            .is_none_or(|name| !builtin_names.contains(name))));

        assert!(all_report
            .workflow_summaries
            .keys()
            .any(|name| builtin_names.contains(name)));
        assert!(all_report.items.iter().any(|item| item
            .workflow_name
            .as_ref()
            .is_some_and(|name| builtin_names.contains(name))));
    }

    #[test]
    fn test_診断_指定directory経路は参照されないfacetを列挙しない() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "policies", "unused", "unused policy");
        fs::write(
            tmp.path().join("custom.yml"),
            r#"name: custom
description: custom workflow
nodes:
  main:
    command: printf ok
"#,
        )
        .unwrap();

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(!report.facet_summaries.contains_key("policy/unused"));
        assert!(report
            .items
            .iter()
            .all(|item| item.facet_key.as_deref() != Some("unused")));
    }

    #[test]
    fn test_診断_指定directory経路はfacets配下の参照済みcustom_facetを判定対象に含める() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            "custom-instruction",
            "custom instruction",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "canonical-custom-facet".to_string(),
            description: "canonical custom facet diagnostic".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("custom-instruction"))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(!report.items.iter().any(|item| {
            item.code == "FAC002" && item.facet_key.as_deref() == Some("custom-instruction")
        }));
        assert!(report
            .facet_summaries
            .contains_key("instruction/custom-instruction"));
        assert!(report
            .facet_usage
            .contains_key("instruction/custom-instruction"));
    }

    #[test]
    fn test_診断_指定directory経路はfacets配下のdisk本文をbuiltinより優先する() {
        // Given
        let tmp = TempDir::new().unwrap();
        let builtin_instruction =
            builtin::list_builtin_facet_keys(FacetKind::Instruction)[0].to_string();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            &builtin_instruction,
            "{{ bad ref }}",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "builtin-override".to_string(),
            description: "disk facet overrides builtin".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some(&builtin_instruction))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some(&builtin_instruction)
        }));
    }

    #[test]
    fn test_診断_指定directory経路は不正参照があっても他のcustomとbuiltin_facetを保持する() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            "custom-instruction",
            "{{ bad ref }}",
        );
        let builtin_knowledge =
            builtin::list_builtin_facet_keys(FacetKind::Knowledge)[0].to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "mixed-references".to_string(),
            description: "mixed valid and invalid references".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                Some("Bad.Key"),
                &[&builtin_knowledge],
                Some("custom-instruction"),
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(report
            .items
            .iter()
            .any(|item| { item.code == "FAC002" && item.facet_key.as_deref() == Some("Bad.Key") }));
        for valid_key in [&builtin_knowledge, "custom-instruction"] {
            assert!(!report.items.iter().any(|item| {
                item.code == "FAC002" && item.facet_key.as_deref() == Some(valid_key)
            }));
        }
        assert!(report
            .facet_summaries
            .contains_key("instruction/custom-instruction"));
        assert!(report
            .facet_usage
            .contains_key("instruction/custom-instruction"));
        assert!(report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some("custom-instruction")
        }));
        assert!(report.items.iter().any(|item| {
            item.code == "FAC000" && item.facet_key.as_deref() == Some(&builtin_knowledge)
        }));
    }

    #[test]
    fn test_診断_指定directory経路は一部inventoryのio失敗時も健全なkindを保持する() {
        // Given
        let tmp = TempDir::new().unwrap();
        let facets_dir = tmp.path().join("facets");
        fs::create_dir_all(&facets_dir).unwrap();
        fs::write(facets_dir.join("policies"), "not a directory").unwrap();
        setup_facet(&facets_dir, "knowledge", "known", "{{ bad ref }}");
        let workflow = WorkflowDefinitionYaml {
            name: "broken-inventory".to_string(),
            description: "one broken facet inventory".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                Some("custom-policy"),
                &["known"],
                None,
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(!report
            .items
            .iter()
            .any(|item| { item.code == "FAC002" && item.facet_key.as_deref() == Some("known") }));
        assert!(report.facet_summaries.contains_key("knowledge/known"));
        assert!(report.facet_usage.contains_key("knowledge/known"));
        assert!(report
            .items
            .iter()
            .any(|item| { item.code == "FAC003" && item.facet_key.as_deref() == Some("known") }));
    }

    #[test]
    fn test_診断_指定directory経路は不正keyと同居するknowledge_facetを保持する() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "knowledge", "team-guide", "{{ bad ref }}");
        let workflow = WorkflowDefinitionYaml {
            name: "mixed-knowledge".to_string(),
            description: "valid knowledge and invalid instruction".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                None,
                &["team-guide"],
                Some("setup.v2"),
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        let missing_refs = report
            .items
            .iter()
            .filter(|item| item.code == "FAC002")
            .collect::<Vec<_>>();
        assert_eq!(missing_refs.len(), 1);
        assert_eq!(missing_refs[0].facet_key.as_deref(), Some("setup.v2"));
        assert!(report.facet_summaries.contains_key("knowledge/team-guide"));
        assert!(report.facet_usage.contains_key("knowledge/team-guide"));
        assert!(report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some("team-guide")
        }));
    }

    #[test]
    fn test_診断_指定directory経路のfac002集合はall_available経路と一致する() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            tmp.path(),
            "instructions",
            "real-instruction",
            "{{ bad ref }}",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "scope-parity".to_string(),
            description: "facet reference parity".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                Some("-bad"),
                &[],
                Some("real-instruction"),
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let directory_report = diagnose_directory(tmp.path());
        let all_available_report = diagnose_all(tmp.path(), tmp.path());
        let fac002_keys = |report: &DiagnosticReport| {
            report
                .items
                .iter()
                .filter(|item| item.code == "FAC002")
                .filter_map(|item| item.facet_key.clone())
                .collect::<HashSet<_>>()
        };

        // Then
        assert_eq!(
            fac002_keys(&directory_report),
            fac002_keys(&all_available_report)
        );
        assert_eq!(
            fac002_keys(&directory_report),
            HashSet::from(["-bad".to_string()])
        );
        assert!(directory_report
            .facet_summaries
            .contains_key("instruction/real-instruction"));
        assert!(directory_report
            .facet_usage
            .contains_key("instruction/real-instruction"));
        assert!(directory_report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some("real-instruction")
        }));
    }

    #[test]
    fn test_診断_指定directory経路は直下inventory破損時も他kindの本文診断を保持する() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("policies"), "not a directory").unwrap();
        setup_facet(tmp.path(), "knowledge", "known", "{{ bad ref }}");
        let workflow = WorkflowDefinitionYaml {
            name: "cross-kind-degrade".to_string(),
            description: "broken policy inventory and healthy knowledge".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                Some("custom-policy"),
                &["known"],
                None,
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(!report
            .items
            .iter()
            .any(|item| { item.code == "FAC002" && item.facet_key.as_deref() == Some("known") }));
        assert!(report.facet_summaries.contains_key("knowledge/known"));
        assert!(report.facet_usage.contains_key("knowledge/known"));
        assert!(report
            .items
            .iter()
            .any(|item| { item.code == "FAC003" && item.facet_key.as_deref() == Some("known") }));
    }

    #[test]
    fn test_診断_指定directory経路で実体のない参照facetをfac002にする() {
        // Given
        let tmp = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "missing-facet".to_string(),
            description: "missing facet reference".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node("main", Some("missing-policy"), &[], None)],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());
        let item = report
            .items
            .iter()
            .find(|item| {
                item.code == "FAC002" && item.facet_key.as_deref() == Some("missing-policy")
            })
            .expect("missing Facet reference must produce FAC002");

        // Then
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(item.stage, DiagnosticStage::Resolve);
        assert_eq!(item.workflow_name.as_deref(), Some("missing-facet"));
        assert_eq!(item.node_name.as_deref(), Some("main"));
        assert_eq!(item.facet_key.as_deref(), Some("missing-policy"));
        assert_eq!(item.facet_kind.as_deref(), Some("policy"));
        assert_eq!(item.field.as_deref(), Some("policy"));
        assert!(item.message.contains("missing-policy"));
        let usage = report.facet_usage.get("policy/missing-policy").unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].workflow_name, "missing-facet");
        assert_eq!(usage[0].node_name, "main");
        assert_eq!(usage[0].slot, "policy");
    }

    #[test]
    fn test_診断_指定directory経路はfacet本文の不正template構文をfac003にする() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            "invalid-template",
            "{{ bad ref }}",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "facet-syntax".to_string(),
            description: "invalid Facet template syntax".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("invalid-template"))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some("invalid-template")
        }));
        assert!(
            report
                .facet_summaries
                .get("instruction/invalid-template")
                .unwrap()
                .error_count
                > 0
        );
    }

    #[test]
    fn test_診断_指定directory経路はworkflow文脈と不整合なfacet本文をwfr003にする() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            "invalid-reference",
            "Use {{ missing_node }}",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "facet-reference".to_string(),
            description: "invalid Facet template reference".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("invalid-reference"))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(report.items.iter().any(|item| {
            item.code == "WFR003"
                && item.workflow_name.as_deref() == Some("facet-reference")
                && item.node_name.as_deref() == Some("main")
                && item.facet_key.as_deref() == Some("invalid-reference")
        }));
        assert!(
            report
                .facet_summaries
                .get("instruction/invalid-reference")
                .unwrap()
                .error_count
                > 0
        );
    }

    #[test]
    fn test_診断_指定directory経路は直下の参照済みcustom_facetを判定対象に含める() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            tmp.path(),
            "instructions",
            "custom-instruction",
            "custom instruction",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "custom-facet".to_string(),
            description: "custom facet diagnostic".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("custom-instruction"))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(!report.items.iter().any(|item| {
            item.code == "FAC002" && item.facet_key.as_deref() == Some("custom-instruction")
        }));
        assert!(report
            .facet_summaries
            .contains_key("instruction/custom-instruction"));
        assert!(report
            .facet_usage
            .contains_key("instruction/custom-instruction"));
    }

    #[test]
    fn test_診断_指定directory経路は参照済みbuiltin_facetを判定対象に含める() {
        // Given
        let tmp = TempDir::new().unwrap();
        let builtin_instruction =
            builtin::list_builtin_facet_keys(FacetKind::Instruction)[0].to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "builtin-facet".to_string(),
            description: "builtin facet diagnostic".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some(&builtin_instruction))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path());

        // Then
        assert!(!report.items.iter().any(|item| {
            item.code == "FAC002" && item.facet_key.as_deref() == Some(&builtin_instruction)
        }));
        assert!(report.items.iter().any(|item| {
            item.code == "FAC000"
                && item.facet_kind.as_deref() == Some("instruction")
                && item.facet_key.as_deref() == Some(&builtin_instruction)
        }));
    }

    #[test]
    fn test_診断_指定directoryで不正permission宣言がwfs002になる() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("permission.yml"),
            permission_yaml("unknown"),
        )
        .unwrap();

        // When
        let report = diagnose_directory(tmp.path());
        let item = report
            .items
            .iter()
            .find(|item| item.code == "WFS002")
            .expect("WFS002 diagnostic");

        // Then
        assert_eq!(item.stage, DiagnosticStage::ParseShape);
        assert_eq!(item.field.as_deref(), Some("permission"));
        assert_eq!(item.span.as_ref().unwrap().start_line, 7);
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
                name: "main".to_string(),
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
                name: "main".to_string(),
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
                name: "main".to_string(),
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

    fn canonical_example_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../workflows/examples/full-cycle-development.yml")
    }

    #[test]
    fn canonical_example_has_zero_diagnostics() {
        let source = fs::read_to_string(canonical_example_path()).unwrap();
        let diagnosis = diagnose_workflow_source(&source, Some("full-cycle-development"));

        assert!(
            diagnosis.diagnostics.is_empty(),
            "canonical example produced diagnostics: {:?}",
            diagnosis.diagnostics
        );
        assert_eq!(
            diagnosis
                .workflow
                .expect("zero diagnostics must yield a workflow")
                .name,
            "full-cycle-development"
        );
    }

    fn expected_stage_for_code(code: &str) -> DiagnosticStage {
        match &code[..3] {
            "WFR" | "WFU" => DiagnosticStage::Resolve,
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
    fn canonical_example_passes_the_real_loader() {
        let source = fs::read_to_string(canonical_example_path()).unwrap();
        let diagnosis = diagnose_workflow_source(&source, Some("full-cycle-development"));
        assert!(
            diagnosis.diagnostics.is_empty(),
            "canonical example produced diagnostics: {:?}",
            diagnosis.diagnostics
        );
        let definition = diagnosis
            .workflow
            .expect("zero diagnostics must yield a workflow");

        let tmp = TempDir::new().unwrap();
        let workflow_path = tmp.path().join("full-cycle-development.yml");
        fs::write(&workflow_path, source).unwrap();
        for node in &definition.nodes {
            let NodeKind::Session(session) = &node.kind else {
                continue;
            };
            if let Some(policy) = &session.facets.policy {
                setup_facet(tmp.path(), "policies", policy, "test policy");
            }
            for knowledge in &session.facets.knowledge {
                setup_facet(tmp.path(), "knowledge", knowledge, "test knowledge");
            }
            if let Some(instruction) = &session.facets.instruction {
                setup_facet(tmp.path(), "instructions", instruction, "test instruction");
            }
        }

        let workflow =
            crate::adaptor::gateway::workflow::storage::load_workflow(&workflow_path, tmp.path())
                .expect("canonical example must pass the real loader");
        assert_eq!(workflow.name, "full-cycle-development");
        assert_eq!(workflow.nodes.len(), definition.nodes.len());
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
    fn workflow_source_diagnosticsは未知fieldとkeywordを拒否する() {
        let cases = [
            (
                "root",
                r#"
name: unknown-root-field
description: unknown root field
future_field: ignored
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: implement
"#,
            ),
            (
                "node",
                r#"
name: unknown-node-field
description: unknown node field
nodes:
  main:
    future_field: ignored
    session:
      provider: claude
      facets:
        instruction: implement
"#,
            ),
            (
                "session",
                r#"
name: unknown-session-field
description: unknown session field
nodes:
  main:
    session:
      provider: claude
      future_field: ignored
      facets:
        instruction: implement
"#,
            ),
            (
                "session.facets",
                r#"
name: unknown-facet-field
description: unknown session facet field
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: implement
        future_field: ignored
"#,
            ),
            (
                "fanout",
                r#"
name: unknown-fanout-field
description: unknown fanout field
nodes:
  main:
    fanout:
      children:
      - worker
      future_field: ignored
  worker:
    session:
      provider: claude
      facets:
        instruction: implement
"#,
            ),
            (
                "rule",
                r#"
name: unknown-rule-field
description: unknown rule field
nodes:
  main:
    sequence:
      children:
      - work:
          rules:
          - next: review
            future_field: ignored
      - review
  work:
    session:
      provider: claude
      facets:
        instruction: implement
  review:
    session:
      provider: claude
      facets:
        instruction: implement
"#,
            ),
            (
                "schemas",
                r#"
name: unknown-schema-keyword
description: unknown schema keyword
schemas:
  review:
    type: object
    future_keyword: ignored
    properties:
      verdict:
        type: boolean
    required:
      - verdict
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: implement
"#,
            ),
        ];

        for (label, source) in cases {
            let known = source
                .lines()
                .filter(|line| !line.contains("future_"))
                .collect::<Vec<_>>()
                .join("\n");
            let known_diagnosis = diagnose_workflow_source(&known, None);
            assert!(
                known_diagnosis.diagnostics.is_empty(),
                "{label} without unknown input must be accepted: {:?}",
                known_diagnosis.diagnostics
            );

            let diagnosis = diagnose_workflow_source(source, None);
            assert!(
                diagnosis
                    .diagnostics
                    .iter()
                    .any(|item| item.code == "WFS002"),
                "unknown input at {label} must be rejected: {:?}",
                diagnosis.diagnostics
            );
            assert!(diagnosis.workflow.is_none(), "{label}");
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
  main:
    command: printf implement
    session:
      provider: claude
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("bad"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        for reference in ["missing_node", "item"] {
            assert!(
                report.items.iter().any(|item| item.code == "WFR003"
                    && item.workflow_name.as_deref() == Some("semantic-template")
                    && item.node_name.as_deref() == Some("main")
                    && item.facet_key.as_deref() == Some("bad")
                    && item.field.as_deref() == Some("content")
                    && item.message.contains(reference)
                    && item.span.is_some()),
                "expected semantic facet diagnostic for '{reference}', got: {:?}",
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
            .field_span("nodes.main.sequence.children[0].judge.rules[0].when.on")
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
                    kind: InvalidArtifactReferenceKind::UnknownParameter,
                    reason: "renamed wording".to_string(),
                },
                "WFR003",
                DiagnosticStage::Resolve,
            ),
            (
                validation::ValidationError::CompositeInclusionCycle {
                    node: "part".to_string(),
                    cycle: "part -> part".to_string(),
                },
                "WFC008",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::UnsupportedWorktreeField {
                    node: "fanout".to_string(),
                },
                "WFU002",
                DiagnosticStage::Resolve,
            ),
            (
                validation::ValidationError::IgnoredChildDependency {
                    node: "main".to_string(),
                    child: "flaky".to_string(),
                    kind: validation::IgnoredChildDependencyKind::RulesEvaluation,
                },
                "WFC009",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::RetryOnCompositeChild {
                    node: "main".to_string(),
                    child: "part".to_string(),
                },
                "WFC010",
                DiagnosticStage::ControlFlow,
            ),
            (
                validation::ValidationError::InvalidInputWiring(Box::new(
                    validation::InputWiringViolation {
                        node: "main".to_string(),
                        child: "consume".to_string(),
                        parameter: "spec".to_string(),
                        source: "ghost".to_string(),
                        kind: validation::InputWiringKind::UnknownSource,
                        reason: "renamed wording".to_string(),
                    },
                )),
                "WFR007",
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
        let span_map = YamlSpanMap::parse("name: sample\nnodes: {}\n").unwrap();
        let cases = [
            ("unknown field `future_field`", "WFS002"),
            ("unknown variant `future_variant`", "WFS002"),
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
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
        assert_eq!(missing.node_name.as_deref(), Some("main"));
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                input: vec![InputParam {
                    name: "item".to_string(),
                    contract: Some("nonexistent-contract".to_string()),
                }],
                ..make_node("main", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.node_name.as_deref() == Some("main")
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                artifact: Some("nonexistent-contract".to_string()),
                ..make_node("main", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.node_name.as_deref() == Some("main")
                && i.field.as_deref() == Some("artifact")),
            "Expected missing-artifact-schema error on main, got: {:?}",
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
            entry: "main".to_string(),
            schemas: [(
                "review-list".to_string(),
                SchemaDef::Array {
                    items: "missing-item".to_string(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![make_node("main", Some("impl"))],
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
            entry: "main".to_string(),
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
                input: vec![InputParam {
                    name: "item".to_string(),
                    contract: Some("input-contract".to_string()),
                }],
                ..make_node("main", Some("impl"))
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![
                make_fanout("main", vec!["child1"]),
                NodeDefinition {
                    input: vec![InputParam {
                        name: "item".to_string(),
                        contract: Some("nonexistent-contract".to_string()),
                    }],
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    kind: NodeKind::Sequence(
                        crate::adaptor::gateway::workflow::schema::SequenceSpec {
                            entry: None,
                            output: None,
                            children: vec![crate::domain::workflow::ChildEntry {
                                on_failure: None,
                                name: "work".to_string(),
                                inputs: Vec::new(),
                                rules: Some(vec![Rule::Next("nonexistent".to_string())]),
                            }],
                        },
                    ),
                    ..make_node("main", None)
                },
                make_node("work", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        let rule_target_errors = report
            .items
            .iter()
            .filter(|i| {
                i.severity == Severity::Error
                    && i.node_name.as_deref() == Some("work")
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

        // main sequence は start → node3 の隣接辺のみ → orphan は到達不能
        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    kind: NodeKind::Sequence(
                        crate::adaptor::gateway::workflow::schema::SequenceSpec {
                            entry: None,
                            output: None,
                            children: vec![
                                crate::domain::workflow::ChildEntry::reference("start"),
                                crate::domain::workflow::ChildEntry::reference("node3"),
                            ],
                        },
                    ),
                    ..make_node("main", None)
                },
                make_node("start", Some("impl")),
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![
                make_node("main", Some("impl")),
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
    fn builtin_workflows_produce_no_diagnostic_errors() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let report = diagnose_all(wf_dir, wf_dir);
        let errors: Vec<_> = report
            .items
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "ビルトインワークフローに診断エラー: {errors:?}"
        );
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
        setup_facet(wf_dir, "instructions", "bad", "Use {{spec..b}} here");
        let wf = WorkflowDefinitionYaml {
            name: "bad-template".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("bad"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.facet_key.as_deref() == Some("bad")
            && i.message.contains("未定義のテンプレート変数 '{{spec..b}}'")));
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_command("main", "cargo build")],
        };
        save_workflow_yaml(wf_dir, &wf);

        let report = diagnose_all(wf_dir, wf_dir);
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.node_name.as_deref() == Some("main")
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_command("main", "   ")],
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("impl"))],
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
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
            assert_eq!(usages[0].node_name, "main");
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
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("impl"))],
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
  main:
    session:
      provider: claude
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
        // Valid YAML but missing required `nodes` field
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
    fn test_診断_nodesマップの重複キーはwfs006で再出現位置を指す() {
        let source = r#"name: dup-node
description: duplicate node key
nodes:
  main:
    command: printf first
  main:
    command: printf second
"#;

        let diagnosis = diagnose_workflow_source(source, Some("dup-node"));
        let item = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFS006")
            .expect("duplicate node key must produce WFS006");
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(item.stage, DiagnosticStage::ParseShape);
        assert_eq!(item.workflow_name.as_deref(), Some("dup-node"));
        let span = item
            .span
            .as_ref()
            .expect("duplicate node key diagnostic must carry a span");
        assert_eq!(
            span.start_line, 6,
            "span must point at the re-occurring node key"
        );
        assert!(diagnosis.workflow.is_none());
    }

    #[test]
    fn test_診断_main不在はwfr006になる() {
        let source = r#"name: no-main
description: nodes without the main root node
nodes:
  prepare:
    command: printf prepare
"#;

        let diagnosis = diagnose_workflow_source(source, None);
        let item = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFR006")
            .expect("nodes without main must produce WFR006");
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(item.stage, DiagnosticStage::Resolve);
        assert_eq!(item.field.as_deref(), Some("nodes"));
        assert!(item.message.contains("main"));
    }

    #[test]
    fn test_診断_予約語node名はwfr004になる() {
        let source = r#"name: reserved-node-name
description: sequence is a reserved node name
nodes:
  main:
    command: printf main
    rules:
      - next: sequence
  sequence:
    command: printf work
"#;

        let diagnosis = diagnose_workflow_source(source, None);
        let item = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFR004")
            .expect("reserved node name must produce WFR004");
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(item.stage, DiagnosticStage::Resolve);
        assert_eq!(item.node_name.as_deref(), Some("sequence"));
        assert!(item.span.is_some());
        assert!(diagnosis.workflow.is_none());
    }

    #[test]
    fn test_診断_旧リスト形式のnodesはwfs002で拒否される() {
        let source = r#"name: nodes-list-form
description: legacy list form nodes are rejected
nodes:
  - name: main
    command: printf done
"#;

        let diagnosis = diagnose_workflow_source(source, None);
        let item = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFS002")
            .expect("list form nodes must produce WFS002");
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(item.stage, DiagnosticStage::ParseShape);
        assert_eq!(item.field.as_deref(), Some("nodes"));
        assert!(item.message.contains("mapping"));
        assert!(diagnosis.workflow.is_none());
    }

    #[test]
    fn test_診断_input単一文字列はwfs002で拒否される() {
        let source = r#"name: input-single-string
description: legacy single string input is rejected
schemas:
  some-contract: string
nodes:
  main:
    command: printf done
    input: some-contract
"#;

        let diagnosis = diagnose_workflow_source(source, None);
        let item = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFS002")
            .expect("single string input must produce WFS002");
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(item.stage, DiagnosticStage::ParseShape);
        assert_eq!(item.node_name.as_deref(), Some("main"));
        assert_eq!(item.field.as_deref(), Some("input"));
        assert!(item.message.contains("list of parameters"));
        assert!(diagnosis.workflow.is_none());
    }

    #[test]
    fn test_診断_トップレベルentryはwfs002で拒否される() {
        let source = r#"name: top-level-entry
description: entry is not part of the workflow YAML surface
entry: main
nodes:
  main:
    command: printf done
"#;

        let diagnosis = diagnose_workflow_source(source, None);
        let item = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFS002")
            .expect("top-level entry must produce WFS002");
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(item.stage, DiagnosticStage::ParseShape);
        assert_eq!(item.field.as_deref(), Some("entry"));
        assert!(diagnosis.workflow.is_none());
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
            entry: "main".to_string(),
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
                    kind: NodeKind::Sequence(
                        crate::adaptor::gateway::workflow::schema::SequenceSpec {
                            entry: None,
                            output: None,
                            children: vec![
                                crate::domain::workflow::ChildEntry::reference("produce"),
                                crate::domain::workflow::ChildEntry {
                                    on_failure: None,
                                    name: "consume".to_string(),
                                    inputs: vec![(
                                        "doc".to_string(),
                                        crate::domain::workflow::value_objects::InputSourceRef::new(
                                            "produce",
                                        ),
                                    )],
                                    rules: None,
                                },
                            ],
                        },
                    ),
                    ..make_node("main", None)
                },
                NodeDefinition {
                    artifact: Some("artifact".to_string()),
                    ..make_node("produce", Some("task"))
                },
                NodeDefinition {
                    input: vec![InputParam {
                        name: "doc".to_string(),
                        contract: None,
                    }],
                    artifact: Some("artifact".to_string()),
                    ..make_node("consume", Some("task"))
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
            "children inputs wiring should not be an error, got: {:?}",
            report.items
        );
    }

    #[test]
    fn diagnose_fanout_child_item_reference_passes() {
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "{{ item.path }}");

        let mut fanout = make_fanout("main", vec!["child1"]);
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
            entry: "main".to_string(),
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
                    input: vec![InputParam {
                        name: "item".to_string(),
                        contract: Some("item-contract".to_string()),
                    }],
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
    fn diagnose_node_level_wiring_is_rejected_with_migration_guidance() {
        let yaml = r#"
name: node-level-wiring
description: test
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: task
    inputs:
      spec: request
    rules:
    - next: main
"#;

        let diagnosis = diagnose_workflow_source(yaml, None);
        for field in ["inputs", "rules"] {
            assert!(
                diagnosis.diagnostics.iter().any(|i| i.code == "WFS007"
                    && i.severity == Severity::Error
                    && i.field.as_deref() == Some(field)),
                "expected WFS007 for node-level '{field}', got: {:?}",
                diagnosis.diagnostics
            );
        }
    }

    #[test]
    fn diagnose_all_reports_lua_syntax_file_and_line() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("broken.lua"),
            "local r = require('releash')\nreturn )",
        )
        .unwrap();

        let report = diagnose_all(tmp.path(), tmp.path());
        let diagnostic = report
            .items
            .iter()
            .find(|item| item.code == "WFS009")
            .expect("Lua syntax diagnostic");
        let span = diagnostic.span.as_ref().expect("Lua source span");

        assert_eq!(span.source.as_deref(), Some("broken.lua"));
        assert_eq!(span.start_line, 2);
    }

    #[test]
    fn lua_and_yaml_domain_errors_keep_the_same_diagnostic_identity() {
        let tmp = TempDir::new().unwrap();
        let yaml = diagnose_workflow_source(
            r#"
name: review
description: Review
nodes:
  main:
    command: ""
"#,
            Some("review"),
        );
        let lua = diagnose_lua_workflow_source(
            "review.lua",
            r#"
local r = require("releash")
return r.workflow{
  name = "review", description = "Review",
  main = r.command{ command = "" },
}
"#,
            tmp.path(),
            tmp.path(),
            Some("review"),
        );
        let yaml_error = yaml
            .diagnostics
            .iter()
            .find(|item| item.severity == Severity::Error)
            .unwrap();
        let lua_error = lua
            .diagnostics
            .iter()
            .find(|item| item.severity == Severity::Error)
            .unwrap();

        assert_eq!(lua_error.code, yaml_error.code);
        assert_eq!(lua_error.stage, yaml_error.stage);
        assert_eq!(lua_error.message, yaml_error.message);
        assert_eq!(
            lua_error.span.as_ref().unwrap().source.as_deref(),
            Some("review.lua")
        );
    }

    #[test]
    fn test_command_env_yamlとluaが同じworkflow_definitionになる() {
        let tmp = TempDir::new().unwrap();
        let yaml = diagnose_workflow_source(
            r#"name: env-equivalence
description: env equivalence
schemas:
  document-contract:
    type: object
    properties:
      body:
        type: object
        properties:
          text: string
        required: [text]
    required:
      - body
nodes:
  main:
    command: printf
    input:
      - document: document-contract
      - context
    env:
      DOC: document
      BODY: document.body.text
      SPEC_DIR: context.spec_dir
"#,
            Some("env-equivalence"),
        );
        let lua = diagnose_lua_workflow_source(
            "env-equivalence.lua",
            r#"local r = require("releash")
local document_contract = r.schema.object{
  name = "document-contract",
  properties = { body = r.schema.object{
    properties = { text = r.schema.string{} }, required = { "text" },
  } },
  required = { "body" },
}
local document = r.input("document", document_contract)
local context = r.input("context")
return r.workflow{
  name = "env-equivalence", description = "env equivalence",
  main = r.command{
    command = "printf",
    input = { document, context },
    env = { DOC = document, BODY = document.body.text, SPEC_DIR = context.spec_dir },
  },
}
"#,
            tmp.path(),
            tmp.path(),
            Some("env-equivalence"),
        );

        assert!(!yaml.has_errors(), "{:?}", yaml.diagnostics);
        assert!(!lua.has_errors(), "{:?}", lua.diagnostics);
        assert_eq!(yaml.workflow, lua.workflow);
    }

    #[test]
    fn test_command_env_yamlとluaが同じ参照error_codeを返す() {
        let tmp = TempDir::new().unwrap();
        let cases = [
            (
                "env-not-map",
                r#"name: env-not-map
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env: document
"#,
                r#"local r = require("releash")
local document = r.input("document")
return r.workflow{
  name = "env-not-map", description = "invalid env",
  main = r.command{ command = "true", input = { document }, env = document },
}
"#,
                "WFS002",
            ),
            (
                "env-value-not-input",
                r#"name: env-value-not-input
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env:
      DOC: 2
"#,
                r#"local r = require("releash")
local document = r.input("document")
return r.workflow{
  name = "env-value-not-input", description = "invalid env",
  main = r.command{ command = "true", input = { document }, env = { DOC = 2 } },
}
"#,
                "WFS002",
            ),
            (
                "unknown-input",
                r#"name: unknown-input
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env:
      DOC: missing
"#,
                r#"local r = require("releash")
local document = r.input("document")
local missing = r.input("missing")
return r.workflow{
  name = "unknown-input", description = "invalid env",
  main = r.command{ command = "true", input = { document }, env = { DOC = missing } },
}
"#,
                "WFR003",
            ),
            (
                "unknown-contract-field",
                r#"name: unknown-contract-field
description: invalid env
schemas:
  document-contract:
    type: object
    properties:
      body: string
    required:
      - body
nodes:
  main:
    command: "true"
    input:
      - document: document-contract
    env:
      DOC: document.missing
"#,
                r#"local r = require("releash")
local document_contract = r.schema.object{
  name = "document-contract",
  properties = { body = r.schema.string{} },
  required = { "body" },
}
local document = r.input("document", document_contract)
return r.workflow{
  name = "unknown-contract-field", description = "invalid env",
  main = r.command{
    command = "true", input = { document }, env = { DOC = document.missing },
  },
}
"#,
                "WFR003",
            ),
            (
                "reserved-env",
                r#"name: reserved-env
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env:
      RELEASH_WORKTREE_PATH: document
"#,
                r#"local r = require("releash")
local document = r.input("document")
return r.workflow{
  name = "reserved-env", description = "invalid env",
  main = r.command{
    command = "true", input = { document },
    env = { RELEASH_WORKTREE_PATH = document },
  },
}
"#,
                "WFR004",
            ),
            (
                "invalid-env-name",
                r#"name: invalid-env-name
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env:
      BAD-NAME: document
"#,
                r#"local r = require("releash")
local document = r.input("document")
return r.workflow{
  name = "invalid-env-name", description = "invalid env",
  main = r.command{
    command = "true", input = { document }, env = { ["BAD-NAME"] = document },
  },
}
"#,
                "WFS006",
            ),
        ];

        for (name, yaml_source, lua_source, expected_code) in cases {
            let yaml = diagnose_workflow_source(yaml_source, Some(name));
            let lua = diagnose_lua_workflow_source(
                &format!("{name}.lua"),
                lua_source,
                tmp.path(),
                tmp.path(),
                Some(name),
            );
            assert!(yaml.has_errors(), "{name}: {:?}", yaml.diagnostics);
            assert!(lua.has_errors(), "{name}: {:?}", lua.diagnostics);
            let expected_stage = expected_stage_for_code(expected_code);
            assert!(
                yaml.diagnostics
                    .iter()
                    .any(|item| item.code == expected_code && item.stage == expected_stage),
                "{name}: {:?}",
                yaml.diagnostics
            );
            assert!(
                lua.diagnostics
                    .iter()
                    .any(|item| item.code == expected_code && item.stage == expected_stage),
                "{name}: {:?}",
                lua.diagnostics
            );
        }
    }

    #[test]
    fn test_command_env_command以外のnodeとenvというnode名を拒否する() {
        let tmp = TempDir::new().unwrap();
        for (kind, body, lua_node) in [
            (
                "session",
                "session:\n      provider: claude\n    env: {}",
                "r.session{ provider = r.provider.claude, env = {} }",
            ),
            (
                "fanout",
                "fanout:\n      children: [worker]\n    env: {}",
                "r.fanout{ children = {}, env = {} }",
            ),
            (
                "sequence",
                "sequence:\n      children: [worker]\n    env: {}",
                "r.sequence{ children = {}, env = {} }",
            ),
        ] {
            let source = format!(
                "name: {kind}-env\ndescription: invalid env\nnodes:\n  main:\n    {body}\n  worker:\n    command: true\n"
            );
            let diagnosis = diagnose_workflow_source(&source, None);
            assert!(
                diagnosis
                    .diagnostics
                    .iter()
                    .any(|item| item.code == "WFS002" && item.field.as_deref() == Some("env")),
                "{kind}: {:?}",
                diagnosis.diagnostics
            );
            assert!(diagnosis.workflow.is_none());

            let lua = diagnose_lua_workflow_source(
                &format!("{kind}-env.lua"),
                &format!(
                    "local r = require(\"releash\")\nreturn r.workflow{{ name = \"{kind}-env\", description = \"invalid env\", main = {lua_node} }}"
                ),
                tmp.path(),
                tmp.path(),
                Some(&format!("{kind}-env")),
            );
            assert!(
                lua.diagnostics.iter().any(|item| item.code == "WFS002"),
                "{kind}: {:?}",
                lua.diagnostics
            );
            assert!(lua.workflow.is_none());
        }

        for reserved_name in ["env", "session"] {
            let workflow_name = format!("{reserved_name}-node-name");
            let node_name = diagnose_workflow_source(
                &format!(
                    "name: {workflow_name}\ndescription: reserved node name\nnodes:\n  main:\n    command: true\n  {reserved_name}:\n    command: true\n"
                ),
                None,
            );
            assert!(node_name.diagnostics.iter().any(|item| {
                item.code == "WFR004" && item.node_name.as_deref() == Some(reserved_name)
            }));
            assert!(node_name.workflow.is_none());

            let lua_node_name = diagnose_lua_workflow_source(
                &format!("{workflow_name}.lua"),
                &format!(
                    "local r = require(\"releash\")\nlocal child = r.command{{ name = \"{reserved_name}\", command = \"true\" }}\nreturn r.workflow{{\n  name = \"{workflow_name}\", description = \"reserved node name\",\n  main = r.sequence{{ children = {{ r.child{{ node = child }} }} }},\n}}\n"
                ),
                tmp.path(),
                tmp.path(),
                Some(&workflow_name),
            );
            assert!(lua_node_name
                .diagnostics
                .iter()
                .any(|item| item.code == "WFR004"));
            assert!(lua_node_name.workflow.is_none());
        }
    }

    #[test]
    fn test_workflow診断_module評価中にhostを呼ぶ定義と他定義を正常に診断する() {
        // Given
        let tmp = TempDir::new().unwrap();
        let module_dir = tmp.path().join("module-host-parts");
        fs::create_dir(&module_dir).unwrap();
        fs::write(
            module_dir.join("nodes.lua"),
            r#"local r = require("releash")
return { main = r.command{ command = "true" } }
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("module-host.lua"),
            r#"local r = require("releash")
local nodes = require("module-host-parts.nodes")
return r.workflow{
  name = "module-host", description = "Module host workflow", main = nodes.main,
}
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("healthy.lua"),
            r#"local r = require("releash")
return r.workflow{
  name = "healthy", description = "Healthy workflow",
  main = r.command{ command = "printf healthy" },
}
"#,
        )
        .unwrap();

        // When
        let loaded = load_workflows_in_scope(
            tmp.path(),
            tmp.path(),
            DiagnosticScope::ReachableFromDirectory,
        );
        let report = diagnose_directory(tmp.path());

        // Then
        for name in ["module-host", "healthy"] {
            let (_, result) = loaded
                .iter()
                .find(|(workflow_name, _)| workflow_name == name)
                .unwrap_or_else(|| panic!("{name} の診断 load 結果"));
            let Ok((workflow, source_diagnostics)) = result else {
                panic!("{name} は診断時に正常 load されるべき: {result:?}");
            };
            assert_eq!(workflow.name, name);
            assert!(source_diagnostics.is_empty(), "{source_diagnostics:?}");
            assert!(!report.items.iter().any(|item| {
                item.workflow_name.as_deref() == Some(name) && item.severity == Severity::Error
            }));
        }

        assert!(!report.items.iter().any(|item| {
            item.workflow_name.as_deref() == Some("module-host")
                && matches!(item.code.as_str(), "WFS009" | "WFS010" | "WFS011")
        }));
    }

    #[test]
    fn test_workflow診断_lua定義のload失敗を位置付きで報告して他定義へ波及させない() {
        // Given
        let tmp = TempDir::new().unwrap();
        let module_dir = tmp.path().join("broken-parts");
        fs::create_dir(&module_dir).unwrap();
        fs::write(
            module_dir.join("nodes.lua"),
            "local r = require(\"releash\")\nreturn r.command{ command = }\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("broken.lua"),
            r#"local r = require("releash")
local nodes = require("broken-parts.nodes")
return r.workflow{
  name = "broken", description = "Broken workflow", main = nodes.main,
}
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("invalid-yaml.yml"),
            r#"name: invalid-yaml
description: Invalid YAML workflow
nodes:
  main:
    artifact: something
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("healthy.lua"),
            r#"local r = require("releash")
return r.workflow{
  name = "healthy", description = "Healthy workflow",
  main = r.command{ command = "printf healthy" },
}
"#,
        )
        .unwrap();

        // When
        let loaded = load_workflows_in_scope(
            tmp.path(),
            tmp.path(),
            DiagnosticScope::ReachableFromDirectory,
        );
        let report = diagnose_directory(tmp.path());

        // Then
        let broken = report
            .items
            .iter()
            .find(|item| item.workflow_name.as_deref() == Some("broken") && item.code == "WFS009")
            .expect("broken module の syntax diagnostic");
        let span = broken.span.as_ref().expect("broken module の失敗位置");
        assert_eq!(span.source.as_deref(), Some("broken-parts/nodes.lua"));
        assert_eq!(span.start_line, 2);

        assert!(report.items.iter().any(|item| {
            item.workflow_name.as_deref() == Some("invalid-yaml")
                && item.severity == Severity::Error
        }));

        let (_, healthy_result) = loaded
            .iter()
            .find(|(name, _)| name == "healthy")
            .expect("healthy の診断 load 結果");
        let Ok((healthy, source_diagnostics)) = healthy_result else {
            panic!("healthy は正常 load されるべき: {healthy_result:?}");
        };
        assert_eq!(healthy.name, "healthy");
        assert!(source_diagnostics.is_empty(), "{source_diagnostics:?}");
        assert!(!report.items.iter().any(|item| {
            item.workflow_name.as_deref() == Some("healthy") && item.severity == Severity::Error
        }));
    }

    #[test]
    fn lua_component_error_uses_workflow_relative_source_and_line() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("component.lua"),
            r#"
local r = require("releash")
return function()
  local child = r.command{ command = "echo" }
  local invalid = child.missing
  return child
end
"#,
        )
        .unwrap();

        let diagnosis = diagnose_lua_workflow_source(
            "review.lua",
            r#"
local component = require("component")
return require("releash").workflow{
  name = "review", description = "Review", main = component(),
}
"#,
            tmp.path(),
            tmp.path(),
            Some("review"),
        );
        let diagnostic = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFR003")
            .expect("component reference diagnostic");
        let span = diagnostic.span.as_ref().expect("component source span");

        assert_eq!(span.source.as_deref(), Some("component.lua"));
        assert_eq!(span.start_line, 5);
    }

    #[test]
    fn test_多段参照診断_yamlとluaでcode_stage_messageが一致する() {
        let tmp = TempDir::new().unwrap();
        let cases = [
            (
                "wiring",
                "WFR007",
                r#"name: wiring
description: wiring
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    sequence:
      children:
        - source
        - target:
            inputs:
              value: source.outer.missing
  source:
    command: source
    artifact: result
  target:
    command: target
    input: [value]
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target", input = { r.input("value") } }
return r.workflow{ name = "wiring", description = "wiring", main = r.sequence{ children = {
  r.child{ node = source },
  r.child{ node = target, inputs = { value = source.outer.missing } },
} } }
"#,
            ),
            (
                "env",
                "WFR003",
                r#"name: env
description: env
schemas:
  document:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    command: target
    input:
      - doc: document
    env:
      VALUE: doc.outer.missing
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local doc = r.input("doc", document)
return r.workflow{ name = "env", description = "env", main = r.command{
  command = "target", input = { doc }, env = { VALUE = doc.outer.missing },
} }
"#,
            ),
            (
                "template",
                "WFR003",
                r#"name: template
description: template
schemas:
  document:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    command: "echo {{ doc.outer.missing }}"
    input:
      - doc: document
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local doc = r.input("doc", document)
return r.workflow{ name = "template", description = "template", main = r.command{
  command = "echo {{ doc.outer.missing }}", input = { doc },
} }
"#,
            ),
            (
                "when",
                "WFT001",
                r#"name: when
description: when
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          flag:
            type: boolean
        required: [flag]
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - when:
                  on: outer.missing
                  then: "yes"
                next: "no"
        - "yes"
        - "no"
  source:
    command: source
    artifact: result
  "yes":
    command: yes
  "no":
    command: no
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { flag = r.schema.boolean() }, required = { "flag" } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local yes = r.command{ name = "yes", command = "yes" }
local no = r.command{ name = "no", command = "no" }
return r.workflow{ name = "when", description = "when", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.when{ on = source.outer.missing, on_true = yes, next = no } } },
  r.child{ node = yes }, r.child{ node = no },
} } }
"#,
            ),
            (
                "switch",
                "WFT002",
                r#"name: switch
description: switch
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          verdict:
            type: string
            enum: [A]
        required: [verdict]
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - switch:
                  on: outer.missing
                  cases:
                    A: target
        - target
  source:
    command: source
    artifact: result
  target:
    command: target
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { verdict = r.schema.string{ enum = { "A" } } }, required = { "verdict" } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target" }
return r.workflow{ name = "switch", description = "switch", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.switch{ on = source.outer.missing, cases = { A = target } } } },
  r.child{ node = target },
} } }
"#,
            ),
            (
                "items",
                "WFR003",
                r#"name: items
description: items
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          values:
            type: array
            items: item
  item: string
nodes:
  main:
    sequence:
      children: [source, spread]
  source:
    command: source
    artifact: result
  spread:
    fanout:
      items: source.outer.missing
      children: [worker]
  worker:
    command: worker
    input: [item]
"#,
                r#"local r = require("releash")
local item = r.schema.string{}
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { values = r.schema.array{ items = item } } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local worker = r.command{ name = "worker", command = "worker", input = { r.input("item") } }
local spread = r.fanout{ name = "spread", items = source.outer.missing, children = { r.child{ node = worker } } }
return r.workflow{ name = "items", description = "items", main = r.sequence{ children = {
  r.child{ node = source }, r.child{ node = spread },
} } }
"#,
            ),
            (
                "wiring-intermediate-missing",
                "WFR007",
                r#"name: wiring-intermediate-missing
description: wiring intermediate missing
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    sequence:
      children:
        - source
        - target:
            inputs:
              value: source.outer.missing.value
  source:
    command: source
    artifact: result
  target:
    command: target
    input: [value]
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target", input = { r.input("value") } }
return r.workflow{ name = "wiring-intermediate-missing", description = "wiring intermediate missing", main = r.sequence{ children = {
  r.child{ node = source },
  r.child{ node = target, inputs = { value = source.outer.missing.value } },
} } }
"#,
            ),
            (
                "when-intermediate-missing",
                "WFT001",
                r#"name: when-intermediate-missing
description: when intermediate missing
schemas:
  result:
    type: object
    properties:
      route:
        type: object
        properties:
          flag:
            type: boolean
        required: [flag]
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - when:
                  on: route.missing.flag
                  then: "yes"
                next: "no"
        - "yes"
        - "no"
  source:
    command: source
    artifact: result
  "yes":
    command: yes
  "no":
    command: no
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  route = r.schema.object{ properties = { flag = r.schema.boolean() }, required = { "flag" } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local yes = r.command{ name = "yes", command = "yes" }
local no = r.command{ name = "no", command = "no" }
return r.workflow{ name = "when-intermediate-missing", description = "when intermediate missing", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.when{ on = source.route.missing.flag, on_true = yes, next = no } } },
  r.child{ node = yes }, r.child{ node = no },
} } }
"#,
            ),
            (
                "switch-intermediate-missing",
                "WFT002",
                r#"name: switch-intermediate-missing
description: switch intermediate missing
schemas:
  result:
    type: object
    properties:
      route:
        type: object
        properties:
          status:
            type: string
            enum: [ready]
        required: [status]
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - switch:
                  on: route.missing.status
                  cases:
                    ready: target
        - target
  source:
    command: source
    artifact: result
  target:
    command: target
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  route = r.schema.object{ properties = { status = r.schema.string{ enum = { "ready" } } }, required = { "status" } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target" }
return r.workflow{ name = "switch-intermediate-missing", description = "switch intermediate missing", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.switch{ on = source.route.missing.status, cases = { ready = target } } } },
  r.child{ node = target },
} } }
"#,
            ),
            (
                "env-intermediate-missing",
                "WFR003",
                r#"name: env-intermediate-missing
description: env intermediate missing
schemas:
  document:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    command: target
    input:
      - doc: document
    env:
      VALUE: doc.outer.missing.value
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local doc = r.input("doc", document)
return r.workflow{ name = "env-intermediate-missing", description = "env intermediate missing", main = r.command{
  command = "target", input = { doc }, env = { VALUE = doc.outer.missing.value },
} }
"#,
            ),
            (
                "template-intermediate-missing",
                "WFR003",
                r#"name: template-intermediate-missing
description: template intermediate missing
schemas:
  document:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    command: "echo {{ doc.outer.missing.value }}"
    input:
      - doc: document
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local doc = r.input("doc", document)
return r.workflow{ name = "template-intermediate-missing", description = "template intermediate missing", main = r.command{
  command = "echo {{ doc.outer.missing.value }}", input = { doc },
} }
"#,
            ),
            (
                "items-intermediate-missing",
                "WFR003",
                r#"name: items-intermediate-missing
description: items intermediate missing
schemas:
  result:
    type: object
    properties:
      batch:
        type: object
        properties:
          values:
            type: array
            items: item
  item: string
nodes:
  main:
    sequence:
      children: [source, spread]
  source:
    command: source
    artifact: result
  spread:
    fanout:
      items: source.batch.missing.values
      children: [worker]
  worker:
    command: worker
    input: [item]
"#,
                r#"local r = require("releash")
local item = r.schema.string{}
local result = r.schema.object{ properties = {
  batch = r.schema.object{ properties = { values = r.schema.array{ items = item } } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local worker = r.command{ name = "worker", command = "worker", input = { r.input("item") } }
local spread = r.fanout{ name = "spread", items = source.batch.missing.values, children = { r.child{ node = worker } } }
return r.workflow{ name = "items-intermediate-missing", description = "items intermediate missing", main = r.sequence{ children = {
  r.child{ node = source }, r.child{ node = spread },
} } }
"#,
            ),
            (
                "wiring-non-object",
                "WFR007",
                r#"name: wiring-non-object
description: wiring non-object
schemas:
  result:
    type: object
    properties:
      outer: string
nodes:
  main:
    sequence:
      children:
        - source
        - target:
            inputs:
              value: source.outer.value
  source:
    command: source
    artifact: result
  target:
    command: target
    input: [value]
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = { outer = r.schema.string{} } }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target", input = { r.input("value") } }
return r.workflow{ name = "wiring-non-object", description = "wiring non-object", main = r.sequence{ children = {
  r.child{ node = source },
  r.child{ node = target, inputs = { value = source.outer.value } },
} } }
"#,
            ),
            (
                "env-non-object",
                "WFR003",
                r#"name: env-non-object
description: env non-object
schemas:
  document:
    type: object
    properties:
      outer: string
nodes:
  main:
    command: target
    input:
      - doc: document
    env:
      VALUE: doc.outer.value
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.string{},
} }
local doc = r.input("doc", document)
return r.workflow{ name = "env-non-object", description = "env non-object", main = r.command{
  command = "target", input = { doc }, env = { VALUE = doc.outer.value },
} }
"#,
            ),
            (
                "template-non-object",
                "WFR003",
                r#"name: template-non-object
description: template non-object
schemas:
  document:
    type: object
    properties:
      outer: string
nodes:
  main:
    command: "echo {{ doc.outer.value }}"
    input:
      - doc: document
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.string{},
} }
local doc = r.input("doc", document)
return r.workflow{ name = "template-non-object", description = "template non-object", main = r.command{
  command = "echo {{ doc.outer.value }}", input = { doc },
} }
"#,
            ),
            (
                "when-non-object",
                "WFT001",
                r#"name: when-non-object
description: when non-object
schemas:
  result:
    type: object
    properties:
      outer: string
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - when:
                  on: outer.flag
                  then: "yes"
                next: "no"
        - "yes"
        - "no"
  source:
    command: source
    artifact: result
  "yes":
    command: yes
  "no":
    command: no
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = { outer = r.schema.string{} } }
local source = r.command{ name = "source", command = "source", artifact = result }
local yes = r.command{ name = "yes", command = "yes" }
local no = r.command{ name = "no", command = "no" }
return r.workflow{ name = "when-non-object", description = "when non-object", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.when{ on = source.outer.flag, on_true = yes, next = no } } },
  r.child{ node = yes }, r.child{ node = no },
} } }
"#,
            ),
            (
                "switch-non-object",
                "WFT002",
                r#"name: switch-non-object
description: switch non-object
schemas:
  result:
    type: object
    properties:
      outer: string
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - switch:
                  on: outer.verdict
                  cases:
                    A: target
        - target
  source:
    command: source
    artifact: result
  target:
    command: target
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = { outer = r.schema.string{} } }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target" }
return r.workflow{ name = "switch-non-object", description = "switch non-object", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.switch{ on = source.outer.verdict, cases = { A = target } } } },
  r.child{ node = target },
} } }
"#,
            ),
            (
                "items-non-object",
                "WFR003",
                r#"name: items-non-object
description: items non-object
schemas:
  result:
    type: object
    properties:
      outer: string
nodes:
  main:
    sequence:
      children: [source, spread]
  source:
    command: source
    artifact: result
  spread:
    fanout:
      items: source.outer.values
      children: [worker]
  worker:
    command: worker
    input: [item]
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = { outer = r.schema.string{} } }
local source = r.command{ name = "source", command = "source", artifact = result }
local worker = r.command{ name = "worker", command = "worker", input = { r.input("item") } }
local spread = r.fanout{ name = "spread", items = source.outer.values, children = { r.child{ node = worker } } }
return r.workflow{ name = "items-non-object", description = "items non-object", main = r.sequence{ children = {
  r.child{ node = source }, r.child{ node = spread },
} } }
"#,
            ),
        ];

        for (name, code, yaml_source, lua_source) in cases {
            let yaml = diagnose_workflow_source(yaml_source, Some(name));
            let lua = diagnose_lua_workflow_source(
                &format!("{name}.lua"),
                lua_source,
                tmp.path(),
                tmp.path(),
                Some(name),
            );
            let yaml_item = yaml
                .diagnostics
                .iter()
                .find(|item| item.code == code)
                .unwrap_or_else(|| panic!("{name} YAML: {:?}", yaml.diagnostics));
            let lua_item = lua
                .diagnostics
                .iter()
                .find(|item| item.code == code)
                .unwrap_or_else(|| panic!("{name} Lua: {:?}", lua.diagnostics));
            assert_eq!(lua_item.code, yaml_item.code, "{name}");
            assert_eq!(lua_item.stage, yaml_item.stage, "{name}");
            assert_eq!(lua_item.message, yaml_item.message, "{name}");
        }
    }
}
