use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::domain::workflow::{
    ChildEntry, NodeDefinition, NodeKindName, SchemaDef, WorkflowDefinition,
};

pub const REQUEST_ARTIFACT: &str = "request";
/// fanout の展開要素を子パラメータへ配線する予約供給元名。
pub const ITEMS_SOURCE: &str = "items";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactReference {
    Request,
    Node { node: String, field: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceParseError {
    Empty,
    InvalidFormat(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceResolveError {
    ReservedNodeName {
        name: String,
    },
    /// 本文（command / facet）の `{{ ... }}` が宣言済み input パラメータ名でない。
    UnknownParameter {
        name: String,
    },
    UnknownField {
        reference: String,
        field: String,
    },
    InvalidInputRef {
        value: String,
    },
}

pub fn parse_reference(input: &str) -> Result<ArtifactReference, ReferenceParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ReferenceParseError::Empty);
    }
    if trimmed.contains(char::is_whitespace) {
        return Err(ReferenceParseError::InvalidFormat(trimmed.to_string()));
    }
    let mut parts = trimmed.split('.');
    let root = parts.next().unwrap_or_default();
    let field = parts.next();
    if parts.next().is_some() {
        return Err(ReferenceParseError::InvalidFormat(trimmed.to_string()));
    }
    if !is_reference_segment(root) || field.is_some_and(|value| !is_reference_segment(value)) {
        return Err(ReferenceParseError::InvalidFormat(trimmed.to_string()));
    }
    match (root, field) {
        (REQUEST_ARTIFACT, None) => Ok(ArtifactReference::Request),
        (REQUEST_ARTIFACT, Some(_)) => Err(ReferenceParseError::InvalidFormat(trimmed.to_string())),
        (node, field) => Ok(ArtifactReference::Node {
            node: node.to_string(),
            field: field.map(ToOwned::to_owned),
        }),
    }
}

pub fn extract_template_references(content: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut start = 0;
    while let Some(open) = content[start..].find("{{") {
        let abs_open = start + open + 2;
        if let Some(close) = content[abs_open..].find("}}") {
            let reference = content[abs_open..abs_open + close].trim();
            if !reference.is_empty() {
                refs.push(reference.to_string());
            }
            start = abs_open + close + 2;
        } else {
            break;
        }
    }
    refs
}

pub(crate) fn validate_workflow_reference_diagnostics(
    workflow: &WorkflowDefinition,
) -> Vec<ReferenceResolveError> {
    let mut errors = Vec::new();

    for node in &workflow.nodes {
        if is_reserved_artifact_name(&node.name) {
            errors.push(ReferenceResolveError::ReservedNodeName {
                name: node.name.clone(),
            });
        }
        if let Some(command) = node.command() {
            validate_node_template_content(node, &workflow.schemas, command, &mut errors);
        }
    }

    errors
}

/// 本文（command / facet）の `{{ ... }}` を、その node の input パラメータ宣言と
/// 突合して検証する。参照できるのはパラメータ名（+ field パス）のみ。
pub fn validate_template_references_for_node(
    node: &NodeDefinition,
    schemas: &BTreeMap<String, SchemaDef>,
    content: &str,
) -> Vec<ReferenceResolveError> {
    let mut errors = Vec::new();
    validate_node_template_content(node, schemas, content, &mut errors);
    errors
}

fn validate_node_template_content(
    node: &NodeDefinition,
    schemas: &BTreeMap<String, SchemaDef>,
    content: &str,
    errors: &mut Vec<ReferenceResolveError>,
) {
    for reference in extract_template_references(content) {
        let Some((root, field)) = split_reference(&reference) else {
            errors.push(ReferenceResolveError::InvalidInputRef { value: reference });
            continue;
        };
        let Some(parameter) = node.input_parameter(root) else {
            errors.push(ReferenceResolveError::UnknownParameter {
                name: root.to_string(),
            });
            continue;
        };
        let Some(field) = field else {
            continue;
        };
        // 型あり（Contract 付き）パラメータの field パスは Contract に対して検証する。
        // 型なしパラメータは供給元の形が実行時に決まるため検証しない。
        if let Some(contract) = parameter.contract.as_deref() {
            if !contract_field_available(contract, field, schemas) {
                errors.push(ReferenceResolveError::UnknownField {
                    reference: root.to_string(),
                    field: field.to_string(),
                });
            }
        }
    }
}

/// `root` / `root.field` の分解。形式不正（空・空白・2 段以上の field）は None。
pub(crate) fn split_reference(value: &str) -> Option<(&str, Option<&str>)> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return None;
    }
    let mut parts = trimmed.split('.');
    let root = parts.next()?;
    let field = parts.next();
    if parts.next().is_some() {
        return None;
    }
    if !is_reference_segment(root) || field.is_some_and(|value| !is_reference_segment(value)) {
        return None;
    }
    Some((root, field))
}

