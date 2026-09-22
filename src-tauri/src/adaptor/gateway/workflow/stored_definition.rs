use crate::adaptor::gateway::workflow::fact_codec;
use serde::Deserialize;
use serde_json::Value;

use crate::domain::workflow::{
    ExecutionOrigin, ExecutionParentRef, ExecutionTreeLaunch, NodeFact, StartedFact, TreeRootFact,
    WorkflowDefinition,
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
    worktree: Option<crate::domain::workflow::IsolatedWorktree>,
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
    #[serde(default)]
    workflow_name: String,
    definition: Box<serde_json::value::RawValue>,
    launched_as: ExecutionTreeLaunch,
}

pub(crate) fn decode_started(detail: &str) -> Result<NodeFact, String> {
    fact_codec::decode("started", detail).map_err(|error| error.to_string())
}

pub(crate) fn definition_error(detail: &str) -> Result<Option<String>, String> {
    let record: StartedRecord = serde_json::from_str(detail).map_err(|error| error.to_string())?;
    record
        .root
        .map(|root| {
            ExecutionOrigin::from_public_value(&root.created_from)
                .map_err(|error| error.to_string())?;
            #[derive(Deserialize)]
            struct Snapshot {
                #[serde(rename = "entry")]
                _entry: String,
                #[serde(flatten)]
                _definition: WorkflowDefinition,
            }
            Ok(serde_json::from_str::<Snapshot>(root.definition.get())
                .err()
                .map(|error| format!("Workflow definition is unavailable: {error}")))
        })
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn decode_terminal_started(detail: &str) -> Result<NodeFact, String> {
    let record: StartedRecord = serde_json::from_str(detail).map_err(|error| error.to_string())?;
    let root = record
        .root
        .map(|root| {
            Ok::<_, String>(Box::new(TreeRootFact {
                repository_root: root.repository_root,
                workspace_identity: root.workspace_identity,
                worktree_path: root.worktree_path,
                created_from: ExecutionOrigin::from_public_value(&root.created_from)
                    .map_err(|error| error.to_string())?,
                request: root.request,
                workflow_name: root.workflow_name,
                definition: None,
                launched_as: root.launched_as,
            }))
        })
        .transpose()?;
    Ok(NodeFact::Started(StartedFact {
        worktree: record.worktree,
        parent: record.parent,
        root,
    }))
}

#[cfg(test)]
#[path = "stored_definition_test.rs"]
mod stored_definition_tests;
