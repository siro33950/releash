//! Workflow prompt construction.
//!
//! 本文（command / facet）はパラメータ名だけを参照する。値の解決規則
//! （children エントリの inputs 束縛・items 自動束縛）は domain の
//! `reference::resolve_entry_bindings` が所有し、ここでは描画のみを行う。

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::prompt_composition;
use crate::domain::workflow::services::reference::{self, REQUEST_ARTIFACT};
#[cfg(test)]
use crate::domain::workflow::services::template_preview;
use crate::domain::workflow::FacetContents;
use crate::domain::workflow::RuntimeArtifact;
use crate::domain::workflow::{ChildEntry, NodeDefinition, SchemaDef};
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

/// root スコープ（sequence の子・単独 node）のパラメータ束縛（YAML の配線順を保持）。
pub(crate) fn parameter_bindings(
    entry: Option<&ChildEntry>,
    runtime_artifacts: &HashMap<String, RuntimeArtifact>,
    request: Option<&str>,
) -> Vec<(String, Value)> {
    let artifacts = artifact_values(runtime_artifacts, request);
    reference::resolve_entry_bindings(entry, &artifacts)
}

/// fanout 自身の束縛済みパラメータ（root スコープのエントリ配線から解決）。
/// 子への供給元はこのパラメータ + `request` + `items` に閉じる。
pub(crate) fn fanout_parent_parameters(
    workflow: &crate::domain::workflow::WorkflowDefinition,
    parent_node_name: &str,
    runtime_artifacts: &HashMap<String, RuntimeArtifact>,
    request: Option<&str>,
) -> HashMap<String, Value> {
    let entry = workflow
        .root_sequence()
        .and_then(|sequence| sequence.child_entry(parent_node_name));
    parameter_bindings(entry, runtime_artifacts, request)
        .into_iter()
        .collect()
}

/// fanout の子のパラメータ束縛（親パラメータ + request + items）。
pub(crate) fn fanout_child_bindings(
    node: &NodeDefinition,
    entry: Option<&ChildEntry>,
    parent_parameters: &HashMap<String, Value>,
    request: Option<&str>,
    item: Option<&Value>,
) -> Vec<(String, Value)> {
    let request = request.map(|value| Value::String(value.to_string()));
    reference::resolve_fanout_child_bindings(entry, node, parent_parameters, request.as_ref(), item)
}

fn binding_values(bindings: &[(String, Value)]) -> HashMap<String, Value> {
    bindings.iter().cloned().collect()
}

pub(crate) fn find_undefined_template_variables(content: &str) -> Vec<String> {
    reference::extract_template_references(content)
        .into_iter()
        .filter(|value| reference::split_reference(value).is_none())
        .collect()
}

#[cfg(test)]
pub(crate) fn render_template_variables(content: &str, values: &HashMap<String, String>) -> String {
    template_preview::render_template_variables(content, values)
}

/// `{{ <パラメータ>(.field) }}` を束縛済みパラメータ値で置換する。
/// 解決できない参照はそのまま残す。
pub(crate) fn render_parameter_references(content: &str, bindings: &[(String, Value)]) -> String {
    let values = binding_values(bindings);
    replace_template_refs(content, |inner| {
        let (root, field) = reference::split_reference(inner)?;
        reference::resolve_template_value(root, field, &values).map(value_to_template_string)
    })
}

