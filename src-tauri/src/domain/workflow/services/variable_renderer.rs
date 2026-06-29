//! Pure template and prompt variable rendering.

use std::collections::HashMap;
use std::path::Path;

use crate::domain::workflow::value_objects::{NodeDefinition, StepHistoryEntry, StepOutput};
#[cfg(test)]
use crate::domain::workflow::STEP_STATE_COMPLETED;

pub const SYSTEM_TEMPLATE_VARIABLES: &[&str] = &["project_name", "task"];
pub const PATH_ALIAS_NAMESPACE: &str = "path_alias";
pub const VARS_NAMESPACE: &str = "vars";
pub const KNOWN_NAMESPACES: &[&str] = &[PATH_ALIAS_NAMESPACE, VARS_NAMESPACE];
pub const KNOWN_PATH_ALIAS_KEYS: &[&str] = &["releash"];

pub fn extract_template_variables(content: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut start = 0;
    while let Some(open) = content[start..].find("{{") {
        let abs_open = start + open + 2;
        if let Some(close) = content[abs_open..].find("}}") {
            let var_name = content[abs_open..abs_open + close].trim();
            if !var_name.is_empty() {
                vars.push(var_name.to_string());
            }
            start = abs_open + close + 2;
        } else {
            break;
        }
    }
    vars
}

pub fn split_namespaced(var: &str) -> Option<(&str, &str)> {
    var.split_once('.')
}

pub fn find_undefined_template_variables(content: &str) -> Vec<String> {
    extract_template_variables(content)
        .into_iter()
        .filter(|v| {
            if let Some((ns, key)) = split_namespaced(v) {
                if ns == PATH_ALIAS_NAMESPACE {
                    !KNOWN_PATH_ALIAS_KEYS.contains(&key)
                } else if ns == VARS_NAMESPACE {
                    false
                } else {
                    !KNOWN_NAMESPACES.contains(&ns)
                }
            } else {
                !SYSTEM_TEMPLATE_VARIABLES.contains(&v.as_str())
            }
        })
        .collect()
}

pub fn find_undefined_workflow_variable_refs(
    content: &str,
    defined: &HashMap<String, String>,
) -> Vec<String> {
    extract_template_variables(content)
        .into_iter()
        .filter_map(|v| {
            split_namespaced(&v).and_then(|(ns, key)| {
                if ns == VARS_NAMESPACE && !defined.contains_key(key) {
                    Some(key.to_string())
                } else {
                    None
                }
            })
        })
        .collect()
}

pub fn render_template_variables(content: &str, values: &HashMap<String, String>) -> String {
    replace_template_refs(content, |inner| values.get(inner).cloned())
}

pub fn render_path_alias_variables_with_name(content: &str, releash_alias_name: &str) -> String {
    replace_template_refs(content, |inner| match inner {
        "path_alias.releash" => Some(releash_alias_name.to_string()),
        _ => None,
    })
}

pub fn render_workflow_variables(content: &str, variables: &HashMap<String, String>) -> String {
    replace_template_refs(content, |inner| {
        inner
            .strip_prefix("vars.")
            .and_then(|name| variables.get(name).cloned())
    })
}

pub fn render_facet_variables(content: &str, worktree_path: &str, task: Option<&str>) -> String {
    let project_name = Path::new(worktree_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    let result = content.replace("{{project_name}}", project_name);
    match task {
        Some(task) => result.replace("{{task}}", task),
        None => result.replace("{{task}}", ""),
    }
}

pub fn render_submit_command_variables(content: &str, run_id: &str, step_name: &str) -> String {
    content
        .replace("{{run_id}}", run_id)
        .replace("{{step_name}}", step_name)
}

pub fn inject_step_outputs(
    prompt: &str,
    node: &NodeDefinition,
    step_outputs: &HashMap<String, StepOutput>,
    step_history: &[StepHistoryEntry],
    workflow_variables: &HashMap<String, String>,
) -> String {
    let mut result = prompt.to_string();

    if node.pass_previous_response == Some(true) {
        if let Some(last_entry) = step_history.last() {
            if let Some(output) = step_outputs.get(&last_entry.step_name) {
                let text = format_step_output_block(output);
                append_step_output_block(&mut result, &last_entry.step_name, &text);
            }
        }
    }

    if let Some(refs) = &node.pass_output_from {
        for step_name in refs {
            let text = match step_outputs.get(step_name.as_str()) {
                Some(output) => format_step_output_block(output),
                None => "(not yet completed)".to_string(),
            };
            append_step_output_block(&mut result, step_name, &text);
        }
    }

    append_workflow_variables_block(&mut result, workflow_variables);
    result
}

pub fn task_block(task: Option<&str>, allow_task_injection: bool) -> Option<String> {
    if !allow_task_injection {
        return None;
    }
    let task = task?;
    if task.is_empty() {
        return None;
    }
    Some(format!("\n\n<task>\n{}\n</task>", escape_xml_text(task)))
}

pub fn escape_xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            other => out.push(other),
        }
    }
    out
}

pub fn format_step_output_block(output: &StepOutput) -> String {
    match &output.structured_output {
        Some(json) => serde_json::to_string_pretty(json).unwrap_or_else(|_| "{}".to_string()),
        None => "(no structured output)".to_string(),
    }
}

