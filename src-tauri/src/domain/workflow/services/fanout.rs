//! Pure fanout expansion and parent-completion rules.

use std::collections::HashMap;

use crate::domain::workflow::value_objects::{
    default_node_history_status, ChildEntry, FailureDisposition, FanoutChildSnapshot, ItemsSource,
    NodeExecutionFailureKind, NodeHistoryEntry, RuntimeArtifact, TokenUsage, WorkflowDefinition,
};
use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub struct FanoutChildOutputMerge {
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutChildCompletionInput {
    pub node_name: String,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub artifact: serde_json::Value,
    pub contract: Option<String>,
    pub token_usage: TokenUsage,
    pub attempt: u32,
    pub completed_at: f64,
    pub state: String,
    pub failure_kind: Option<NodeExecutionFailureKind>,
    pub failure_disposition: Option<FailureDisposition>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutParentCompletionPlan {
    pub parent_artifact: RuntimeArtifact,
    pub history_entry: NodeHistoryEntry,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutChildExpansionPlan {
    pub node_name: String,
    pub attempt: u32,
    pub item: Option<serde_json::Value>,
    pub item_index: Option<usize>,
    pub child_index: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutExpansionPlan {
    pub children: Vec<FanoutChildExpansionPlan>,
}

#[cfg(test)]
pub fn merge_fanout_child_completion_output(
    completed_artifact: Option<serde_json::Value>,
    prior_artifact: Option<serde_json::Value>,
    prior_contract: Option<String>,
) -> FanoutChildOutputMerge {
    FanoutChildOutputMerge {
        artifact: completed_artifact.or(prior_artifact),
        contract: prior_contract,
    }
}

fn resolve_fanout_items(
    source: Option<&ItemsSource>,
    artifacts: &HashMap<String, RuntimeArtifact>,
) -> Result<Option<Vec<serde_json::Value>>, WorkflowError> {
    match source {
        None => Ok(None),
        Some(ItemsSource::Literal(items)) => Ok(Some(items.clone())),
        Some(ItemsSource::ArtifactField { node, field }) => {
            let value = artifacts
                .get(node)
                .and_then(|output| output.artifact.as_ref())
                .and_then(serde_json::Value::as_object)
                .and_then(|artifact| artifact.get(field))
                .ok_or_else(|| {
                    WorkflowError::invalid_state(format!(
                        "fanout items source '{node}.{field}' is unavailable"
                    ))
                })?;
            let items = value.as_array().ok_or_else(|| {
                WorkflowError::invalid_state(format!(
                    "fanout items source '{node}.{field}' is not an array"
                ))
            })?;
            Ok(Some(items.clone()))
        }
    }
}

fn next_child_attempts(
    counts: &HashMap<String, u32>,
    child_names: impl IntoIterator<Item = String>,
) -> Vec<u32> {
    let mut counts = counts.clone();
    child_names
        .into_iter()
        .map(|name| {
            let count = counts.entry(name).or_insert(0);
            *count += 1;
            *count
        })
        .collect()
}

pub fn plan_fanout_expansion(
    workflow: &WorkflowDefinition,
    children_entries: &[ChildEntry],
    items_source: Option<&ItemsSource>,
    artifacts: &HashMap<String, RuntimeArtifact>,
    counts: &HashMap<String, u32>,
) -> Result<FanoutExpansionPlan, WorkflowError> {
    let items = resolve_fanout_items(items_source, artifacts)?;
    let coordinates = match items {
        Some(items) => items
            .into_iter()
            .enumerate()
            .flat_map(|(item_index, item)| {
                children_entries
                    .iter()
                    .enumerate()
                    .map(move |(child_index, entry)| {
                        (
                            entry.name.clone(),
                            Some(item.clone()),
                            Some(item_index),
                            child_index,
                        )
                    })
            })
            .collect::<Vec<_>>(),
        None => children_entries
            .iter()
            .enumerate()
            .map(|(child_index, entry)| (entry.name.clone(), None, None, child_index))
            .collect(),
    };
    let attempts = next_child_attempts(
        counts,
        coordinates.iter().map(|(name, _, _, _)| name.clone()),
    );
    let children = coordinates
        .into_iter()
        .zip(attempts)
        .map(|((name, item, item_index, child_index), attempt)| {
            let node = workflow
                .nodes
                .iter()
                .find(|node| node.name == name)
                .ok_or_else(|| {
                    WorkflowError::invalid_state(format!("fanout child node '{name}' is undefined"))
                })?;
            if node.is_composite() {
                return Err(WorkflowError::invalid_state(format!(
                    "fanout child node '{name}' cannot be a composite"
                )));
            }
            Ok(FanoutChildExpansionPlan {
                node_name: name,
                attempt,
                item,
                item_index,
                child_index,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FanoutExpansionPlan { children })
}

pub fn plan_fanout_parent_completion(
    parent_node_name: &str,
    parent_attempt: u32,
    children: &[FanoutChildCompletionInput],
    timestamp: f64,
) -> FanoutParentCompletionPlan {
    let mut combined_tokens = TokenUsage::default();
    for child in children {
        combined_tokens.add(&child.token_usage);
    }

    let parent_artifact = serde_json::Value::Array(
        children
            .iter()
            .map(|child| child.artifact.clone())
            .collect(),
    );

    let fanout_children = children
        .iter()
        .map(|child| FanoutChildSnapshot {
            node_name: child.node_name.clone(),
            session_id: child.session_id.clone(),
            result: child.result.clone(),
            attempt: child.attempt,
            completed_at: child.completed_at,
            artifact: Some(child.artifact.clone()),
            contract: child.contract.clone(),
            state: child.state.clone(),
            failure_kind: child.failure_kind,
            failure_disposition: child.failure_disposition,
        })
        .collect();

    FanoutParentCompletionPlan {
        parent_artifact: RuntimeArtifact {
            node_name: parent_node_name.to_string(),
            attempt: parent_attempt,
            session_id: None,
            result: None,
            artifact: Some(parent_artifact.clone()),
            contract: None,
            token_usage: Some(combined_tokens.clone()),
            completed_at: timestamp,
        },
        history_entry: NodeHistoryEntry {
            node_name: parent_node_name.to_string(),
            completed_at: timestamp,
            result: Some("complete".to_string()),
            session_id: None,
            token_usage: Some(combined_tokens),
            artifact: Some(parent_artifact),
            attempt: parent_attempt,
            fanout_children: Some(fanout_children),
            state: default_node_history_status(),
        },
    }
}

#[cfg(test)]
mod fanout_tests {
    use super::*;

    fn completed_child(node_name: &str, result: Option<&str>) -> FanoutChildCompletionInput {
        FanoutChildCompletionInput {
            node_name: node_name.to_string(),
            session_id: Some(format!("session-{node_name}")),
            result: result.map(str::to_string),
            artifact: serde_json::json!({ "node": node_name }),
            contract: Some("review".to_string()),
            token_usage: TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
            },
            attempt: 1,
            completed_at: 10.0,
            state: default_node_history_status(),
            failure_kind: None,
            failure_disposition: None,
        }
    }

    fn session_node(name: &str) -> crate::domain::workflow::NodeDefinition {
        crate::domain::workflow::NodeDefinition {
            name: name.to_string(),
            kind: crate::domain::workflow::NodeKind::Session(
                crate::domain::workflow::SessionSpec::default(),
            ),
            ..Default::default()
        }
    }

    fn fanout_node(name: &str) -> crate::domain::workflow::NodeDefinition {
        crate::domain::workflow::NodeDefinition {
            name: name.to_string(),
            kind: crate::domain::workflow::NodeKind::Fanout(
                crate::domain::workflow::FanoutSpec::default(),
            ),
            ..Default::default()
        }
    }

    fn workflow_with_nodes(
        nodes: Vec<crate::domain::workflow::NodeDefinition>,
    ) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            nodes,
            ..Default::default()
        }
    }

    #[test]
    fn plan_fanout_expansion_uses_items_major_order_and_indices() {
        let workflow = workflow_with_nodes(vec![session_node("a"), session_node("b")]);
        let children = vec![ChildEntry::reference("a"), ChildEntry::reference("b")];
        let plan = plan_fanout_expansion(
            &workflow,
            &children,
            Some(&ItemsSource::Literal(vec![
                serde_json::json!("first"),
                serde_json::json!("second"),
            ])),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(
            plan.children
                .iter()
                .map(|child| (
                    child.node_name.as_str(),
                    child.item.clone(),
                    child.item_index,
                    child.child_index,
                    child.attempt
                ))
                .collect::<Vec<_>>(),
            vec![
                ("a", Some(serde_json::json!("first")), Some(0), 0, 1),
                ("b", Some(serde_json::json!("first")), Some(0), 1, 1),
                ("a", Some(serde_json::json!("second")), Some(1), 0, 2),
                ("b", Some(serde_json::json!("second")), Some(1), 1, 2),
            ]
        );
    }

    #[test]
    fn plan_fanout_expansion_rejects_fanout_child() {
        let workflow = workflow_with_nodes(vec![fanout_node("nested")]);
        let children = vec![ChildEntry::reference("nested")];

        let err =
            plan_fanout_expansion(&workflow, &children, None, &HashMap::new(), &HashMap::new())
                .unwrap_err();

        assert!(matches!(
            err,
            WorkflowError::InvalidState(message)
                if message == "fanout child node 'nested' cannot be a composite"
        ));
    }

    #[test]
    fn plan_fanout_parent_completion_builds_ordered_artifact_array() {
        let children = vec![
            completed_child("review-a", Some("LGTM")),
            completed_child("review-b", Some("LGTM")),
        ];
        let plan = plan_fanout_parent_completion("fanout-review", 2, &children, 12.0);

        assert_eq!(plan.history_entry.node_name, "fanout-review");
        assert_eq!(plan.history_entry.result.as_deref(), Some("complete"));
        assert_eq!(
            plan.history_entry
                .token_usage
                .as_ref()
                .map(|usage| (usage.input_tokens, usage.output_tokens)),
            Some((4, 6))
        );
        assert_eq!(
            plan.parent_artifact
                .artifact
                .as_ref()
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            plan.parent_artifact.artifact,
            Some(serde_json::json!([
                { "node": "review-a" },
                { "node": "review-b" }
            ]))
        );
    }

    #[test]
    fn merge_fanout_child_completion_output_keeps_prior_submitted_contract() {
        let merge = merge_fanout_child_completion_output(
            None,
            Some(serde_json::json!({ "verdict": "LGTM" })),
            Some("review-contract".to_string()),
        );

        assert_eq!(
            merge.artifact,
            Some(serde_json::json!({ "verdict": "LGTM" }))
        );
        assert_eq!(merge.contract.as_deref(), Some("review-contract"));
    }

    #[test]
    fn merge_fanout_child_completion_output_prefers_completed_artifact() {
        let merge = merge_fanout_child_completion_output(
            Some(serde_json::json!({ "verdict": "NEEDS_FIX" })),
            Some(serde_json::json!({ "verdict": "LGTM" })),
            Some("review-contract".to_string()),
        );

        assert_eq!(
            merge.artifact,
            Some(serde_json::json!({ "verdict": "NEEDS_FIX" }))
        );
        assert_eq!(merge.contract.as_deref(), Some("review-contract"));
    }
}
