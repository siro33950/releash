use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

use crate::domain::workflow::{
    ChildEntry, EnvironmentVariableName, FieldPath, InputParameterRef, NodeDefinition, NodeKind,
    NodeKindName, SchemaDef, WorkflowDefinition,
};

pub const REQUEST_ARTIFACT: &str = "request";
/// fanout の展開要素を子パラメータへ配線する予約供給元名。
pub const ITEMS_SOURCE: &str = "items";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandEnvironmentReferenceError {
    pub node: String,
    pub variable: String,
    pub reference: String,
    pub source: ReferenceResolveError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandEnvironmentResolutionError {
    MissingParameter { reference: String },
    MissingField { reference: String },
}

impl std::fmt::Display for CommandEnvironmentResolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingParameter { reference } => write!(
                formatter,
                "command environment reference '{reference}' has no runtime input binding"
            ),
            Self::MissingField { reference } => write!(
                formatter,
                "command environment reference '{reference}' has no runtime field value"
            ),
        }
    }
}

impl std::error::Error for CommandEnvironmentResolutionError {}

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

pub(crate) fn validate_workflow_command_environment_references(
    workflow: &WorkflowDefinition,
) -> Vec<CommandEnvironmentReferenceError> {
    let mut errors = Vec::new();
    for node in &workflow.nodes {
        let Some(command) = node.command_spec() else {
            continue;
        };
        for (variable, input_reference) in &command.env {
            if let Some(source) = validate_input_parameter_reference(
                node,
                &workflow.schemas,
                input_reference.parameter(),
                input_reference.field_path(),
            ) {
                errors.push(CommandEnvironmentReferenceError {
                    node: node.name.clone(),
                    variable: variable.as_str().to_string(),
                    reference: input_reference.as_string(),
                    source,
                });
            }
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
        let Some((root, field_path)) = split_reference(&reference) else {
            errors.push(ReferenceResolveError::InvalidInputRef { value: reference });
            continue;
        };
        if let Some(error) = validate_input_parameter_reference(node, schemas, root, &field_path) {
            errors.push(error);
        }
    }
}

fn validate_input_parameter_reference(
    node: &NodeDefinition,
    schemas: &BTreeMap<String, SchemaDef>,
    root: &str,
    field_path: &FieldPath,
) -> Option<ReferenceResolveError> {
    let Some(parameter) = node.input_parameter(root) else {
        return Some(ReferenceResolveError::UnknownParameter {
            name: root.to_string(),
        });
    };
    if field_path.is_empty() {
        return None;
    }
    // 型あり（Contract 付き）パラメータの field パスは Contract に対して検証する。
    // 型なしパラメータは供給元の形が実行時に決まるため検証しない。
    if let Some(contract) = parameter.contract.as_deref() {
        let resolved = schemas.get(contract).and_then(|schema| {
            crate::domain::workflow::services::contract_schema::resolve_field_path(
                schema, field_path,
            )
            .ok()
        });
        if resolved.is_none() {
            return Some(ReferenceResolveError::UnknownField {
                reference: root.to_string(),
                field: field_path.as_string(),
            });
        }
    }
    None
}

/// `root` / `root.field...` の分解。
pub(crate) fn split_reference(value: &str) -> Option<(&str, FieldPath)> {
    let (_, field_path) = FieldPath::from_reference(value).ok()?;
    Some((value.split('.').next()?, field_path))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeReferenceSchemaError {
    ArtifactNotObject,
    NoReferenceableArtifact,
}

pub(crate) fn node_reference_schema(
    workflow: &WorkflowDefinition,
    node: &NodeDefinition,
) -> Result<SchemaDef, NodeReferenceSchemaError> {
    fn resolve(
        workflow: &WorkflowDefinition,
        node: &NodeDefinition,
        visited: &mut HashSet<String>,
    ) -> Result<SchemaDef, NodeReferenceSchemaError> {
        if !visited.insert(node.name.clone()) {
            return Err(NodeReferenceSchemaError::NoReferenceableArtifact);
        }
        let artifact_schema = node
            .artifact
            .as_deref()
            .and_then(|contract| workflow.schemas.get(contract));
        match &node.kind {
            NodeKind::Command(_) => {
                crate::domain::workflow::services::contract_schema::command_reference_schema(
                    artifact_schema,
                )
                .map_err(|_| NodeReferenceSchemaError::ArtifactNotObject)
            }
            NodeKind::Session(_) => artifact_schema
                .cloned()
                .ok_or(NodeReferenceSchemaError::NoReferenceableArtifact),
            NodeKind::Fanout(_) => Err(NodeReferenceSchemaError::NoReferenceableArtifact),
            NodeKind::Sequence(sequence) => {
                let properties = sequence
                    .children
                    .iter()
                    .filter_map(|entry| {
                        let child = workflow.node_by_name(&entry.name)?;
                        resolve(workflow, child, visited)
                            .ok()
                            .map(|schema| (entry.name.clone(), schema))
                    })
                    .collect();
                Ok(SchemaDef::Object {
                    properties,
                    required: Default::default(),
                })
            }
        }
    }
    resolve(workflow, node, &mut HashSet::new())
}

/// fanout items（`<node>.<field>...`）の終端 schema を解決する。
/// 参照先は Artifact を産出するカタログ node（fanout の子は親の配列へ集約される
/// ため参照不可）。
pub(crate) fn artifact_field_schema(
    workflow: &WorkflowDefinition,
    node_name: &str,
    field_path: &FieldPath,
) -> Result<SchemaDef, String> {
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
    let schema = node_reference_schema(workflow, node).map_err(|error| match error {
        NodeReferenceSchemaError::ArtifactNotObject => {
            format!("node '{node_name}' Artifact Contract is not an object")
        }
        NodeReferenceSchemaError::NoReferenceableArtifact => {
            format!("node '{node_name}' has no Artifact field path '{field_path}'")
        }
    })?;
    crate::domain::workflow::services::contract_schema::resolve_field_path(&schema, field_path)
    .map(|resolved| resolved.schema.clone())
    .map_err(|error| match error.kind {
        crate::domain::workflow::services::contract_schema::FieldPathResolutionErrorKind::NonObject => format!(
            "node '{node_name}' Artifact cannot resolve segment {} ('{}') from a non-object value",
            error.position + 1,
            error.segment
        ),
        crate::domain::workflow::services::contract_schema::FieldPathResolutionErrorKind::MissingProperty => format!(
            "node '{node_name}' Artifact does not declare segment {} ('{}')",
            error.position + 1,
            error.segment
        ),
    })
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
                .and_then(|value| {
                    let (_, field_path) = FieldPath::from_reference(source.raw()).ok()?;
                    resolve_value_at_path(value, &field_path)
                })
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
                    .and_then(|value| {
                        let (_, field_path) = FieldPath::from_reference(source.raw()).ok()?;
                        resolve_value_at_path(value, &field_path)
                    })
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
pub fn resolve_template_value<'a>(
    root: &str,
    field_path: &FieldPath,
    values: &'a HashMap<String, Value>,
) -> Option<&'a Value> {
    let value = values.get(root)?;
    resolve_value_at_path(value, field_path)
}

pub fn resolve_command_environment(
    env: &BTreeMap<EnvironmentVariableName, InputParameterRef>,
    bindings: &[(String, Value)],
) -> Result<Vec<(String, String)>, CommandEnvironmentResolutionError> {
    let values: HashMap<&str, &Value> = bindings
        .iter()
        .map(|(parameter, value)| (parameter.as_str(), value))
        .collect();
    env.iter()
        .map(|(variable, input_reference)| {
            let value = values.get(input_reference.parameter()).ok_or_else(|| {
                CommandEnvironmentResolutionError::MissingParameter {
                    reference: input_reference.as_string(),
                }
            })?;
            let value =
                resolve_value_at_path(value, input_reference.field_path()).ok_or_else(|| {
                    CommandEnvironmentResolutionError::MissingField {
                        reference: input_reference.as_string(),
                    }
                })?;
            let value = reference_value_to_string(value);
            Ok((variable.as_str().to_string(), value))
        })
        .collect()
}

pub fn reference_value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        value => serde_json::to_string(value)
            .expect("serde_json::Value must always serialize to JSON text"),
    }
}

