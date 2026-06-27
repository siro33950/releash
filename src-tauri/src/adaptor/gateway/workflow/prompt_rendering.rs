use std::collections::HashMap;

use crate::adaptor::gateway::workflow::domain_mapping::{
    node_definition_to_domain, step_history_entries_to_domain, step_output_to_domain,
    step_outputs_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::schema::{ChildNodeDefinition, NodeDefinition};
use crate::adaptor::gateway::workflow::state::{StepHistoryEntry, StepOutput};
use crate::domain::workflow::services::variable_renderer;

/// ファセット内容中のテンプレート変数を展開する。
pub(crate) fn render_facet_variables(
    content: &str,
    worktree_path: &str,
    task: Option<&str>,
) -> String {
    variable_renderer::render_facet_variables(content, worktree_path, task)
}

/// `releash workflow output submit` の CLI 例を実 run_id / step_name に展開する。
pub(crate) fn render_submit_command_variables(
    content: &str,
    run_id: &str,
    step_name: &str,
) -> String {
    variable_renderer::render_submit_command_variables(content, run_id, step_name)
}

/// facet 本文に対し、起動環境別 alias と workflow 定義変数を namespace 展開する。
pub(crate) fn render_namespaced_variables(
    content: &str,
    workflow_declared_variables: &HashMap<String, String>,
) -> String {
    let releash_alias = crate::infrastructure::platform::path_aliases::alias_name_for_profile(
        crate::infrastructure::platform::path_aliases::BuildProfile::current(),
    );
    let rendered_alias =
        variable_renderer::render_path_alias_variables_with_name(content, releash_alias);
    variable_renderer::render_workflow_variables(&rendered_alias, workflow_declared_variables)
}

fn render_workflow_instruction(
    instruction: &str,
    run_id: &str,
    step_name: &str,
    worktree_path: &str,
    task: Option<&str>,
    workflow_declared_variables: &HashMap<String, String>,
) -> Option<String> {
    let rendered = render_facet_variables(instruction, worktree_path, task);
    let rendered = render_submit_command_variables(&rendered, run_id, step_name);
    let rendered = render_namespaced_variables(&rendered, workflow_declared_variables);
    let rendered = rendered.trim().to_string();
    (!rendered.is_empty()).then_some(rendered)
}

pub(crate) fn render_step_workflow_instruction(
    step: &NodeDefinition,
    run_id: &str,
    worktree_path: &str,
    task: Option<&str>,
    workflow_declared_variables: &HashMap<String, String>,
) -> Option<String> {
    render_workflow_instruction(
        step.resolved_facets.instruction.as_ref()?,
        run_id,
        &step.name,
        worktree_path,
        task,
        workflow_declared_variables,
    )
}

pub(crate) fn render_child_workflow_instruction(
    step: &ChildNodeDefinition,
    run_id: &str,
    worktree_path: &str,
    task: Option<&str>,
    workflow_declared_variables: &HashMap<String, String>,
) -> Option<String> {
    render_workflow_instruction(
        step.resolved_facets.instruction.as_ref()?,
        run_id,
        &step.name,
        worktree_path,
        task,
        workflow_declared_variables,
    )
}

/// ステップの出力をプロンプトにコンテキストブロックとして注入する。
pub(crate) fn inject_step_outputs(
    prompt: &str,
    step: &NodeDefinition,
    step_outputs: &HashMap<String, StepOutput>,
    step_history: &[StepHistoryEntry],
    workflow_variables: &HashMap<String, String>,
) -> String {
    let step = node_definition_to_domain(step);
    let step_outputs = step_outputs_to_domain(step_outputs);
    let step_history = step_history_entries_to_domain(step_history);
    variable_renderer::inject_step_outputs(
        prompt,
        &step,
        &step_outputs,
        &step_history,
        workflow_variables,
    )
}

pub(crate) fn append_task_block(
    prompt: &mut String,
    task: Option<&str>,
    allow_task_injection: bool,
) {
    if let Some(block) = variable_renderer::task_block(task, allow_task_injection) {
        prompt.push_str(&block);
    }
}

pub(crate) fn append_output_contract_completion_action(
    prompt: &mut String,
    output_contract: Option<&str>,
    run_id: &str,
    step_name: &str,
    workflow_declared_variables: &HashMap<String, String>,
) {
    let Some(contract) = output_contract else {
        return;
    };
    let action =
        crate::adaptor::gateway::workflow::facet::output_contract_completion_action(contract);
    let action = render_submit_command_variables(&action, run_id, step_name);
    let action = render_namespaced_variables(&action, workflow_declared_variables);
    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }
    prompt.push_str(&action);
}

