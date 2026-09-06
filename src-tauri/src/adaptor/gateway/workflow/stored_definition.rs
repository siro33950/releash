use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::domain::workflow::{
    DefinitionResolution, ExecutionOrigin, ExecutionParentRef, ExecutionTreeLaunch, NodeFact,
    StartedFact, TreeRootFact, WorkflowDefinition,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TreeRootHeader {
    pub(crate) workspace_identity: String,
    pub(crate) worktree_path: String,
    pub(crate) launched_as: ExecutionTreeLaunch,
}

#[derive(Deserialize)]
struct HeaderRecord {
    root: Option<TreeRootHeader>,
}

pub(crate) fn read_tree_header(detail: &str) -> Result<Option<TreeRootHeader>, String> {
    serde_json::from_str::<HeaderRecord>(detail)
        .map(|record| record.root)
        .map_err(|error| format!("tree root metadata is unavailable: {error}"))
}

#[derive(Deserialize)]
struct StartedRecord {
    parent: Option<ExecutionParentRef>,
    root: Option<RootRecord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RootRecord {
    workspace_identity: String,
    worktree_path: String,
    created_from: String,
    request: String,
    definition: Value,
    launched_as: ExecutionTreeLaunch,
}

#[derive(Deserialize)]
struct DefinitionRecord {
    name: String,
    description: String,
    #[serde(default)]
    builtin: bool,
    #[serde(default)]
    schemas: BTreeMap<String, Value>,
    nodes: BTreeMap<String, Value>,
    entry: String,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

pub(crate) fn decode_started(detail: &str) -> Result<NodeFact, String> {
    let record: StartedRecord = serde_json::from_str(detail).map_err(|error| error.to_string())?;
    let root = record
        .root
        .map(|root| {
            let (definition, definition_resolution) = read_definition(root.definition);
            Ok::<_, String>(TreeRootFact {
                workspace_identity: root.workspace_identity,
                worktree_path: root.worktree_path,
                created_from: ExecutionOrigin::from_public_value(&root.created_from)
                    .map_err(|error| error.to_string())?,
                request: root.request,
                definition,
                definition_resolution: Box::new(definition_resolution),
                launched_as: root.launched_as,
            })
        })
        .transpose()?;
    Ok(NodeFact::Started(StartedFact {
        parent: record.parent,
        root,
    }))
}

fn read_definition(value: Value) -> (WorkflowDefinition, DefinitionResolution) {
    let record = match serde_json::from_value::<DefinitionRecord>(value) {
        Ok(record) => record,
        Err(error) => {
            return (
                WorkflowDefinition::default(),
                DefinitionResolution {
                    definition_error: Some(format!("Workflow definition is unavailable: {error}")),
                    ..DefinitionResolution::default()
                },
            )
        }
    };
    let mut definition = WorkflowDefinition {
        name: record.name,
        description: record.description,
        builtin: record.builtin,
        entry: record.entry,
        ..WorkflowDefinition::default()
    };
    let mut resolution = DefinitionResolution::default();
    if !record.extra.is_empty() {
        resolution.definition_error = Some(format!(
            "Workflow definition has unsupported fields: {}",
            record.extra.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    for (name, value) in record.schemas {
        match serde_json::from_value(value) {
            Ok(schema) => {
                definition.schemas.insert(name, schema);
            }
            Err(error) => {
                resolution.schema_errors.insert(name, error.to_string());
            }
        }
    }
    for (name, value) in record.nodes {
        let dynamic_fanout = value
            .get("fanout")
            .and_then(|fanout| fanout.get("items"))
            .and_then(|items| {
                serde_json::from_value::<crate::domain::workflow::ItemsSource>(items.clone()).ok()
            })
            .is_some_and(|items| {
                matches!(
                    items,
                    crate::domain::workflow::ItemsSource::ArtifactField { .. }
                )
            });
        let single_node = serde_json::json!({
            "name": "", "description": "", "nodes": { &name: value }
        });
        match serde_json::from_value::<WorkflowDefinition>(single_node) {
            Ok(node_definition) => definition.nodes.extend(node_definition.nodes),
            Err(error) => {
                if dynamic_fanout {
                    resolution.dynamic_fanout_names.insert(name.clone());
                }
                resolution.node_errors.insert(name, error.to_string());
            }
        }
    }
    (definition, resolution)
}

#[cfg(test)]
#[path = "stored_definition_test.rs"]
mod stored_definition_tests;