/// fanout items（`<node>.<field>`）の要素 schema を解決する。
/// 参照先は Artifact を産出するカタログ node（fanout の子は親の配列へ集約される
/// ため参照不可）。
pub(crate) fn artifact_field_schema<'a>(
    workflow: &'a WorkflowDefinition,
    node_name: &str,
    field: &str,
) -> Result<Option<&'a SchemaDef>, String> {
    if workflow
        .nodes
        .iter()
        .filter_map(NodeDefinition::fanout)
        .flat_map(|fanout| fanout.children.iter())
        .any(|entry| entry.name == node_name)
    {
        return Err(format!(
            "node '{node_name}' is a fanout child and its Artifact is not referenceable"
        ));
    }
    let Some(node) = workflow.node_by_name(node_name) else {
        return Err(format!("unknown Artifact-producing node '{node_name}'"));
    };
    if !node_has_artifact(node) {
        return Err(format!("node '{node_name}' does not produce an Artifact"));
    }
    if node.kind_name() == NodeKindName::Command
        && crate::domain::workflow::services::contract_schema::COMMAND_RESERVED_FIELDS
            .contains(&field)
    {
        return Ok(None);
    }
    let Some(contract) = node.artifact.as_deref() else {
        return Err(format!(
            "node '{node_name}' has no Artifact field '{field}'"
        ));
    };
    let Some(SchemaDef::Object { properties, .. }) = workflow.schemas.get(contract) else {
        return Err(format!(
            "node '{node_name}' has no Artifact field '{field}'"
        ));
    };
    properties
        .get(field)
        .map(Some)
        .ok_or_else(|| format!("node '{node_name}' has no Artifact field '{field}'"))
}

/// sequence（root スコープ）の children エントリの inputs から、node 起動時の
/// パラメータ束縛を解決する。
///
/// 供給元: 兄弟 node の Artifact（`artifacts` は node 名キー）/ `request`。
/// 解決できない供給元は束縛から除かれる（テンプレートは未解決のまま残る）。
pub fn resolve_entry_bindings(
    entry: Option<&ChildEntry>,
    artifacts: &HashMap<String, Value>,
) -> Vec<(String, Value)> {
    let Some(entry) = entry else {
        return Vec::new();
    };
    entry
        .inputs
        .iter()
        .filter_map(|(parameter, source)| {
            artifacts
                .get(source.root())
                .and_then(|value| field_value(value, source.field()))
                .map(|value| (parameter.clone(), value.clone()))
        })
        .collect()
}

