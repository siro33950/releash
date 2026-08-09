use crate::domain::workflow::FacetContents;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedPrompt {
    pub system_prompt: Option<String>,
    pub user_message: String,
}

pub fn empty_facet_contents() -> &'static FacetContents {
    static EMPTY: std::sync::OnceLock<FacetContents> = std::sync::OnceLock::new();
    EMPTY.get_or_init(FacetContents::default)
}

pub fn compose_facets(resolved: Option<&FacetContents>) -> ComposedPrompt {
    let resolved = match resolved {
        Some(resolved) => resolved,
        None => empty_facet_contents(),
    };
    let mut user_parts = resolved
        .knowledge
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if let Some(content) = resolved.instruction.as_deref() {
        user_parts.push(content);
    }
    ComposedPrompt {
        system_prompt: resolved.policy.clone(),
        user_message: user_parts.join("\n\n"),
    }
}

pub fn provider_tui_initial_instruction(system_prompt: Option<&str>, user_message: &str) -> String {
    match system_prompt.filter(|value| !value.trim().is_empty()) {
        Some(system_prompt) if !user_message.trim().is_empty() => {
            format!("{system_prompt}\n\n{user_message}")
        }
        Some(system_prompt) => system_prompt.to_string(),
        None => user_message.to_string(),
    }
}

pub fn artifact_completion_action(
    key: &str,
    execution_id: &str,
    node_name: &str,
    node_execution_id: Option<&str>,
) -> String {
    let quoted_key = crate::domain::shell::quote_path_for_shell(key);
    let quoted_execution_id = crate::domain::shell::quote_path_for_shell(execution_id);
    let quoted_node_name = crate::domain::shell::quote_path_for_shell(node_name);
    let node_execution_arg = node_execution_id
        .map(crate::domain::shell::quote_path_for_shell)
        .map(|id| format!("  --node-execution {id} \\\n"))
        .unwrap_or_default();
    format!(
        "## 完了時の必須アクション\n\n\
提出値が確定した時点で、次の assistant action は最終応答ではなく CLI 実行でなければならない。\n\
チャット本文に JSON や要約を書いても提出とは扱われない。必ず次のコマンドで Artifact を提出すること。\n\
このコマンドが成功するまで node は完了していない。\n\
成功したら追加の調査やtool実行を行わず、そのturnを終了すること。\n\n\
```sh\n\
releash workflow output submit {execution_id} \\\n  --node {node_name} \\\n{node_execution_arg}  --type {key} \\\n  --json '{{...}}'\n\
```",
        execution_id = quoted_execution_id,
        node_name = quoted_node_name,
        node_execution_arg = node_execution_arg,
        key = quoted_key
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_policy_and_user_facet_parts() {
        let contents = FacetContents {
            policy: Some("policy".into()),
            knowledge: vec!["knowledge".into()],
            instruction: Some("instruction".into()),
        };
        let composed = compose_facets(Some(&contents));
        assert_eq!(composed.system_prompt.as_deref(), Some("policy"));
        assert_eq!(composed.user_message, "knowledge\n\ninstruction");
    }

    #[test]
    fn test_provider_tui初期指示_policyとuser_messageを各一度だけ連結する() {
        let instruction =
            provider_tui_initial_instruction(Some("policy"), "knowledge\ninstruction");

        assert_eq!(instruction, "policy\n\nknowledge\ninstruction");
        assert_eq!(instruction.matches("policy").count(), 1);
        assert_eq!(instruction.matches("instruction").count(), 1);
    }
}