pub fn workflow_variables_block(workflow_variables: &HashMap<String, String>) -> Option<String> {
    let filtered_variables: HashMap<_, _> = workflow_variables
        .iter()
        .filter(|(key, _)| !key.starts_with("approved_fix_policy"))
        .collect();
    if filtered_variables.is_empty() {
        return None;
    }
    let vars_json = serde_json::to_string_pretty(&filtered_variables).unwrap_or_default();
    Some(format!(
        "\n\n<workflow_variables>\n{}\n</workflow_variables>",
        vars_json
    ))
}

fn append_step_output_block(result: &mut String, step_name: &str, text: &str) {
    result.push_str(&format!(
        "\n\n<step_output name=\"{}\">\n{}\n</step_output>",
        step_name, text
    ));
}

fn append_workflow_variables_block(
    result: &mut String,
    workflow_variables: &HashMap<String, String>,
) {
    if let Some(block) = workflow_variables_block(workflow_variables) {
        result.push_str(&block);
    }
}

fn replace_template_refs(content: &str, mut resolve: impl FnMut(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while !rest.is_empty() {
        let Some(open_idx) = rest.find("{{") else {
            result.push_str(rest);
            break;
        };
        result.push_str(&rest[..open_idx]);
        let after_open = &rest[open_idx + 2..];
        let Some(close_idx) = after_open.find("}}") else {
            result.push_str("{{");
            result.push_str(after_open);
            break;
        };
        let raw_inner = &after_open[..close_idx];
        let inner = raw_inner.trim();
        match resolve(inner) {
            Some(value) => result.push_str(&value),
            None => {
                result.push_str("{{");
                result.push_str(raw_inner);
                result.push_str("}}");
            }
        }
        rest = &after_open[close_idx + 2..];
    }
    result
}

#[cfg(test)]
mod variable_renderer_tests {
    use super::*;

    #[test]
    fn test_template_variables_空白付き参照も置換する() {
        let values = HashMap::from([("task".to_string(), "write spec".to_string())]);
        assert_eq!(
            render_template_variables("do {{ task }}", &values),
            "do write spec"
        );
    }

    #[test]
    fn test_workflow_variables_二次展開しない() {
        let values = HashMap::from([
            ("a".to_string(), "{{vars.b}}".to_string()),
            ("b".to_string(), "B".to_string()),
        ]);
        assert_eq!(
            render_workflow_variables("{{vars.a}}", &values),
            "{{vars.b}}"
        );
    }

    #[test]
    fn test_undefined_template_variables_既知namespaceを許可する() {
        let content = "{{project_name}} {{vars.x}} {{path_alias.releash}} {{unknown_top}}";
        assert_eq!(
            find_undefined_template_variables(content),
            vec!["unknown_top".to_string()]
        );
    }

    #[test]
    fn test_undefined_template_variables_未知namespaceを検出する() {
        assert_eq!(
            find_undefined_template_variables("{{not_a_namespace.key}}"),
            vec!["not_a_namespace.key".to_string()]
        );
    }

    #[test]
    fn test_undefined_template_variables_未知path_alias_keyを検出する() {
        assert_eq!(
            find_undefined_template_variables("{{path_alias.relase}} {{path_alias.releash}}"),
            vec!["path_alias.relase".to_string()]
        );
    }

    #[test]
    fn test_undefined_workflow_variable_refs_宣言外varsだけ返す() {
        let values = HashMap::from([("known".to_string(), "value".to_string())]);
        let mut undefined = find_undefined_workflow_variable_refs(
            "{{vars.known}} {{vars.unknown}} {{vars.another_missing}}",
            &values,
        );
        undefined.sort();
        assert_eq!(undefined, vec!["another_missing", "unknown"]);
    }

    #[test]
    fn test_facet_variables_project_nameとtaskを展開する() {
        assert_eq!(
            render_facet_variables("{{project_name}}: {{task}}", "/tmp/repo", Some("fix")),
            "repo: fix"
        );
    }

    #[test]
    fn test_task_block_信頼境界外文字をxmlエスケープする() {
        assert_eq!(
            task_block(Some("<tag>&value</tag>"), true).unwrap(),
            "\n\n<task>\n&lt;tag&gt;&amp;value&lt;/tag&gt;\n</task>"
        );
        assert!(task_block(Some("ignored"), false).is_none());
    }

    #[test]
    fn test_inject_step_outputs_pass_previous_responseとworkflow_variablesを追加する() {
        let mut node = NodeDefinition {
            name: "implement".to_string(),
            pass_previous_response: Some(true),
            ..Default::default()
        };
        node.pass_output_from = Some(vec!["missing".to_string()]);
        let step_outputs = HashMap::from([(
            "plan".to_string(),
            StepOutput {
                step_name: "plan".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({"summary": "ready"})),
                output_contract: None,
                token_usage: None,
                completed_at: 1.0,
            },
        )]);
        let history = vec![StepHistoryEntry {
            step_name: "plan".to_string(),
            completed_at: 1.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: STEP_STATE_COMPLETED.to_string(),
        }];
        let variables = HashMap::from([
            ("spec_dir".to_string(), "docs/specs/x".to_string()),
            (
                "approved_fix_policy.secret".to_string(),
                "hidden".to_string(),
            ),
        ]);

        let rendered = inject_step_outputs("Prompt", &node, &step_outputs, &history, &variables);

        assert!(rendered.contains("<step_output name=\"plan\">"));
        assert!(rendered.contains("\"summary\": \"ready\""));
        assert!(rendered.contains("<step_output name=\"missing\">"));
        assert!(rendered.contains("(not yet completed)"));
        assert!(rendered.contains("spec_dir"));
        assert!(!rendered.contains("approved_fix_policy"));
    }
}
