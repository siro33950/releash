use std::collections::HashMap;

use serde_json::Value;

use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::facet::FacetContents;
use crate::adaptor::gateway::workflow::schema::NodeDefinition;
use crate::adaptor::gateway::workflow::state::StepOutput;
use crate::domain::workflow::services::reference::{
    self, resolve_runtime_reference, REQUEST_ARTIFACT,
};
#[cfg(test)]
use crate::domain::workflow::services::template_preview;

pub(crate) fn artifact_values(
    step_outputs: &HashMap<String, StepOutput>,
    request: Option<&str>,
) -> HashMap<String, Value> {
    let mut artifacts = HashMap::new();
    artifacts.insert(
        REQUEST_ARTIFACT.to_string(),
        Value::String(request.unwrap_or_default().to_string()),
    );
    for (name, output) in step_outputs {
        if let Some(value) = &output.structured_output {
            artifacts.insert(name.clone(), value.clone());
        }
    }
    artifacts
}

fn render_prompt_content(
    content: &str,
    artifacts: &HashMap<String, Value>,
    item: Option<&Value>,
) -> String {
    render_artifact_references(content, artifacts, item)
}

pub(crate) fn find_undefined_template_variables(content: &str) -> Vec<String> {
    reference::extract_template_references(content)
        .into_iter()
        .filter(|value| reference::parse_reference(value).is_err())
        .collect()
}

#[cfg(test)]
pub(crate) fn render_template_variables(content: &str, values: &HashMap<String, String>) -> String {
    template_preview::render_template_variables(content, values)
}

pub(crate) fn render_artifact_references(
    content: &str,
    artifacts: &HashMap<String, Value>,
    item: Option<&Value>,
) -> String {
    replace_template_refs(content, |inner| {
        let parsed = reference::parse_reference(inner).ok()?;
        resolve_runtime_reference(&parsed, artifacts, item).map(value_to_template_string)
    })
}

pub(crate) fn inject_input_artifacts(
    prompt: &str,
    inputs: &[String],
    artifacts: &HashMap<String, Value>,
) -> String {
    let mut result = prompt.to_string();
    if let Some(block) = input_artifacts_block(inputs, artifacts) {
        result.push_str(&block);
    }
    result
}

fn input_artifacts_block(inputs: &[String], artifacts: &HashMap<String, Value>) -> Option<String> {
    let mut blocks = Vec::new();
    for input in inputs {
        let Ok(parsed) = reference::parse_reference(input) else {
            continue;
        };
        let name = input.trim();
        let Some(value) = resolve_runtime_reference(&parsed, artifacts, None) else {
            continue;
        };
        let json = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "null".to_string());
        blocks.push(format!("## input: {name}\n```json\n{json}\n```"));
    }
    (!blocks.is_empty()).then(|| format!("\n\n{}", blocks.join("\n\n")))
}

fn inject_fanout_item(prompt: &str, input_contract: Option<&str>, item: Option<&Value>) -> String {
    let (Some(input_contract), Some(item)) = (input_contract, item) else {
        return prompt.to_string();
    };
    let json = serde_json::to_string_pretty(item).unwrap_or_else(|_| "null".to_string());
    let block = format!("## input: item ({input_contract})\n```json\n{json}\n```");
    if prompt.is_empty() {
        block
    } else {
        format!("{prompt}\n\n{block}")
    }
}