pub(crate) fn format_step_output_block(output: &StepOutput) -> String {
    let output = step_output_to_domain(output);
    variable_renderer::format_step_output_block(&output)
}

pub(crate) fn append_workflow_variables_block(
    result: &mut String,
    workflow_variables: &HashMap<String, String>,
) {
    if let Some(block) = variable_renderer::workflow_variables_block(workflow_variables) {
        result.push_str(&block);
    }
}

/// ファセット合成パイプライン: compose → 変数展開 → step output 注入。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_step_prompt(
    step: &NodeDefinition,
    run_id: &str,
    worktree_path: &str,
    task: Option<&str>,
    step_outputs: &HashMap<String, StepOutput>,
    step_history: &[StepHistoryEntry],
    workflow_variables: &HashMap<String, String>,
    workflow_declared_variables: &HashMap<String, String>,
) -> Result<(Option<String>, String), WorkflowEngineError> {
    if !step.has_facet_refs() {
        if let Some(ref inline) = step.inline_prompt {
            let rendered = render_facet_variables(inline, worktree_path, task);
            let rendered = render_submit_command_variables(&rendered, run_id, &step.name);
            let rendered = render_namespaced_variables(&rendered, workflow_declared_variables);
            let prompt = inject_step_outputs(
                &rendered,
                step,
                step_outputs,
                step_history,
                workflow_variables,
            );
            return Ok((None, prompt));
        }
        return Err(WorkflowEngineError::InvalidWorkflow(format!(
            "Step '{}' has no facet refs and no inline_prompt.",
            step.name
        )));
    }

    if step.resolved_facets.is_empty() {
        return Err(WorkflowEngineError::InvalidWorkflow(format!(
            "Step '{}' has unresolved facet refs (workflow must go through load pipeline)",
            step.name
        )));
    }
    let composed = crate::adaptor::gateway::workflow::facet::compose_facets(step);
    let system_prompt = composed.system_prompt.map(|s| {
        let s = render_facet_variables(&s, worktree_path, task);
        let s = render_submit_command_variables(&s, run_id, &step.name);
        render_namespaced_variables(&s, workflow_declared_variables)
    });
    let rendered_user = {
        let s = render_facet_variables(&composed.user_message, worktree_path, task);
        let s = render_submit_command_variables(&s, run_id, &step.name);
        render_namespaced_variables(&s, workflow_declared_variables)
    };
    let mut prompt = inject_step_outputs(
        &rendered_user,
        step,
        step_outputs,
        step_history,
        workflow_variables,
    );
    let allow_task = step.input_contracts.as_ref().is_some_and(|v| !v.is_empty());
    append_task_block(&mut prompt, task, allow_task);
    append_output_contract_completion_action(
        &mut prompt,
        step.output_contract.as_deref(),
        run_id,
        &step.name,
        workflow_declared_variables,
    );
    Ok((system_prompt, prompt))
}