/// fanout の children エントリの inputs から、子 node 起動時のパラメータ束縛を
/// 解決する。
///
/// 供給元は自 fanout の input パラメータ（`parent_parameters`）/ `request` /
/// `items`（展開の各要素 = `item` 引数）に閉じる。兄弟や他 node の直接参照は
/// 存在しない（fanout の子は並走し、Artifact は親配列へ集約されるため）。
///
/// 自動束縛: entry が items を明示配線せず、node のパラメータがちょうど1つで
/// 未配線なら、そのパラメータへ item を束縛する。
pub fn resolve_fanout_child_bindings(
    entry: Option<&ChildEntry>,
    node: &NodeDefinition,
    parent_parameters: &HashMap<String, Value>,
    request: Option<&Value>,
    item: Option<&Value>,
) -> Vec<(String, Value)> {
    let mut bindings: Vec<(String, Value)> = Vec::new();
    let mut items_bound = false;

    if let Some(entry) = entry {
        for (parameter, source) in &entry.inputs {
            let root = source.root();
            let value = if root == ITEMS_SOURCE {
                items_bound = true;
                item.cloned()
            } else if root == REQUEST_ARTIFACT {
                request.cloned()
            } else {
                parent_parameters
                    .get(root)
                    .and_then(|value| field_value(value, source.field()))
                    .cloned()
            };
            if let Some(value) = value {
                bindings.push((parameter.clone(), value));
            }
        }
    }

    if let Some(item) = item {
        if !items_bound {
            let unbound: Vec<_> = node
                .input
                .iter()
                .filter(|param| {
                    !bindings
                        .iter()
                        .any(|(bound, _)| bound == param.name.as_str())
                })
                .collect();
            if node.input.len() == 1 {
                if let [sole] = unbound.as_slice() {
                    bindings.push((sole.name.clone(), item.clone()));
                }
            }
        }
    }

    bindings
}

/// 束縛済みパラメータ値から `{{ root(.field) }}` を解決する。
pub fn resolve_template_value(
    root: &str,
    field: Option<&str>,
    values: &HashMap<String, Value>,
) -> Option<Value> {
    let value = values.get(root)?;
    field_value(value, field).cloned()
}

fn field_value<'a>(value: &'a Value, field: Option<&str>) -> Option<&'a Value> {
    match field {
        None => Some(value),
        Some(field) => value.as_object()?.get(field),
    }
}

