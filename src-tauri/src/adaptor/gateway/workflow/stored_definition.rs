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
    #[serde(default)]
    pub(crate) repository_root: Option<String>,
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
pub(crate) struct TreeRootContext {
    #[serde(flatten)]
    pub(crate) header: TreeRootHeader,
    pub(crate) definition: Value,
}

pub(crate) fn read_tree_context(detail: &str) -> Result<Option<TreeRootContext>, String> {
    #[derive(Deserialize)]
    struct Record {
        root: Option<TreeRootContext>,
    }
    serde_json::from_str::<Record>(detail)
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
    #[serde(default)]
    repository_root: Option<String>,
    workspace_identity: String,
    worktree_path: String,
    created_from: String,
    request: String,
    definition: Box<serde_json::value::RawValue>,
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
    nodes: BTreeMap<String, Box<serde_json::value::RawValue>>,
    entry: String,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

pub(crate) fn decode_started(detail: &str) -> Result<NodeFact, String> {
    let record: StartedRecord = serde_json::from_str(detail).map_err(|error| error.to_string())?;
    let root = record
        .root
        .map(|root| {
            let (definition, definition_resolution) = read_definition(&root.definition);
            Ok::<_, String>(TreeRootFact {
                repository_root: root.repository_root,
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
        root: root.map(Box::new),
    }))
}

fn read_definition(
    value: &serde_json::value::RawValue,
) -> (WorkflowDefinition, DefinitionResolution) {
    let record = match serde_json::from_str::<DefinitionRecord>(value.get()) {
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
        #[derive(serde::Serialize)]
        struct SingleNode<'a> {
            name: &'static str,
            description: &'static str,
            nodes: BTreeMap<&'a str, &'a serde_json::value::RawValue>,
        }
        let single_node = SingleNode {
            name: "",
            description: "",
            nodes: BTreeMap::from([(name.as_str(), value.as_ref())]),
        };
        let parsed = serde_json::to_string(&single_node)
            .and_then(|source| serde_json::from_str::<WorkflowDefinition>(&source));
        match parsed {
            Ok(node_definition) => definition.nodes.extend(node_definition.nodes),
            Err(error) => {
                let dynamic_fanout = serde_json::from_str::<Value>(value.get())
                    .ok()
                    .and_then(|value| value.get("fanout")?.get("items").cloned())
                    .and_then(|items| {
                        serde_json::from_value::<crate::domain::workflow::ItemsSource>(items).ok()
                    })
                    .is_some_and(|items| {
                        matches!(
                            items,
                            crate::domain::workflow::ItemsSource::ArtifactField { .. }
                        )
                    });
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
