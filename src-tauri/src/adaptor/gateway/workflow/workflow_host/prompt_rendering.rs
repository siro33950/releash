//! Workflow prompt construction.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::prompt_composition;
use crate::domain::workflow::services::reference::{
    self, resolve_runtime_reference, REQUEST_ARTIFACT,
};
#[cfg(test)]
use crate::domain::workflow::services::template_preview;
use crate::domain::workflow::FacetContents;
use crate::domain::workflow::RuntimeArtifact;
use crate::domain::workflow::{NodeDefinition, SchemaDef};
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

pub(crate) fn artifact_values(
    runtime_artifacts: &HashMap<String, RuntimeArtifact>,
    request: Option<&str>,
) -> HashMap<String, Value> {
    let mut artifacts = HashMap::new();
    artifacts.insert(
        REQUEST_ARTIFACT.to_string(),
        Value::String(request.unwrap_or_default().to_string()),
    );
    for (name, output) in runtime_artifacts {
        if let Some(value) = &output.artifact {
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

pub(crate) fn append_completion_action(
    prompt: &mut String,
    artifact: Option<&str>,
    schemas: &BTreeMap<String, SchemaDef>,
    execution_id: &str,
    node_name: &str,
    node_execution_id: &str,
) {
    let (schema_guidance, action) = match artifact {
        Some(contract) => {
            let domain_schemas = schemas.clone();
            let schema_guidance =
                workflow_contract::render_contract_prompt_guidance(&domain_schemas, contract);
            let action = prompt_composition::artifact_completion_action(
                contract,
                execution_id,
                node_name,
                Some(node_execution_id),
            );
            (schema_guidance, action)
        }
        None => (
            None,
            prompt_composition::artifactless_completion_action(
                execution_id,
                node_name,
                node_execution_id,
            ),
        ),
    };
    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }
    if let Some(schema_guidance) = schema_guidance {
        prompt.push_str(&schema_guidance);
        prompt.push_str("\n\n");
    }
    prompt.push_str(&action);
}

pub(crate) fn build_node_prompt(
    node: &NodeDefinition,
    facet_contents: Option<&FacetContents>,
    execution_id: &str,
    node_execution_id: &str,
    request: Option<&str>,
    artifacts: &HashMap<String, RuntimeArtifact>,
    schemas: &BTreeMap<String, SchemaDef>,
) -> Result<(Option<String>, String), WorkflowRuntimeError> {
    if !node.has_facet_refs() {
        return Err(WorkflowRuntimeError::InvalidWorkflow(format!(
            "Node '{}' has no facet refs.",
            node.name
        )));
    }

    if node.has_facet_refs() && facet_contents.is_none_or(FacetContents::is_empty) {
        return Err(WorkflowRuntimeError::InvalidWorkflow(format!(
            "Node '{}' has unresolved facet refs (workflow must go through load pipeline)",
            node.name
        )));
    }

    let artifacts = artifact_values(artifacts, request);
    let composed = prompt_composition::compose_facets(facet_contents);
    let system_prompt = composed
        .system_prompt
        .map(|content| render_prompt_content(&content, &artifacts, None));
    let rendered_user = render_prompt_content(&composed.user_message, &artifacts, None);
    let mut prompt = inject_input_artifacts(&rendered_user, &node.inputs, &artifacts);
    append_completion_action(
        &mut prompt,
        node.artifact.as_deref(),
        schemas,
        execution_id,
        &node.name,
        node_execution_id,
    );
    Ok((system_prompt, prompt))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FanoutChildPromptContext<'a> {
    item: Option<&'a Value>,
    node_execution_id: &'a str,
}

impl<'a> FanoutChildPromptContext<'a> {
    pub(crate) fn new(item: Option<&'a Value>, node_execution_id: &'a str) -> Self {
        Self {
            item,
            node_execution_id,
        }
    }
}

pub(crate) fn build_fanout_child_prompt(
    node: &NodeDefinition,
    facet_contents: Option<&FacetContents>,
    execution_id: &str,
    request: Option<&str>,
    artifacts: &HashMap<String, RuntimeArtifact>,
    context: FanoutChildPromptContext<'_>,
    schemas: &BTreeMap<String, SchemaDef>,
) -> Result<(Option<String>, String), WorkflowRuntimeError> {
    if node.has_facet_refs() && facet_contents.is_none_or(FacetContents::is_empty) {
        return Err(WorkflowRuntimeError::InvalidWorkflow(format!(
            "Fanout child '{}' has unresolved facet refs (workflow must go through load pipeline)",
            node.name
        )));
    }

    let artifacts = artifact_values(artifacts, request);
    let composed = prompt_composition::compose_facets(facet_contents);
    let system_prompt = composed
        .system_prompt
        .map(|content| render_prompt_content(&content, &artifacts, context.item));
    let rendered_user = render_prompt_content(&composed.user_message, &artifacts, context.item);
    let rendered_user = inject_input_artifacts(&rendered_user, &node.inputs, &artifacts);
    let mut user_message = inject_fanout_item(&rendered_user, node.input.as_deref(), context.item);
    append_completion_action(
        &mut user_message,
        node.artifact.as_deref(),
        schemas,
        execution_id,
        &node.name,
        context.node_execution_id,
    );

    Ok((system_prompt, user_message))
}

pub(crate) fn request_node_artifact(request: &str, timestamp: f64) -> RuntimeArtifact {
    RuntimeArtifact {
        node_name: REQUEST_ARTIFACT.to_string(),
        attempt: 0,
        session_id: None,
        result: None,
        artifact: Some(Value::String(request.to_string())),
        contract: Some("string".to_string()),
        token_usage: None,
        completed_at: timestamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{FacetRefs, NodeKind, SessionSpec};

    fn make_test_node(name: &str, instruction: &str) -> NodeDefinition {
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
    fn build_node_prompt_reports_missing_facets_with_node_vocabulary() {
        let node = NodeDefinition {
            name: "review".to_string(),
            ..NodeDefinition::default()
        };

        let error = build_node_prompt(
            &node,
            None,
            "execution-1",
            "node-execution-1",
            None,
            &HashMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("node without facet refs must be rejected");

        assert!(matches!(
            error,
            WorkflowRuntimeError::InvalidWorkflow(message)
                if message == "Node 'review' has no facet refs."
        ));
    }

    #[test]
    fn build_node_prompt_injects_inputs_as_json() {
        let mut node = make_test_node("implement", "Implement {{ request }}");
        node.inputs = vec!["request".to_string(), "plan".to_string()];
        let resolved = instruction_contents("Implement {{ request }}");
        let outputs = HashMap::from([(
            "plan".to_string(),
            RuntimeArtifact {
                node_name: "plan".to_string(),
                attempt: 1,
                session_id: None,
                result: None,
                artifact: Some(serde_json::json!({"summary": "ready"})),
                contract: Some("plan".to_string()),
                token_usage: None,
                completed_at: 1.0,
            },
        )]);

        let (_system, prompt) = build_node_prompt(
            &node,
            Some(&resolved),
            "execution-1",
            "node-execution-1",
            Some("ship"),
            &outputs,
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("Implement ship"));
        assert!(prompt.contains("## input: request"));
        assert!(prompt.contains("\"ship\""));
        assert!(prompt.contains("## input: plan"));
        assert!(prompt.contains("\"summary\": \"ready\""));
    }

    #[test]
    fn build_node_prompt_renders_node_field() {
        let node = make_test_node("fix", "Spec dir: {{ authoring.spec_dir }}");
        let resolved = instruction_contents("Spec dir: {{ authoring.spec_dir }}");
        let outputs = HashMap::from([(
            "authoring".to_string(),
            RuntimeArtifact {
                node_name: "authoring".to_string(),
                attempt: 1,
                session_id: None,
                result: None,
                artifact: Some(serde_json::json!({"spec_dir": "docs/specs/foo"})),
                contract: Some("spec-directory".to_string()),
                token_usage: None,
                completed_at: 1.0,
            },
        )]);

        let (_system, prompt) = build_node_prompt(
            &node,
            Some(&resolved),
            "execution-1",
            "node-execution-1",
            Some(""),
            &outputs,
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("Spec dir: docs/specs/foo"));
    }

    #[test]
    fn build_fanout_child_prompt_binds_item_as_declared_input() {
        let mut node = make_test_node("worker", "Review {{ item.path }}");
        node.input = Some("work-item".to_string());
        let resolved = instruction_contents("Review {{ item.path }}");
        let item = serde_json::json!({"path": "src/lib.rs", "priority": 2});

        let (_system, prompt) = build_fanout_child_prompt(
            &node,
            Some(&resolved),
            "execution-1",
            None,
            &HashMap::new(),
            FanoutChildPromptContext::new(Some(&item), "node-execution-1"),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("Review src/lib.rs"));
        assert!(prompt.contains("## input: item (work-item)"));
        assert!(prompt.contains("\"priority\": 2"));
    }

    #[test]
    fn build_fanout_child_prompt_injects_ordinary_inputs_and_binds_item() {
        let mut node = make_test_node("worker", "Review {{ item.path }} for {{ request }}");
        node.inputs = vec!["request".to_string(), "plan".to_string()];
        node.input = Some("work-item".to_string());
        let resolved = instruction_contents("Review {{ item.path }} for {{ request }}");
        let outputs = HashMap::from([(
            "plan".to_string(),
            RuntimeArtifact {
                node_name: "plan".to_string(),
                attempt: 1,
                session_id: None,
                result: None,
                artifact: Some(serde_json::json!({"summary": "ready"})),
                contract: Some("plan".to_string()),
                token_usage: None,
                completed_at: 1.0,
            },
        )]);
        let item = serde_json::json!({"path": "src/lib.rs", "priority": 2});

        let (_system, prompt) = build_fanout_child_prompt(
            &node,
            Some(&resolved),
            "execution-1",
            Some("ship"),
            &outputs,
            FanoutChildPromptContext::new(Some(&item), "node-execution-1"),
            &BTreeMap::new(),
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

    #[test]
    fn build_node_prompt_appends_artifactless_output_submit_action() {
        let node = make_test_node("review", "Review the change.");
        let resolved = instruction_contents("Review the change.");

        let (_system, prompt) = build_node_prompt(
            &node,
            Some(&resolved),
            "execution-1",
            "node-execution-1",
            None,
            &HashMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("releash workflow output submit execution-1"));
        assert!(prompt.contains("--node review"));
        assert!(prompt.contains("--node-execution node-execution-1"));
        assert!(!prompt.contains("--type"));
        assert!(!prompt.contains("--json"));
    }

    #[test]
    fn build_node_prompt_appends_canonical_output_submit_action() {
        let mut node = make_test_node("review", "Review the change.");
        node.artifact = Some("review-result".to_string());
        let resolved = instruction_contents("Review the change.");

        let schemas = BTreeMap::from([(
            "review-result".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([(
                    "verdict".to_string(),
                    SchemaDef::String { r#enum: None },
                )]),
                required: ["verdict".to_string()].into_iter().collect(),
            },
        )]);
        let (_system, prompt) = build_node_prompt(
            &node,
            Some(&resolved),
            "execution-1",
            "node-execution-1",
            None,
            &HashMap::new(),
            &schemas,
        )
        .unwrap();

        assert!(prompt.contains("## Artifact contract"));
        assert!(prompt.contains("\"verdict\": \"string\""));
        assert!(prompt.contains("Fields not listed in `properties` are accepted"));
        assert!(prompt.contains("releash workflow output submit execution-1"));
        assert!(prompt.contains("--node review"));
        assert!(prompt.contains("--node-execution node-execution-1"));
        assert!(prompt.contains("--type review-result"));
        let deprecated_step_flag = ["--", "step"].concat();
        assert!(!prompt.contains(&deprecated_step_flag));
    }

    #[test]
    fn build_fanout_child_prompt_appends_artifactless_output_submit_action() {
        let node = make_test_node("review", "Review the change.");
        let resolved = instruction_contents("Review the change.");

        let (_system, prompt) = build_fanout_child_prompt(
            &node,
            Some(&resolved),
            "execution-1",
            None,
            &HashMap::new(),
            FanoutChildPromptContext::new(None, "node-execution-1"),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("releash workflow output submit execution-1"));
        assert!(prompt.contains("--node review"));
        assert!(prompt.contains("--node-execution node-execution-1"));
        assert!(!prompt.contains("--type"));
        assert!(!prompt.contains("--json"));
    }

    #[test]
    fn build_fanout_child_prompt_addresses_the_node_execution() {
        let mut node = make_test_node("review", "Review {{ item.path }}.");
        node.input = Some("work-item".to_string());
        node.artifact = Some("review-result".to_string());
        let resolved = instruction_contents("Review {{ item.path }}.");
        let item = serde_json::json!({"path": "src/lib.rs"});

        let (_system, prompt) = build_fanout_child_prompt(
            &node,
            Some(&resolved),
            "execution-1",
            None,
            &HashMap::new(),
            FanoutChildPromptContext::new(Some(&item), "node-execution-1"),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("releash workflow output submit execution-1"));
        assert!(prompt.contains("--node review"));
        assert!(prompt.contains("--node-execution node-execution-1"));
        assert!(prompt.contains("--type review-result"));
    }
}
