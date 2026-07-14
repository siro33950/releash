use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

use crate::domain::workflow::{NodeDefinition, NodeKindName, SchemaDef, WorkflowDefinition};

pub const REQUEST_ARTIFACT: &str = "request";
pub const ITEM_ARTIFACT: &str = "item";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactReference {
    Request,
    Node { node: String, field: Option<String> },
    Item { field: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceParseError {
    Empty,
    InvalidFormat(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceResolveError {
    ReservedNodeName { name: String },
    UnknownNode { name: String },
    UnavailableArtifact { name: String },
    UnknownField { reference: String, field: String },
    ItemOutOfScope,
    InvalidInputRef { value: String },
    InputsNotAllowedOnFanout { node: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceResolveContext {
    Inputs,
    Template,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceResolveDiagnostic {
    pub(crate) error: ReferenceResolveError,
    pub(crate) context: ReferenceResolveContext,
}

impl ReferenceResolveDiagnostic {
    fn new(error: ReferenceResolveError, context: ReferenceResolveContext) -> Self {
        Self { error, context }
    }
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
        (ITEM_ARTIFACT, field) => Ok(ArtifactReference::Item {
            field: field.map(ToOwned::to_owned),
        }),
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
) -> Vec<ReferenceResolveDiagnostic> {
    let mut errors = Vec::new();
    let context = ReferenceValidationContext::new(workflow);

    for node in &workflow.nodes {
        if is_reserved_artifact_name(&node.name) {
            errors.push(ReferenceResolveDiagnostic::new(
                ReferenceResolveError::ReservedNodeName {
                    name: node.name.clone(),
                },
                ReferenceResolveContext::Inputs,
            ));
        }
        if node.is_fanout() && !node.inputs.is_empty() {
            errors.push(ReferenceResolveDiagnostic::new(
                ReferenceResolveError::InputsNotAllowedOnFanout {
                    node: node.name.clone(),
                },
                ReferenceResolveContext::Inputs,
            ));
        }
        for input in &node.inputs {
            match parse_reference(input) {
                Ok(ArtifactReference::Request)
                | Ok(ArtifactReference::Node { field: None, .. }) => {
                    let mut input_errors = Vec::new();
                    validate_reference(input, &context, false, &mut input_errors);
                    errors.extend(input_errors.into_iter().map(|error| {
                        ReferenceResolveDiagnostic::new(error, ReferenceResolveContext::Inputs)
                    }));
                }
                Ok(_) | Err(_) => errors.push(ReferenceResolveDiagnostic::new(
                    ReferenceResolveError::InvalidInputRef {
                        value: input.clone(),
                    },
                    ReferenceResolveContext::Inputs,
                )),
            }
        }
        let mut template_errors = Vec::new();
        validate_node_templates(
            node,
            &context,
            context.fanout_child_names.contains(node.name.as_str()),
            &mut template_errors,
        );
        errors.extend(template_errors.into_iter().map(|error| {
            ReferenceResolveDiagnostic::new(error, ReferenceResolveContext::Template)
        }));
    }

    errors
}

pub fn validate_template_references(
    workflow: &WorkflowDefinition,
    content: &str,
    allow_item: bool,
) -> Vec<ReferenceResolveError> {
    let context = ReferenceValidationContext::new(workflow);
    let mut errors = Vec::new();
    validate_template_content(content, &context, allow_item, &mut errors);
    errors
}

struct ReferenceValidationContext<'a> {
    top_level_nodes: HashMap<&'a str, &'a NodeDefinition>,
    fanout_child_names: HashSet<&'a str>,
    schemas: &'a BTreeMap<String, SchemaDef>,
}

impl<'a> ReferenceValidationContext<'a> {
    fn new(workflow: &'a WorkflowDefinition) -> Self {
        Self {
            top_level_nodes: workflow
                .nodes
                .iter()
                .map(|node| (node.name.as_str(), node))
                .collect(),
            fanout_child_names: workflow
                .nodes
                .iter()
                .filter_map(NodeDefinition::fanout)
                .flat_map(|fanout| fanout.child.iter().map(String::as_str))
                .collect(),
            schemas: &workflow.schemas,
        }
    }
}

fn validate_node_templates(
    node: &NodeDefinition,
    context: &ReferenceValidationContext<'_>,
    allow_item: bool,
    errors: &mut Vec<ReferenceResolveError>,
) {
    if let Some(command) = node.command() {
        validate_template_content(command, context, allow_item, errors);
    }
}

fn validate_template_content(
    content: &str,
    context: &ReferenceValidationContext<'_>,
    allow_item: bool,
    errors: &mut Vec<ReferenceResolveError>,
) {
    for reference in extract_template_references(content) {
        match parse_reference(&reference) {
            Ok(_) => validate_reference(&reference, context, allow_item, errors),
            Err(_) => errors.push(ReferenceResolveError::InvalidInputRef { value: reference }),
        }
    }
}

fn validate_reference(
    raw: &str,
    context: &ReferenceValidationContext<'_>,
    allow_item: bool,
    errors: &mut Vec<ReferenceResolveError>,
) {
    match parse_reference(raw) {
        Ok(ArtifactReference::Request) => {}
        Ok(ArtifactReference::Item { .. }) if allow_item => {}
        Ok(ArtifactReference::Item { .. }) => errors.push(ReferenceResolveError::ItemOutOfScope),
        Ok(ArtifactReference::Node { node, field }) => {
            if context.fanout_child_names.contains(node.as_str()) {
                errors.push(ReferenceResolveError::UnavailableArtifact { name: node });
                return;
            }
            let Some(definition) = context.top_level_nodes.get(node.as_str()) else {
                errors.push(ReferenceResolveError::UnknownNode { name: node });
                return;
            };
            if !node_has_artifact(definition) {
                errors.push(ReferenceResolveError::UnavailableArtifact { name: node });
                return;
            }
            if let Some(field) = field {
                if !node_field_available(definition, &field, context.schemas) {
                    errors.push(ReferenceResolveError::UnknownField {
                        reference: node,
                        field,
                    });
                }
            }
        }
        Err(_) => errors.push(ReferenceResolveError::InvalidInputRef {
            value: raw.to_string(),
        }),
    }
}

pub(crate) fn artifact_field_schema<'a>(
    workflow: &'a WorkflowDefinition,
    node_name: &str,
    field: &str,
) -> Result<Option<&'a SchemaDef>, ReferenceResolveError> {
    let context = ReferenceValidationContext::new(workflow);
    if context.fanout_child_names.contains(node_name) {
        return Err(ReferenceResolveError::UnavailableArtifact {
            name: node_name.to_string(),
        });
    }
    let Some(node) = context.top_level_nodes.get(node_name).copied() else {
        return Err(ReferenceResolveError::UnknownNode {
            name: node_name.to_string(),
        });
    };
    if !node_has_artifact(node) {
        return Err(ReferenceResolveError::UnavailableArtifact {
            name: node_name.to_string(),
        });
    }
    if node.kind_name() == NodeKindName::Command
        && crate::domain::workflow::services::contract_schema::COMMAND_RESERVED_FIELDS
            .contains(&field)
    {
        return Ok(None);
    }
    let Some(contract) = node.artifact.as_deref() else {
        return Err(ReferenceResolveError::UnknownField {
            reference: node_name.to_string(),
            field: field.to_string(),
        });
    };
    let Some(SchemaDef::Object { properties, .. }) = context.schemas.get(contract) else {
        return Err(ReferenceResolveError::UnknownField {
            reference: node_name.to_string(),
            field: field.to_string(),
        });
    };
    properties
        .get(field)
        .map(Some)
        .ok_or_else(|| ReferenceResolveError::UnknownField {
            reference: node_name.to_string(),
            field: field.to_string(),
        })
}

pub(crate) fn resolve_runtime_reference(
    reference: &ArtifactReference,
    artifacts: &HashMap<String, Value>,
    item: Option<&Value>,
) -> Option<Value> {
    match reference {
        ArtifactReference::Request => artifacts.get(REQUEST_ARTIFACT).cloned(),
        ArtifactReference::Node { node, field } => {
            let value = artifacts.get(node)?;
            field_value(value, field.as_deref()).cloned()
        }
        ArtifactReference::Item { field } => {
            let value = item?;
            field_value(value, field.as_deref()).cloned()
        }
    }
}

fn field_value<'a>(value: &'a Value, field: Option<&str>) -> Option<&'a Value> {
    match field {
        None => Some(value),
        Some(field) => value.as_object()?.get(field),
    }
}

fn is_reference_segment(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_reserved_artifact_name(value: &str) -> bool {
    matches!(value, REQUEST_ARTIFACT | ITEM_ARTIFACT)
}

fn node_has_artifact(node: &NodeDefinition) -> bool {
    match node.kind_name() {
        NodeKindName::Command | NodeKindName::Fanout => true,
        NodeKindName::Session => node.artifact.is_some(),
    }
}

fn node_field_available(
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
    let Some(SchemaDef::Object { properties, .. }) = schemas.get(contract) else {
        return false;
    };
    properties.contains_key(field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{CommandSpec, FanoutSpec, NodeKind};

    fn command_node(name: &str, command: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: command.to_string(),
            }),
            ..Default::default()
        }
    }

    fn fanout_node(name: &str, child: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec![child.to_string()],
                items: None,
            }),
            ..Default::default()
        }
    }

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            nodes,
            ..Default::default()
        }
    }

    #[test]
    fn test_reference_parse_request_node_and_item() {
        assert_eq!(parse_reference("request"), Ok(ArtifactReference::Request));
        assert_eq!(
            parse_reference("plan.summary"),
            Ok(ArtifactReference::Node {
                node: "plan".to_string(),
                field: Some("summary".to_string())
            })
        );
        assert_eq!(
            parse_reference("item.path"),
            Ok(ArtifactReference::Item {
                field: Some("path".to_string())
            })
        );
    }

    #[test]
    fn fanout_child_artifact_is_not_globally_referenceable() {
        let workflow = workflow(vec![
            fanout_node("fanout", "worker"),
            command_node("worker", "echo work"),
            command_node("consume", "echo {{ worker.stdout }}"),
        ]);

        let diagnostics = validate_workflow_reference_diagnostics(&workflow);

        assert!(diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.error,
            ReferenceResolveError::UnavailableArtifact { name } if name == "worker"
        )));
    }

    #[test]
    fn item_template_reference_is_available_on_top_level_fanout_child() {
        let workflow = workflow(vec![
            fanout_node("fanout", "worker"),
            command_node("worker", "echo {{ item.path }}"),
        ]);

        let diagnostics = validate_workflow_reference_diagnostics(&workflow);

        assert!(!diagnostics
            .iter()
            .any(|diagnostic| matches!(&diagnostic.error, ReferenceResolveError::ItemOutOfScope)));
    }
}