pub(crate) fn is_reference_segment(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub(crate) fn is_reserved_artifact_name(value: &str) -> bool {
    value == REQUEST_ARTIFACT
}

pub(crate) fn node_has_artifact(node: &NodeDefinition) -> bool {
    match node.kind_name() {
        NodeKindName::Command | NodeKindName::Fanout => true,
        NodeKindName::Session => node.artifact.is_some(),
        // sequence の Artifact は output の子から委譲される（宣言があるときのみ）。
        NodeKindName::Sequence => node.artifact.is_some(),
    }
}

pub(crate) fn contract_field_available(
    contract: &str,
    field: &str,
    schemas: &BTreeMap<String, SchemaDef>,
) -> bool {
    let Some(SchemaDef::Object { properties, .. }) = schemas.get(contract) else {
        return false;
    };
    properties.contains_key(field)
}

/// 兄弟 node の Artifact field が参照可能か（field パス付き inputs 配線の検証）。
pub(crate) fn node_field_available(
    node: &NodeDefinition,
    field: &str,
    schemas: &BTreeMap<String, SchemaDef>,
) -> bool {
    if node.kind_name() == NodeKindName::Command
        && crate::domain::workflow::services::contract_schema::COMMAND_RESERVED_FIELDS
            .contains(&field)
    {
        return true;
    }
    let Some(contract) = node.artifact.as_deref() else {
        return false;
    };
    contract_field_available(contract, field, schemas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::value_objects::InputSourceRef;
    use crate::domain::workflow::{CommandSpec, InputParam, NodeKind};

    fn command_node_with_params(
        name: &str,
        command: &str,
        params: Vec<InputParam>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: command.to_string(),
            }),
            artifact: None,
            input: params,
            completion: Default::default(),
            worktree: None,
        }
    }

    fn untyped(name: &str) -> InputParam {
        InputParam {
            name: name.to_string(),
            contract: None,
        }
    }

    #[test]
    fn test_reference_parse_requestとnode参照() {
        assert_eq!(parse_reference("request"), Ok(ArtifactReference::Request));
        assert_eq!(
            parse_reference("plan.summary"),
            Ok(ArtifactReference::Node {
                node: "plan".to_string(),
                field: Some("summary".to_string())
            })
        );
        assert!(parse_reference("request.field").is_err());
    }

    #[test]
    fn test_本文検証_宣言済みパラメータ名のみ参照できる() {
        let node = command_node_with_params(
            "delete",
            "rm -f -- '{{ spec }}/behavior.md'",
            vec![untyped("spec")],
        );
        let errors =
            validate_template_references_for_node(&node, &BTreeMap::new(), node.command().unwrap());
        assert!(errors.is_empty());
    }

    #[test]
    fn test_本文検証_未宣言の参照を拒否する() {
        let node = command_node_with_params("echo", "echo '{{ item }}'", vec![untyped("task")]);
        let errors =
            validate_template_references_for_node(&node, &BTreeMap::new(), node.command().unwrap());
        assert!(errors.iter().any(|error| matches!(
            error,
            ReferenceResolveError::UnknownParameter { name } if name == "item"
        )));
    }

    #[test]
    fn test_束縛解決_sequenceは兄弟とrequestを解決する() {
        let entry = ChildEntry {
            name: "consume".to_string(),
            inputs: vec![
                ("spec".to_string(), InputSourceRef::new("collect.spec_dir")),
                ("goal".to_string(), InputSourceRef::new("request")),
            ],
            rules: None,
        };
        let mut artifacts = HashMap::new();
        artifacts.insert(
            "collect".to_string(),
            serde_json::json!({"spec_dir": "specs/x"}),
        );
        artifacts.insert(
            REQUEST_ARTIFACT.to_string(),
            Value::String("build it".to_string()),
        );

        let bindings = resolve_entry_bindings(Some(&entry), &artifacts);

        assert_eq!(
            bindings,
            vec![
                ("spec".to_string(), Value::String("specs/x".to_string())),
                ("goal".to_string(), Value::String("build it".to_string())),
            ]
        );
    }

    #[test]
    fn test_束縛解決_fanout子は親パラメータとrequestとitemsを解決する() {
        let node = command_node_with_params(
            "worker",
            "echo",
            vec![untyped("thread"), untyped("spec"), untyped("goal")],
        );
        let entry = ChildEntry {
            name: "worker".to_string(),
            inputs: vec![
                ("thread".to_string(), InputSourceRef::new("items")),
                ("spec".to_string(), InputSourceRef::new("context.spec_dir")),
                ("goal".to_string(), InputSourceRef::new("request")),
            ],
            rules: None,
        };
        let mut parent_parameters = HashMap::new();
        parent_parameters.insert(
            "context".to_string(),
            serde_json::json!({"spec_dir": "specs/x"}),
        );
        let request = Value::String("build it".to_string());
        let item = serde_json::json!({"thread_id": "t-1"});

        let bindings = resolve_fanout_child_bindings(
            Some(&entry),
            &node,
            &parent_parameters,
            Some(&request),
            Some(&item),
        );

        assert_eq!(
            bindings,
            vec![
                ("thread".to_string(), item.clone()),
                ("spec".to_string(), Value::String("specs/x".to_string())),
                ("goal".to_string(), Value::String("build it".to_string())),
            ]
        );
    }

    #[test]
    fn test_束縛解決_fanout子は兄弟nodeを直接参照できない() {
        let node = command_node_with_params("worker", "echo", vec![untyped("spec")]);
        let entry = ChildEntry {
            name: "worker".to_string(),
            inputs: vec![("spec".to_string(), InputSourceRef::new("collect"))],
            rules: None,
        };

        let bindings =
            resolve_fanout_child_bindings(Some(&entry), &node, &HashMap::new(), None, None);

        assert!(bindings.is_empty());
    }

    #[test]
    fn test_束縛解決_単一パラメータへのitems自動束縛() {
        let node = command_node_with_params("worker", "echo", vec![untyped("task")]);
        let item = serde_json::json!({"task_id": "T1"});

        let bindings =
            resolve_fanout_child_bindings(None, &node, &HashMap::new(), None, Some(&item));

        assert_eq!(bindings, vec![("task".to_string(), item)]);
    }

    #[test]
    fn test_束縛解決_解決できない供給元は束縛から除かれる() {
        let entry = ChildEntry {
            name: "consume".to_string(),
            inputs: vec![("spec".to_string(), InputSourceRef::new("missing_node"))],
            rules: None,
        };

        let bindings = resolve_entry_bindings(Some(&entry), &HashMap::new());

        assert!(bindings.is_empty());
    }
}
