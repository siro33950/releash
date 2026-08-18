//! Workflow prompt construction.
//!
//! 本文（command / facet）はパラメータ名だけを参照する。値の解決規則
//! （children エントリの inputs 束縛・items 自動束縛）は domain の
//! `reference::resolve_entry_bindings` が所有し、ここでは描画のみを行う。

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::prompt_composition;
use crate::domain::workflow::services::reference;
#[cfg(test)]
use crate::domain::workflow::services::template_preview;
use crate::domain::workflow::FacetContents;
use crate::domain::workflow::{NodeDefinition, SchemaDef};
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

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

/// leaf の束縛済みパラメータからプロンプトを構築する。
/// 束縛の解決規則は domain（実行木スコープ）が所有し、ここでは描画のみを行う。
pub(crate) fn build_leaf_prompt(
    node: &NodeDefinition,
    facet_contents: Option<&FacetContents>,
    node_execution_id: &str,
    bindings: &[(String, Value)],
    schemas: &BTreeMap<String, SchemaDef>,
) -> Result<(Option<String>, String), WorkflowRuntimeError> {
    if !node.has_facet_refs() {
        return Err(WorkflowRuntimeError::InvalidWorkflow(format!(
            "Node '{}' has no facet refs.",
            node.name
        )));
    }
    if facet_contents.is_none_or(FacetContents::is_empty) {
        return Err(WorkflowRuntimeError::InvalidWorkflow(format!(
            "Node '{}' has unresolved facet refs (workflow must go through load pipeline)",
            node.name
        )));
    }
    let composed = prompt_composition::compose_facets(facet_contents);
    let system_prompt = composed
        .system_prompt
        .map(|content| render_parameter_references(&content, bindings));
    let rendered_user = render_parameter_references(&composed.user_message, bindings);
    let mut prompt = inject_input_parameters(&rendered_user, node, bindings);
    append_completion_action(
        &mut prompt,
        node.artifact.as_deref(),
        schemas,
        node_execution_id,
    );
    Ok((system_prompt, prompt))
}