fn value_to_template_string(value: Value) -> String {
    match value {
        Value::String(value) => value,
        other => serde_json::to_string(&other).unwrap_or_else(|_| "null".to_string()),
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
        match resolve(raw_inner.trim()) {
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

fn render_workflow_instruction(
    instruction: &str,
    artifacts: &HashMap<String, Value>,
    item: Option<&Value>,
) -> Option<String> {
    let rendered = render_prompt_content(instruction, artifacts, item)
        .trim()
        .to_string();
    (!rendered.is_empty()).then_some(rendered)
}

pub(crate) fn render_step_workflow_instruction(
    _step: &NodeDefinition,
    facet_contents: Option<&FacetContents>,
    request: Option<&str>,
    step_outputs: &HashMap<String, StepOutput>,
) -> Option<String> {
    let artifacts = artifact_values(step_outputs, request);
    render_workflow_instruction(facet_contents?.instruction.as_ref()?, &artifacts, None)
}

pub(crate) fn render_fanout_child_workflow_instruction(
    _step: &NodeDefinition,
    facet_contents: Option<&FacetContents>,
    request: Option<&str>,
    step_outputs: &HashMap<String, StepOutput>,
    item: Option<&Value>,
) -> Option<String> {
    let artifacts = artifact_values(step_outputs, request);
    render_workflow_instruction(facet_contents?.instruction.as_ref()?, &artifacts, item)
}

pub(crate) fn append_artifact_completion_action(
    prompt: &mut String,
    artifact: Option<&str>,
    run_id: &str,
    step_name: &str,
    node_execution_id: Option<&str>,
) {
    let Some(contract) = artifact else {
        return;
    };
    let action = crate::adaptor::gateway::workflow::facet::artifact_completion_action(
        contract,
        run_id,
        step_name,
        node_execution_id,
    );
    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }
    prompt.push_str(&action);
}

pub(crate) fn build_step_prompt(
    step: &NodeDefinition,
    facet_contents: Option<&FacetContents>,
    run_id: &str,
    request: Option<&str>,
    step_outputs: &HashMap<String, StepOutput>,
) -> Result<(Option<String>, String), WorkflowEngineError> {
    if !step.has_facet_refs() {
        return Err(WorkflowEngineError::InvalidWorkflow(format!(
            "Step '{}' has no facet refs.",
            step.name
        )));
    }

    if step.has_facet_refs() && facet_contents.is_none_or(FacetContents::is_empty) {
        return Err(WorkflowEngineError::InvalidWorkflow(format!(
            "Step '{}' has unresolved facet refs (workflow must go through load pipeline)",
            step.name
        )));
    }

    let artifacts = artifact_values(step_outputs, request);
    let composed = crate::adaptor::gateway::workflow::facet::compose_facets(facet_contents);
    let system_prompt = composed
        .system_prompt
        .map(|content| render_prompt_content(&content, &artifacts, None));
    let rendered_user = render_prompt_content(&composed.user_message, &artifacts, None);
    let mut prompt = inject_input_artifacts(&rendered_user, &step.inputs, &artifacts);
    append_artifact_completion_action(
        &mut prompt,
        step.artifact.as_deref(),
        run_id,
        &step.name,
        None,
    );
    Ok((system_prompt, prompt))
}

pub(crate) fn build_fanout_child_prompt(
    step: &NodeDefinition,
    facet_contents: Option<&FacetContents>,
    run_id: &str,
    request: Option<&str>,
    step_outputs: &HashMap<String, StepOutput>,
    item: Option<&Value>,
    node_execution_id: &str,
) -> Result<(Option<String>, String), WorkflowEngineError> {
    if step.has_facet_refs() && facet_contents.is_none_or(FacetContents::is_empty) {
        return Err(WorkflowEngineError::InvalidWorkflow(format!(
            "Fanout child '{}' has unresolved facet refs (workflow must go through load pipeline)",
            step.name
        )));
    }

    let artifacts = artifact_values(step_outputs, request);
    let composed = crate::adaptor::gateway::workflow::facet::compose_facets(facet_contents);
    let system_prompt = composed
        .system_prompt
        .map(|content| render_prompt_content(&content, &artifacts, item));
    let rendered_user = render_prompt_content(&composed.user_message, &artifacts, item);
    let rendered_user = inject_input_artifacts(&rendered_user, &step.inputs, &artifacts);
    let mut user_message = inject_fanout_item(&rendered_user, step.input.as_deref(), item);
    append_artifact_completion_action(
        &mut user_message,
        step.artifact.as_deref(),
        run_id,
        &step.name,
        Some(node_execution_id),
    );

    Ok((system_prompt, user_message))
}

pub(crate) fn request_step_output(request: &str, timestamp: f64) -> StepOutput {
    StepOutput {
        step_name: REQUEST_ARTIFACT.to_string(),
        run_index: 0,
        session_id: None,
        result: None,
        structured_output: Some(Value::String(request.to_string())),
        artifact_contract: Some("string".to_string()),
        token_usage: None,
        completed_at: timestamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{FacetRefs, NodeKind, SessionSpec};

    fn make_test_step(name: &str, instruction: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    instruction: Some(instruction.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn instruction_contents(instruction: &str) -> FacetContents {
        FacetContents {
            instruction: Some(instruction.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn build_step_prompt_injects_inputs_as_json() {
        let mut step = make_test_step("implement", "Implement {{ request }}");
        step.inputs = vec!["request".to_string(), "plan".to_string()];
        let resolved = instruction_contents("Implement {{ request }}");
        let outputs = HashMap::from([(
            "plan".to_string(),
            StepOutput {
                step_name: "plan".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({"summary": "ready"})),
                artifact_contract: Some("plan".to_string()),
                token_usage: None,
                completed_at: 1.0,
            },
        )]);

        let (_system, prompt) =
            build_step_prompt(&step, Some(&resolved), "run-1", Some("ship"), &outputs).unwrap();

        assert!(prompt.contains("Implement ship"));
        assert!(prompt.contains("## input: request"));
        assert!(prompt.contains("\"ship\""));
        assert!(prompt.contains("## input: plan"));
        assert!(prompt.contains("\"summary\": \"ready\""));
    }

    #[test]
    fn build_step_prompt_renders_node_field() {
        let step = make_test_step("fix", "Spec dir: {{ authoring.spec_dir }}");
        let resolved = instruction_contents("Spec dir: {{ authoring.spec_dir }}");
        let outputs = HashMap::from([(
            "authoring".to_string(),
            StepOutput {
                step_name: "authoring".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({"spec_dir": "docs/specs/foo"})),
                artifact_contract: Some("spec-directory".to_string()),
                token_usage: None,
                completed_at: 1.0,
            },
        )]);

        let (_system, prompt) =
            build_step_prompt(&step, Some(&resolved), "run-1", Some(""), &outputs).unwrap();

        assert!(prompt.contains("Spec dir: docs/specs/foo"));
    }

    #[test]
    fn build_fanout_child_prompt_binds_item_as_declared_input() {
        let mut step = make_test_step("worker", "Review {{ item.path }}");
        step.input = Some("work-item".to_string());
        let resolved = instruction_contents("Review {{ item.path }}");
        let item = serde_json::json!({"path": "src/lib.rs", "priority": 2});

        let (_system, prompt) = build_fanout_child_prompt(
            &step,
            Some(&resolved),
            "run-1",
            None,
            &HashMap::new(),
            Some(&item),
            "node-execution-1",
        )
        .unwrap();

        assert!(prompt.contains("Review src/lib.rs"));
        assert!(prompt.contains("## input: item (work-item)"));
        assert!(prompt.contains("\"priority\": 2"));
    }

    #[test]
    fn build_fanout_child_prompt_injects_ordinary_inputs_and_binds_item() {
        let mut step = make_test_step("worker", "Review {{ item.path }} for {{ request }}");
        step.inputs = vec!["request".to_string(), "plan".to_string()];
        step.input = Some("work-item".to_string());
        let resolved = instruction_contents("Review {{ item.path }} for {{ request }}");
        let outputs = HashMap::from([(
            "plan".to_string(),
            StepOutput {
                step_name: "plan".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({"summary": "ready"})),
                artifact_contract: Some("plan".to_string()),
                token_usage: None,
                completed_at: 1.0,
            },
        )]);
        let item = serde_json::json!({"path": "src/lib.rs", "priority": 2});

        let (_system, prompt) = build_fanout_child_prompt(
            &step,
            Some(&resolved),
            "run-1",
            Some("ship"),
            &outputs,
            Some(&item),
            "node-execution-1",
        )
        .unwrap();

        assert!(prompt.contains("Review src/lib.rs for ship"));
        assert!(prompt.contains("## input: request"));
        assert!(prompt.contains("\"ship\""));
        assert!(prompt.contains("## input: plan"));
        assert!(prompt.contains("\"summary\": \"ready\""));
        assert!(prompt.contains("## input: item (work-item)"));
        assert!(prompt.contains("\"priority\": 2"));
    }
}