pub fn resolve_value_at_path<'a>(value: &'a Value, field_path: &FieldPath) -> Option<&'a Value> {
    let mut current = value;
    for segment in field_path.segments() {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

pub(crate) fn is_reserved_artifact_name(value: &str) -> bool {
    value == REQUEST_ARTIFACT
}

#[cfg(test)]
#[path = "reference_path_test.rs"]
mod reference_path_test;

pub(crate) fn node_has_artifact(node: &NodeDefinition) -> bool {
    match node.kind_name() {
        NodeKindName::Command | NodeKindName::Fanout | NodeKindName::Sequence => true,
        NodeKindName::Session => node.artifact.is_some(),
    }
}

#[cfg(test)]
#[path = "reference_test.rs"]
mod reference_tests;

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
                env: Default::default(),
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

    fn command_env(
        entries: &[(&str, &str)],
    ) -> BTreeMap<EnvironmentVariableName, InputParameterRef> {
        entries
            .iter()
            .map(|(name, reference)| {
                (
                    EnvironmentVariableName::new(*name).unwrap(),
                    InputParameterRef::new(*reference).unwrap(),
                )
            })
            .collect()
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
    fn test_command環境解決_stringは無変換で非stringはcompact_jsonになる() {
        let env = command_env(&[
            ("DOC", "document"),
            ("META", "metadata"),
            ("COUNT", "metadata.count"),
        ]);
        let bindings = vec![
            (
                "document".to_string(),
                Value::String("{{ untouched }}; `still data`\n$HOME".to_string()),
            ),
            (
                "metadata".to_string(),
                serde_json::json!({"count": 2, "ready": true}),
            ),
        ];

        let resolved = resolve_command_environment(&env, &bindings).unwrap();

        assert!(resolved.contains(&(
            "DOC".to_string(),
            "{{ untouched }}; `still data`\n$HOME".to_string()
        )));
        assert!(resolved.contains(&(
            "META".to_string(),
            r#"{"count":2,"ready":true}"#.to_string()
        )));
        assert!(resolved.contains(&("COUNT".to_string(), "2".to_string())));
    }

    #[test]
    fn test_command環境解決_束縛またはfieldが無ければ全体を失敗する() {
        let missing_parameter = command_env(&[("DOC", "document")]);
        assert!(matches!(
            resolve_command_environment(&missing_parameter, &[]),
            Err(CommandEnvironmentResolutionError::MissingParameter { .. })
        ));

        let missing_field = command_env(&[("DOC", "document.body")]);
        let bindings = vec![("document".to_string(), serde_json::json!({"title": "x"}))];
        assert!(matches!(
            resolve_command_environment(&missing_field, &bindings),
            Err(CommandEnvironmentResolutionError::MissingField { .. })
        ));
    }

    #[test]
    fn test_command環境参照検証_未宣言inputと型ありinputの未知fieldを拒否する() {
        let mut node = command_node_with_params(
            "main",
            "true",
            vec![InputParam {
                name: "document".to_string(),
                contract: Some("document-contract".to_string()),
            }],
        );
        let NodeKind::Command(command) = &mut node.kind else {
            unreachable!();
        };
        command.env = command_env(&[("UNKNOWN", "missing"), ("FIELD", "document.body")]);
        let workflow = WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            schemas: [(
                "document-contract".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::new(),
                    required: Default::default(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![node],
            entry: "main".to_string(),
            ..Default::default()
        };

        let errors = validate_workflow_command_environment_references(&workflow);

        assert!(errors.iter().any(|error| matches!(
            &error.source,
            ReferenceResolveError::UnknownParameter { .. }
        )));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.source, ReferenceResolveError::UnknownField { .. })));
    }

    #[test]
    fn test_束縛解決_sequenceは兄弟とrequestを解決する() {
        let entry = ChildEntry {
            on_failure: None,
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
            on_failure: None,
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
            on_failure: None,
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
            on_failure: None,
            name: "consume".to_string(),
            inputs: vec![("spec".to_string(), InputSourceRef::new("missing_node"))],
            rules: None,
        };

        let bindings = resolve_entry_bindings(Some(&entry), &HashMap::new());

        assert!(bindings.is_empty());
    }
}