/// 束縛済みパラメータを `## input: <パラメータ>` ブロックとして本文末尾へ注入する。
/// 型あり（Contract 付き）パラメータは Contract 名を併記する。
pub(crate) fn inject_input_parameters(
    prompt: &str,
    node: &NodeDefinition,
    bindings: &[(String, Value)],
) -> String {
    let mut blocks = Vec::new();
    for (parameter, value) in bindings {
        let json = serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string());
        let heading = match node
            .input_parameter(parameter)
            .and_then(|param| param.contract.as_deref())
        {
            Some(contract) => format!("## input: {parameter} ({contract})"),
            None => format!("## input: {parameter}"),
        };
        blocks.push(format!("{heading}\n```json\n{json}\n```"));
    }
    if blocks.is_empty() {
        return prompt.to_string();
    }
    if prompt.is_empty() {
        blocks.join("\n\n")
    } else {
        format!("{prompt}\n\n{}", blocks.join("\n\n"))
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
    node_execution_id: &str,
) {
    let (schema_guidance, action) = match artifact {
        Some(contract) => {
            let domain_schemas = schemas.clone();
            let schema_guidance =
                workflow_contract::render_contract_prompt_guidance(&domain_schemas, contract);
            let action =
                prompt_composition::artifact_completion_action(contract, node_execution_id);
            (schema_guidance, action)
        }
        None => (
            None,
            prompt_composition::artifactless_completion_action(node_execution_id),
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
    node_execution_id: &str,
    entry: Option<&ChildEntry>,
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

    let bindings = parameter_bindings(entry, artifacts, request);
    let composed = prompt_composition::compose_facets(facet_contents);
    let system_prompt = composed
        .system_prompt
        .map(|content| render_parameter_references(&content, &bindings));
    let rendered_user = render_parameter_references(&composed.user_message, &bindings);
    let mut prompt = inject_input_parameters(&rendered_user, node, &bindings);
    append_completion_action(
        &mut prompt,
        node.artifact.as_deref(),
        schemas,
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
    entry: Option<&ChildEntry>,
    request: Option<&str>,
    parent_parameters: &HashMap<String, Value>,
    context: FanoutChildPromptContext<'_>,
    schemas: &BTreeMap<String, SchemaDef>,
) -> Result<(Option<String>, String), WorkflowRuntimeError> {
    if node.has_facet_refs() && facet_contents.is_none_or(FacetContents::is_empty) {
        return Err(WorkflowRuntimeError::InvalidWorkflow(format!(
            "Fanout child '{}' has unresolved facet refs (workflow must go through load pipeline)",
            node.name
        )));
    }

    let bindings = fanout_child_bindings(node, entry, parent_parameters, request, context.item);
    let composed = prompt_composition::compose_facets(facet_contents);
    let system_prompt = composed
        .system_prompt
        .map(|content| render_parameter_references(&content, &bindings));
    let rendered_user = render_parameter_references(&composed.user_message, &bindings);
    let mut user_message = inject_input_parameters(&rendered_user, node, &bindings);
    append_completion_action(
        &mut user_message,
        node.artifact.as_deref(),
        schemas,
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
    use crate::domain::workflow::value_objects::InputSourceRef;
    use crate::domain::workflow::{FacetRefs, InputParam, NodeKind, SessionSpec};

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

    fn untyped_param(name: &str) -> InputParam {
        InputParam {
            name: name.to_string(),
            contract: None,
        }
    }

    fn entry_with_inputs(name: &str, inputs: Vec<(&str, &str)>) -> ChildEntry {
        ChildEntry {
            name: name.to_string(),
            inputs: inputs
                .into_iter()
                .map(|(parameter, source)| (parameter.to_string(), InputSourceRef::new(source)))
                .collect(),
            rules: None,
        }
    }

    fn instruction_contents(instruction: &str) -> FacetContents {
        FacetContents {
            instruction: Some(instruction.to_string()),
            ..Default::default()
        }
    }

    fn plan_artifact() -> (String, RuntimeArtifact) {
        (
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
        )
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
            "node-execution-1",
            None,
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
    fn build_node_prompt_injects_bound_parameters_as_json() {
        let mut node = make_test_node("implement", "Implement {{ goal }}");
        node.input = vec![untyped_param("goal"), untyped_param("plan_doc")];
        let entry = entry_with_inputs("implement", vec![("goal", "request"), ("plan_doc", "plan")]);
        let resolved = instruction_contents("Implement {{ goal }}");
        let outputs = HashMap::from([plan_artifact()]);

        let (_system, prompt) = build_node_prompt(
            &node,
            Some(&resolved),
            "node-execution-1",
            Some(&entry),
            Some("ship"),
            &outputs,
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("Implement ship"));
        assert!(prompt.contains("## input: goal"));
        assert!(prompt.contains("\"ship\""));
        assert!(prompt.contains("## input: plan_doc"));
        assert!(prompt.contains("\"summary\": \"ready\""));
    }

    #[test]
    fn build_node_prompt_renders_parameter_field_path() {
        let mut node = make_test_node("fix", "Spec dir: {{ spec.spec_dir }}");
        node.input = vec![untyped_param("spec")];
        let entry = entry_with_inputs("fix", vec![("spec", "authoring")]);
        let resolved = instruction_contents("Spec dir: {{ spec.spec_dir }}");
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
            "node-execution-1",
            Some(&entry),
            Some(""),
            &outputs,
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("Spec dir: docs/specs/foo"));
    }

    #[test]
    fn build_fanout_child_prompt_auto_binds_item_to_sole_parameter() {
        let mut node = make_test_node("worker", "Review {{ task.path }}");
        node.input = vec![InputParam {
            name: "task".to_string(),
            contract: Some("work-item".to_string()),
        }];
        let resolved = instruction_contents("Review {{ task.path }}");
        let item = serde_json::json!({"path": "src/lib.rs", "priority": 2});

        let (_system, prompt) = build_fanout_child_prompt(
            &node,
            Some(&resolved),
            None,
            None,
            &HashMap::new(),
            FanoutChildPromptContext::new(Some(&item), "node-execution-1"),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("Review src/lib.rs"));
        assert!(prompt.contains("## input: task (work-item)"));
        assert!(prompt.contains("\"priority\": 2"));
    }

    #[test]
    fn build_fanout_child_prompt_binds_explicit_inputs_and_items() {
        let mut node = make_test_node("worker", "Review {{ task.path }} for {{ goal }}");
        node.input = vec![
            InputParam {
                name: "task".to_string(),
                contract: Some("work-item".to_string()),
            },
            untyped_param("goal"),
            untyped_param("plan_doc"),
        ];
        let entry = entry_with_inputs(
            "worker",
            vec![("task", "items"), ("goal", "request"), ("plan_doc", "plan")],
        );
        let resolved = instruction_contents("Review {{ task.path }} for {{ goal }}");
        // fanout の子への供給は親 fanout の束縛済みパラメータから解決する。
        let parent_parameters =
            HashMap::from([("plan".to_string(), serde_json::json!({"summary": "ready"}))]);
        let item = serde_json::json!({"path": "src/lib.rs", "priority": 2});

        let (_system, prompt) = build_fanout_child_prompt(
            &node,
            Some(&resolved),
            Some(&entry),
            Some("ship"),
            &parent_parameters,
            FanoutChildPromptContext::new(Some(&item), "node-execution-1"),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("Review src/lib.rs for ship"));
        assert!(prompt.contains("## input: task (work-item)"));
        assert!(prompt.contains("\"priority\": 2"));
        assert!(prompt.contains("## input: goal"));
        assert!(prompt.contains("\"ship\""));
        assert!(prompt.contains("## input: plan_doc"));
        assert!(prompt.contains("\"summary\": \"ready\""));
    }

    #[test]
    fn build_node_prompt_appends_artifactless_output_submit_action() {
        let node = make_test_node("review", "Review the change.");
        let resolved = instruction_contents("Review the change.");

        let (_system, prompt) = build_node_prompt(
            &node,
            Some(&resolved),
            "node-execution-1",
            None,
            None,
            &HashMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("releash workflow output submit"));
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
            "node-execution-1",
            None,
            None,
            &HashMap::new(),
            &schemas,
        )
        .unwrap();

        assert!(prompt.contains("## Artifact contract"));
        assert!(prompt.contains("\"verdict\": \"string\""));
        assert!(prompt.contains("Fields not listed in `properties` are accepted"));
        assert!(prompt.contains("releash workflow output submit"));
        assert!(prompt.contains("--node-execution node-execution-1"));
        assert!(prompt.contains("--type review-result"));
    }

    #[test]
    fn build_fanout_child_prompt_appends_artifactless_output_submit_action() {
        let node = make_test_node("review", "Review the change.");
        let resolved = instruction_contents("Review the change.");

        let (_system, prompt) = build_fanout_child_prompt(
            &node,
            Some(&resolved),
            None,
            None,
            &HashMap::new(),
            FanoutChildPromptContext::new(None, "node-execution-1"),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("releash workflow output submit"));
        assert!(prompt.contains("--node-execution node-execution-1"));
        assert!(!prompt.contains("--type"));
        assert!(!prompt.contains("--json"));
    }

    #[test]
    fn build_fanout_child_prompt_addresses_the_node_execution() {
        let mut node = make_test_node("review", "Review {{ task.path }}.");
        node.input = vec![InputParam {
            name: "task".to_string(),
            contract: Some("work-item".to_string()),
        }];
        node.artifact = Some("review-result".to_string());
        let resolved = instruction_contents("Review {{ task.path }}.");
        let item = serde_json::json!({"path": "src/lib.rs"});

        let (_system, prompt) = build_fanout_child_prompt(
            &node,
            Some(&resolved),
            None,
            None,
            &HashMap::new(),
            FanoutChildPromptContext::new(Some(&item), "node-execution-1"),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(prompt.contains("releash workflow output submit"));
        assert!(prompt.contains("--node-execution node-execution-1"));
        assert!(prompt.contains("--type review-result"));
    }
}
