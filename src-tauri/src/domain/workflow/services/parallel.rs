//! Pure fanout/collect reduce rules.

use std::collections::HashMap;

use regex::RegexBuilder;

use crate::domain::workflow::value_objects::{
    default_step_entry_state, ChildOutputSnapshot, CollectConfig, FailureDisposition, ItemsSource,
    ParallelAggregate, ReduceStrategy, StepHistoryEntry, StepOutput, TokenUsage,
    WorkflowDefinition, WorkflowStepFailureKind,
};
use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelReduceResult {
    pub result: Option<String>,
    pub structured_output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub struct FanoutChildOutputMerge {
    pub structured_output: Option<serde_json::Value>,
    pub artifact_contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutChildCompletionInput {
    pub node_execution_id: String,
    pub node_name: String,
    pub session_id: Option<String>,
    pub result: Option<String>,
    pub artifact: serde_json::Value,
    pub artifact_contract: Option<String>,
    pub token_usage: TokenUsage,
    pub attempt: u32,
    pub completed_at: f64,
    pub state: String,
    pub failure_kind: Option<WorkflowStepFailureKind>,
    pub failure_disposition: Option<FailureDisposition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FanoutParentTransitionPlan {
    Advance,
    TransitionTo {
        target_node_name: String,
        aggregate_result: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutParentCompletionPlan {
    pub child_node_names: Vec<String>,
    pub parent_step_output: StepOutput,
    pub history_entry: StepHistoryEntry,
    pub transition: FanoutParentTransitionPlan,
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
    completed_structured_output: Option<serde_json::Value>,
    prior_structured_output: Option<serde_json::Value>,
    prior_artifact_contract: Option<String>,
) -> FanoutChildOutputMerge {
    FanoutChildOutputMerge {
        structured_output: completed_structured_output.or(prior_structured_output),
        artifact_contract: prior_artifact_contract,
    }
}

pub fn apply_reduce(
    collect: &CollectConfig,
    step_outputs: &HashMap<String, StepOutput>,
) -> ParallelReduceResult {
    match collect.reduce {
        ReduceStrategy::Last => {
            let last_output = collect
                .from
                .iter()
                .filter_map(|name| step_outputs.get(name.as_str()))
                .max_by(|a, b| {
                    a.completed_at
                        .partial_cmp(&b.completed_at)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            match last_output {
                Some(output) => ParallelReduceResult {
                    result: output.result.clone(),
                    structured_output: output.structured_output.clone(),
                },
                None => ParallelReduceResult {
                    result: None,
                    structured_output: None,
                },
            }
        }
        ReduceStrategy::Concat => {
            let entries = collect_step_output_entries(&collect.from, step_outputs);
            ParallelReduceResult {
                result: None,
                structured_output: non_empty_array(entries),
            }
        }
        ReduceStrategy::Grouped => {
            let mut groups: HashMap<String, Vec<String>> = HashMap::new();
            for step_name in &collect.from {
                if let Some(output) = step_outputs.get(step_name.as_str()) {
                    let key = output
                        .result
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    groups.entry(key).or_default().push(step_name.clone());
                }
            }
            let grouped_json: serde_json::Map<String, serde_json::Value> = groups
                .into_iter()
                .map(|(key, values)| {
                    (
                        key,
                        serde_json::Value::Array(
                            values.into_iter().map(serde_json::Value::String).collect(),
                        ),
                    )
                })
                .collect();
            ParallelReduceResult {
                result: None,
                structured_output: if grouped_json.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(grouped_json))
                },
            }
        }
        ReduceStrategy::AnyNeedsFix => {
            let any_needs_fix = collect.from.iter().any(|step_name| {
                if let Some(output) = step_outputs.get(step_name.as_str()) {
                    matches!(
                        resolve_step_result(output).as_deref(),
                        Some("NEEDS_FIX") | Some("needs_fix")
                    )
                } else {
                    true
                }
            });
            let entries = collect_step_output_entries(&collect.from, step_outputs);
            ParallelReduceResult {
                result: Some(if any_needs_fix { "NEEDS_FIX" } else { "LGTM" }.to_string()),
                structured_output: non_empty_array(entries),
            }
        }
        ReduceStrategy::AllPassed => {
            let all_passed = collect.from.iter().all(|step_name| {
                step_outputs
                    .get(step_name.as_str())
                    .and_then(resolve_step_result)
                    .is_some_and(|result| matches!(result.as_str(), "PASSED" | "passed" | "LGTM"))
            });
            let entries = collect_step_output_entries(&collect.from, step_outputs);
            ParallelReduceResult {
                result: Some(if all_passed { "PASSED" } else { "FAILED" }.to_string()),
                structured_output: non_empty_array(entries),
            }
        }
    }
}

fn resolve_fanout_items(
    source: Option<&ItemsSource>,
    step_outputs: &HashMap<String, StepOutput>,
) -> Result<Option<Vec<serde_json::Value>>, WorkflowError> {
    match source {
        None => Ok(None),
        Some(ItemsSource::Literal(items)) => Ok(Some(items.clone())),
        Some(ItemsSource::ArtifactField { node, field }) => {
            let value = step_outputs
                .get(node)
                .and_then(|output| output.structured_output.as_ref())
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
    child_names: &[String],
    items_source: Option<&ItemsSource>,
    step_outputs: &HashMap<String, StepOutput>,
    counts: &HashMap<String, u32>,
) -> Result<FanoutExpansionPlan, WorkflowError> {
    let items = resolve_fanout_items(items_source, step_outputs)?;
    let coordinates = match items {
        Some(items) => items
            .into_iter()
            .enumerate()
            .flat_map(|(item_index, item)| {
                child_names
                    .iter()
                    .enumerate()
                    .map(move |(child_index, name)| {
                        (
                            name.clone(),
                            Some(item.clone()),
                            Some(item_index),
                            child_index,
                        )
                    })
            })
            .collect::<Vec<_>>(),
        None => child_names
            .iter()
            .enumerate()
            .map(|(child_index, name)| (name.clone(), None, None, child_index))
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
            if node.is_fanout() {
                return Err(WorkflowError::invalid_state(format!(
                    "fanout child node '{name}' cannot be a fanout"
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
    aggregate: Option<&ParallelAggregate>,
    children: &[FanoutChildCompletionInput],
    timestamp: f64,
) -> FanoutParentCompletionPlan {
    let child_node_names: Vec<String> = children
        .iter()
        .map(|child| child.node_name.clone())
        .collect();
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

    let child_outputs = children
        .iter()
        .map(|child| ChildOutputSnapshot {
            step_name: child.node_name.clone(),
            session_id: child.session_id.clone(),
            result: child.result.clone(),
            run_index: child.attempt,
            completed_at: child.completed_at,
            structured_output: Some(child.artifact.clone()),
            artifact_contract: child.artifact_contract.clone(),
            state: child.state.clone(),
            failure_kind: child.failure_kind,
            failure_disposition: child.failure_disposition,
        })
        .collect();

    let (transition, history_result) = if let Some(aggregate) = aggregate {
        let agg_result = evaluate_fanout_aggregate(aggregate, children);
        let aggregate_result = if agg_result { "then" } else { "else" }.to_string();
        let target_node_name = if agg_result {
            aggregate.then.clone()
        } else {
            aggregate.r#else.clone()
        };
        (
            FanoutParentTransitionPlan::TransitionTo {
                target_node_name,
                aggregate_result: aggregate_result.clone(),
            },
            aggregate_result,
        )
    } else {
        (FanoutParentTransitionPlan::Advance, "complete".to_string())
    };

    FanoutParentCompletionPlan {
        child_node_names,
        parent_step_output: StepOutput {
            step_name: parent_node_name.to_string(),
            run_index: parent_attempt,
            session_id: None,
            result: None,
            structured_output: Some(parent_artifact.clone()),
            artifact_contract: None,
            token_usage: Some(combined_tokens.clone()),
            completed_at: timestamp,
        },
        history_entry: StepHistoryEntry {
            step_name: parent_node_name.to_string(),
            completed_at: timestamp,
            result: Some(history_result),
            session_id: None,
            token_usage: Some(combined_tokens),
            structured_output: Some(parent_artifact),
            run_index: parent_attempt,
            child_outputs: Some(child_outputs),
            state: default_step_entry_state(),
        },
        transition,
    }
}

pub fn evaluate_fanout_aggregate(
    aggregate: &ParallelAggregate,
    children: &[FanoutChildCompletionInput],
) -> bool {
    if let Some(pattern) = aggregate.all_match.as_deref() {
        let regex = RegexBuilder::new(pattern).size_limit(1 << 20).build().ok();
        children
            .iter()
            .all(|child| matches_result_pattern(child.result.as_deref(), pattern, &regex))
    } else if let Some(pattern) = aggregate.any_match.as_deref() {
        let regex = RegexBuilder::new(pattern).size_limit(1 << 20).build().ok();
        children
            .iter()
            .any(|child| matches_result_pattern(child.result.as_deref(), pattern, &regex))
    } else {
        true
    }
}

fn matches_result_pattern(
    result: Option<&str>,
    pattern: &str,
    regex: &Option<regex::Regex>,
) -> bool {
    let Some(result) = result else {
        return false;
    };
    if let Some(regex) = regex {
        regex.is_match(result)
    } else {
        result.contains(pattern)
    }
}

fn non_empty_array(entries: Vec<serde_json::Value>) -> Option<serde_json::Value> {
    if entries.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(entries))
    }
}

pub fn collect_step_output_entries(
    from: &[String],
    step_outputs: &HashMap<String, StepOutput>,
) -> Vec<serde_json::Value> {
    let mut entries = Vec::new();
    for step_name in from {
        if let Some(output) = step_outputs.get(step_name.as_str()) {
            if let Some(structured_output) = &output.structured_output {
                entries.push(serde_json::json!({
                    "stepName": step_name,
                    "output": structured_output,
                }));
            }
        }
    }
    entries
}

pub fn resolve_step_result(output: &StepOutput) -> Option<String> {
    if let Some(structured_output) = &output.structured_output {
        if let Some(verdict) = structured_output
            .get("verdict")
            .and_then(|value| value.as_str())
        {
            return Some(verdict.to_string());
        }
        if let Some(status) = structured_output
            .get("status")
            .and_then(|value| value.as_str())
        {
            return Some(status.to_string());
        }
    }
    output.result.clone()
}

#[cfg(test)]
pub fn evaluate_aggregate(
    aggregate: &ParallelAggregate,
    step_outputs: &HashMap<String, StepOutput>,
    child_step_names: &[String],
) -> bool {
    let child_outputs: Vec<&StepOutput> = child_step_names
        .iter()
        .filter_map(|name| step_outputs.get(name))
        .collect();

    if let Some(pattern) = aggregate.all_match.as_deref() {
        if child_outputs.len() != child_step_names.len() {
            return false;
        }
        let regex = RegexBuilder::new(pattern).size_limit(1 << 20).build().ok();
        child_outputs
            .iter()
            .all(|output| matches_aggregate_pattern(output, pattern, &regex))
    } else if let Some(pattern) = aggregate.any_match.as_deref() {
        let regex = RegexBuilder::new(pattern).size_limit(1 << 20).build().ok();
        child_outputs
            .iter()
            .any(|output| matches_aggregate_pattern(output, pattern, &regex))
    } else {
        true
    }
}

#[cfg(test)]
fn matches_aggregate_pattern(
    output: &StepOutput,
    pattern: &str,
    regex: &Option<regex::Regex>,
) -> bool {
    let Some(result) = output.result.as_ref() else {
        return false;
    };
    if let Some(regex) = regex {
        regex.is_match(result)
    } else {
        result.contains(pattern)
    }
}

#[cfg(test)]
mod parallel_tests {
    use super::*;

    fn output(step_name: &str, result: Option<&str>, completed_at: f64) -> StepOutput {
        StepOutput {
            step_name: step_name.to_string(),
            run_index: 0,
            session_id: None,
            result: result.map(str::to_string),
            structured_output: None,
            artifact_contract: None,
            token_usage: None,
            completed_at,
        }
    }

    fn completed_child(step_name: &str, result: Option<&str>) -> FanoutChildCompletionInput {
        FanoutChildCompletionInput {
            node_execution_id: format!("execution-{step_name}"),
            node_name: step_name.to_string(),
            session_id: Some(format!("session-{step_name}")),
            result: result.map(str::to_string),
            artifact: serde_json::json!({ "node": step_name }),
            artifact_contract: Some("review".to_string()),
            token_usage: TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
            },
            attempt: 1,
            completed_at: 10.0,
            state: default_step_entry_state(),
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
        let children = vec!["a".to_string(), "b".to_string()];
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
        let children = vec!["nested".to_string()];

        let err =
            plan_fanout_expansion(&workflow, &children, None, &HashMap::new(), &HashMap::new())
                .unwrap_err();

        assert!(matches!(
            err,
            WorkflowError::InvalidState(message)
                if message == "fanout child node 'nested' cannot be a fanout"
        ));
    }

    #[test]
    fn test_reduce_last_最新completed_atの出力を選ぶ() {
        let collect = CollectConfig {
            from: vec!["a".to_string(), "b".to_string()],
            reduce: ReduceStrategy::Last,
        };
        let outputs = HashMap::from([
            ("a".to_string(), output("a", Some("old"), 1.0)),
            ("b".to_string(), output("b", Some("new"), 2.0)),
        ]);
        assert_eq!(
            apply_reduce(&collect, &outputs).result.as_deref(),
            Some("new")
        );
    }

    #[test]
    fn plan_fanout_parent_completion_builds_ordered_array_and_then_transition() {
        let aggregate = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "ship".to_string(),
            r#else: "fix".to_string(),
        };
        let children = vec![
            completed_child("review-a", Some("LGTM")),
            completed_child("review-b", Some("LGTM")),
        ];
        let plan =
            plan_fanout_parent_completion("parallel-review", 2, Some(&aggregate), &children, 12.0);

        assert_eq!(
            plan.transition,
            FanoutParentTransitionPlan::TransitionTo {
                target_node_name: "ship".to_string(),
                aggregate_result: "then".to_string()
            }
        );
        assert_eq!(plan.history_entry.step_name, "parallel-review");
        assert_eq!(plan.history_entry.result.as_deref(), Some("then"));
        assert_eq!(
            plan.history_entry
                .token_usage
                .as_ref()
                .map(|usage| (usage.input_tokens, usage.output_tokens)),
            Some((4, 6))
        );
        assert_eq!(
            plan.parent_step_output
                .structured_output
                .as_ref()
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            plan.parent_step_output.structured_output,
            Some(serde_json::json!([
                { "node": "review-a" },
                { "node": "review-b" }
            ]))
        );
    }

    #[test]
    fn plan_fanout_parent_completion_without_aggregate_advances() {
        let children = vec![completed_child("review-a", Some("LGTM"))];
        let plan = plan_fanout_parent_completion("parallel-review", 1, None, &children, 12.0);

        assert_eq!(plan.transition, FanoutParentTransitionPlan::Advance);
        assert_eq!(plan.history_entry.result.as_deref(), Some("complete"));
    }

    #[test]
    fn merge_fanout_child_completion_output_keeps_prior_submitted_contract() {
        let merge = merge_fanout_child_completion_output(
            None,
            Some(serde_json::json!({ "verdict": "LGTM" })),
            Some("review-contract".to_string()),
        );

        assert_eq!(
            merge.structured_output,
            Some(serde_json::json!({ "verdict": "LGTM" }))
        );
        assert_eq!(merge.artifact_contract.as_deref(), Some("review-contract"));
    }

    #[test]
    fn merge_fanout_child_completion_output_prefers_completed_structured_output() {
        let merge = merge_fanout_child_completion_output(
            Some(serde_json::json!({ "verdict": "NEEDS_FIX" })),
            Some(serde_json::json!({ "verdict": "LGTM" })),
            Some("review-contract".to_string()),
        );

        assert_eq!(
            merge.structured_output,
            Some(serde_json::json!({ "verdict": "NEEDS_FIX" }))
        );
        assert_eq!(merge.artifact_contract.as_deref(), Some("review-contract"));
    }

    #[test]
    fn test_reduce_any_needs_fix_未完了stepはneeds_fix扱い() {
        let collect = CollectConfig {
            from: vec!["a".to_string(), "missing".to_string()],
            reduce: ReduceStrategy::AnyNeedsFix,
        };
        let outputs = HashMap::from([("a".to_string(), output("a", Some("LGTM"), 1.0))]);
        assert_eq!(
            apply_reduce(&collect, &outputs).result.as_deref(),
            Some("NEEDS_FIX")
        );
    }

    #[test]
    fn test_reduce_any_needs_fix_resultなしの完了出力はneeds_fix扱いしない() {
        let collect = CollectConfig {
            from: vec!["a".to_string()],
            reduce: ReduceStrategy::AnyNeedsFix,
        };
        let outputs = HashMap::from([("a".to_string(), output("a", None, 1.0))]);
        assert_eq!(
            apply_reduce(&collect, &outputs).result.as_deref(),
            Some("LGTM")
        );
    }

    #[test]
    fn test_evaluate_aggregate_all_match_requires_all_child_outputs() {
        let aggregate = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "fix".to_string(),
        };
        let outputs = HashMap::from([("a".to_string(), output("a", Some("LGTM"), 1.0))]);
        let children = vec!["a".to_string(), "missing".to_string()];

        assert!(!evaluate_aggregate(&aggregate, &outputs, &children));
    }

    #[test]
    fn test_evaluate_aggregate_invalid_regex_falls_back_to_contains() {
        let aggregate = ParallelAggregate {
            all_match: Some("[invalid(regex".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "fix".to_string(),
        };
        let outputs = HashMap::from([("a".to_string(), output("a", Some("[invalid(regex"), 1.0))]);
        let children = vec!["a".to_string()];

        assert!(evaluate_aggregate(&aggregate, &outputs, &children));
    }
}