/// 並列子ステップ用のプロンプトを構築する。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_parallel_step_prompt(
    step: &ChildNodeDefinition,
    run_id: &str,
    worktree_path: &str,
    task: Option<&str>,
    step_outputs: &HashMap<String, StepOutput>,
    pass_previous_response: bool,
    pass_output_from: Option<&[String]>,
    workflow_variables: &HashMap<String, String>,
    workflow_declared_variables: &HashMap<String, String>,
) -> Result<(Option<String>, String), WorkflowEngineError> {
    if step.has_facet_refs() && step.resolved_facets.is_empty() {
        return Err(WorkflowEngineError::InvalidWorkflow(format!(
            "Parallel child '{}' has unresolved facet refs (workflow must go through load pipeline)",
            step.name
        )));
    }
    let composed = crate::adaptor::gateway::workflow::facet::compose_child_facets(step);

    let system_prompt = composed.system_prompt.map(|s| {
        let s = render_facet_variables(&s, worktree_path, task);
        let s = render_submit_command_variables(&s, run_id, &step.name);
        render_namespaced_variables(&s, workflow_declared_variables)
    });
    let mut user_message = render_facet_variables(&composed.user_message, worktree_path, task);
    user_message = render_submit_command_variables(&user_message, run_id, &step.name);
    user_message = render_namespaced_variables(&user_message, workflow_declared_variables);

    if let Some(from_steps) = pass_output_from {
        let mut injections = Vec::new();
        for step_name in from_steps {
            if let Some(output) = step_outputs.get(step_name) {
                let text = format_step_output_block(output);
                injections.push(format!(
                    "<step_output name=\"{step_name}\">\n{text}\n</step_output>",
                ));
            }
        }
        if !injections.is_empty() {
            user_message = format!("{}\n\n{}", injections.join("\n\n"), user_message);
        }
    } else if pass_previous_response {
        if let Some(last_output) = step_outputs.values().max_by(|a, b| {
            a.completed_at
                .partial_cmp(&b.completed_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            let text = format_step_output_block(last_output);
            user_message = format!(
                "<step_output name=\"{}\">\n{}\n</step_output>\n\n{}",
                last_output.step_name, text, user_message
            );
        }
    }

    append_workflow_variables_block(&mut user_message, workflow_variables);
    let allow_task = step.input_contracts.as_ref().is_some_and(|v| !v.is_empty());
    append_task_block(&mut user_message, task, allow_task);
    append_output_contract_completion_action(
        &mut user_message,
        step.output_contract.as_deref(),
        run_id,
        &step.name,
        workflow_declared_variables,
    );

    Ok((system_prompt, user_message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::NodeType;

    fn make_test_step(name: &str, node_type: NodeType, instruction: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            node_type,
            instruction: Some(instruction.to_string()),
            ..NodeDefinition::default()
        }
    }

    fn make_step_output(step_name: &str, output_text: &str, result: Option<&str>) -> StepOutput {
        StepOutput {
            step_name: step_name.to_string(),
            run_index: 0,
            session_id: None,
            result: result.map(str::to_string),
            structured_output: Some(serde_json::json!({ "text": output_text })),
            output_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        }
    }

    fn history_entry(step_name: &str) -> StepHistoryEntry {
        StepHistoryEntry {
            step_name: step_name.to_string(),
            completed_at: 1000.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: 0,
            child_outputs: None,
            state: crate::adaptor::gateway::workflow::state::default_step_entry_state(),
        }
    }

    #[test]
    fn render_facet_variables_replaces_task_and_project_name() {
        let content = "Task: {{task}}\nProject: {{project_name}}";

        let result = render_facet_variables(content, "/home/user/my-project", Some("Fix bug"));

        assert_eq!(result, "Task: Fix bug\nProject: my-project");
    }

    #[test]
    fn render_facet_variables_task_none_replaces_with_empty() {
        let content = "Do: {{task}}";

        let result = render_facet_variables(content, "/home/user/proj", None);

        assert_eq!(result, "Do: ");
    }

    #[test]
    fn render_facet_variables_no_variables_unchanged() {
        let content = "No variables here";

        let result = render_facet_variables(content, "/home/user/proj", Some("task"));

        assert_eq!(result, "No variables here");
    }

    #[test]
    fn inject_step_outputs_pass_previous_response() {
        let mut step = make_test_step("step_b", NodeType::Agent, "Do B");
        step.pass_previous_response = Some(true);
        let outputs = HashMap::from([(
            "step_a".to_string(),
            make_step_output("step_a", "output from A", None),
        )]);
        let history = vec![history_entry("step_a")];

        let result = inject_step_outputs("Do B", &step, &outputs, &history, &HashMap::new());

        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output from A"));
    }

    #[test]
    fn inject_step_outputs_no_pass_previous_response() {
        let step = make_test_step("step_b", NodeType::Agent, "Do B");

        let result = inject_step_outputs("Do B", &step, &HashMap::new(), &[], &HashMap::new());

        assert_eq!(result, "Do B");
    }

    #[test]
    fn inject_step_outputs_pass_output_from_single() {
        let mut step = make_test_step("step_c", NodeType::Agent, "Do C");
        step.pass_output_from = Some(vec!["step_a".to_string()]);
        let outputs = HashMap::from([(
            "step_a".to_string(),
            make_step_output("step_a", "output A", None),
        )]);

        let result = inject_step_outputs("Do C", &step, &outputs, &[], &HashMap::new());

        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output A"));
    }

    #[test]
    fn reject_comment_accessible_via_pass_output_from() {
        let mut step = make_test_step("fix", NodeType::Agent, "Fix issues");
        step.pass_output_from = Some(vec!["review".to_string()]);
        let outputs = HashMap::from([(
            "review".to_string(),
            make_step_output("review", "Fix the naming convention", Some("reject")),
        )]);

        let result = inject_step_outputs("Fix issues", &step, &outputs, &[], &HashMap::new());

        assert!(result.contains("<step_output name=\"review\">"));
        assert!(result.contains("Fix the naming convention"));
    }

    #[test]
    fn inject_step_outputs_pass_output_from_multiple() {
        let mut step = make_test_step("step_c", NodeType::Agent, "Do C");
        step.pass_output_from = Some(vec!["step_a".to_string(), "step_b".to_string()]);
        let outputs = HashMap::from([
            (
                "step_a".to_string(),
                make_step_output("step_a", "output A", None),
            ),
            (
                "step_b".to_string(),
                make_step_output("step_b", "output B", None),
            ),
        ]);

        let result = inject_step_outputs("Do C", &step, &outputs, &[], &HashMap::new());

        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output A"));
        assert!(result.contains("<step_output name=\"step_b\">"));
        assert!(result.contains("output B"));
    }

    #[test]
    fn inject_step_outputs_pass_previous_response_no_output_injects_nothing() {
        let mut step = make_test_step("step_b", NodeType::Agent, "Do B");
        step.pass_previous_response = Some(true);
        let history = vec![history_entry("step_a")];

        let result = inject_step_outputs("Do B", &step, &HashMap::new(), &history, &HashMap::new());

        assert_eq!(result, "Do B");
    }

    #[test]
    fn inject_step_outputs_missing_step_shows_not_completed() {
        let mut step = make_test_step("step_b", NodeType::Agent, "Do B");
        step.pass_output_from = Some(vec!["step_a".to_string()]);

        let result = inject_step_outputs("Do B", &step, &HashMap::new(), &[], &HashMap::new());

        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("(not yet completed)"));
    }

    #[test]
    fn inject_step_outputs_workflow_variables_injected() {
        let step = make_test_step("step_b", NodeType::Agent, "Do B");
        let workflow_variables = HashMap::from([(
            "spec_dir".to_string(),
            "docs/spec/issues-909.md".to_string(),
        )]);

        let result = inject_step_outputs("Do B", &step, &HashMap::new(), &[], &workflow_variables);

        assert!(result.contains("<workflow_variables>"));
        assert!(result.contains("spec_dir"));
        assert!(result.contains("docs/spec/issues-909.md"));
    }

    #[test]
    fn inject_step_outputs_empty_workflow_variables_not_injected() {
        let step = make_test_step("step_b", NodeType::Agent, "Do B");

        let result = inject_step_outputs("Do B", &step, &HashMap::new(), &[], &HashMap::new());

        assert!(!result.contains("<workflow_variables>"));
    }

    #[test]
    fn inject_step_outputs_parallel_parent_aggregated_children() {
        let mut step = make_test_step("spec_fix", NodeType::Agent, "Fix plan");
        step.pass_output_from = Some(vec![
            "spec_review_parallel".to_string(),
            "plan_draft".to_string(),
        ]);
        let mut outputs = HashMap::new();
        outputs.insert(
            "spec_review_parallel".to_string(),
            StepOutput {
                step_name: "spec_review_parallel".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({
                    "review_completeness": {
                        "verdict": "NEEDS_FIX",
                        "findings": [{ "severity": "must_fix", "message": "Missing error handling" }]
                    },
                    "review_clarity": {
                        "verdict": "LGTM",
                        "findings": []
                    }
                })),
                output_contract: None,
                token_usage: None,
                completed_at: 1000.0,
            },
        );
        outputs.insert(
            "plan_draft".to_string(),
            make_step_output("plan_draft", "Draft spec content", None),
        );

        let result = inject_step_outputs("Fix plan", &step, &outputs, &[], &HashMap::new());

        assert!(result.contains("<step_output name=\"spec_review_parallel\">"));
        assert!(result.contains("NEEDS_FIX"));
        assert!(result.contains("Missing error handling"));
        assert!(result.contains("<step_output name=\"plan_draft\">"));
        assert!(result.contains("Draft spec content"));
    }

    #[test]
    fn inject_step_outputs_parallel_parent_via_pass_previous_response() {
        let mut step = make_test_step("spec_fix", NodeType::Agent, "Fix plan");
        step.pass_previous_response = Some(true);
        let outputs = HashMap::from([(
            "spec_review_parallel".to_string(),
            StepOutput {
                step_name: "spec_review_parallel".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({
                    "review_completeness": { "verdict": "LGTM", "findings": [] },
                    "review_security": {
                        "verdict": "NEEDS_FIX",
                        "findings": [{ "severity": "must_fix", "message": "SQL injection risk" }]
                    }
                })),
                output_contract: None,
                token_usage: None,
                completed_at: 1000.0,
            },
        )]);
        let mut history = history_entry("spec_review_parallel");
        history.result = Some("else".to_string());
        history.run_index = 1;

        let result = inject_step_outputs("Fix plan", &step, &outputs, &[history], &HashMap::new());

        assert!(result.contains("<step_output name=\"spec_review_parallel\">"));
        assert!(result.contains("NEEDS_FIX"));
        assert!(result.contains("SQL injection risk"));
    }
}
