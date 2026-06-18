//! Pure parallel/collect reduce rules.

use std::collections::HashMap;

use regex::RegexBuilder;

use crate::domain::workflow::value_objects::{
    CollectConfig, ParallelAggregate, ReduceStrategy, StepOutput,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelReduceResult {
    pub result: Option<String>,
    pub structured_output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelChildOutputMerge {
    pub structured_output: Option<serde_json::Value>,
    pub output_contract: Option<String>,
}

pub fn merge_parallel_child_completion_output(
    completed_structured_output: Option<serde_json::Value>,
    prior_structured_output: Option<serde_json::Value>,
    prior_output_contract: Option<String>,
) -> ParallelChildOutputMerge {
    ParallelChildOutputMerge {
        structured_output: completed_structured_output.or(prior_structured_output),
        output_contract: prior_output_contract,
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
            output_contract: None,
            token_usage: None,
            completed_at,
        }
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
    fn merge_parallel_child_completion_output_keeps_prior_submitted_contract() {
        let merge = merge_parallel_child_completion_output(
            None,
            Some(serde_json::json!({ "verdict": "LGTM" })),
            Some("review-contract".to_string()),
        );

        assert_eq!(
            merge.structured_output,
            Some(serde_json::json!({ "verdict": "LGTM" }))
        );
        assert_eq!(merge.output_contract.as_deref(), Some("review-contract"));
    }

    #[test]
    fn merge_parallel_child_completion_output_prefers_completed_structured_output() {
        let merge = merge_parallel_child_completion_output(
            Some(serde_json::json!({ "verdict": "NEEDS_FIX" })),
            Some(serde_json::json!({ "verdict": "LGTM" })),
            Some("review-contract".to_string()),
        );

        assert_eq!(
            merge.structured_output,
            Some(serde_json::json!({ "verdict": "NEEDS_FIX" }))
        );
        assert_eq!(merge.output_contract.as_deref(), Some("review-contract"));
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
