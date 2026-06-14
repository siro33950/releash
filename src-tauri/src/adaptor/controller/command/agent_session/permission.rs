use std::sync::Arc;

use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime::{AgentBackendRegistry, AgentProcessMap};
use crate::usecase::agent_session::session::SessionStore;

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPermissionAllowedPrompt {
    pub tool: String,
    pub prompt: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPermissionQuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPermissionQuestion {
    pub question: String,
    pub header: String,
    pub options: Vec<AgentPermissionQuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPermissionPresentation {
    pub kind: String,
    pub can_edit_input: bool,
    pub can_edit_content: bool,
    pub can_edit_multi_edit_content: bool,
    pub direct_content_edit_label: Option<String>,
    pub direct_content: String,
    pub multi_edit_replacement_contents: Vec<String>,
    pub multi_edit_old_strings: Vec<String>,
    pub has_resolved_detail: bool,
    pub plan: String,
    pub allowed_prompts: Vec<AgentPermissionAllowedPrompt>,
    pub questions: Vec<AgentPermissionQuestion>,
}

fn is_edit_preview_tool(tool_name: &str) -> bool {
    matches!(tool_name, "Edit" | "MultiEdit" | "Write")
}

fn is_direct_content_edit_tool(tool_name: &str) -> bool {
    matches!(tool_name, "Edit" | "Write")
}

fn direct_content_edit_label(tool_name: &str) -> Option<String> {
    match tool_name {
        "Write" => Some("Edit file content".to_string()),
        "Edit" => Some("Edit replacement content".to_string()),
        _ => None,
    }
}

fn direct_content_from_input(tool_name: &str, input: &serde_json::Value) -> String {
    let key = if tool_name == "Write" {
        "content"
    } else {
        "new_string"
    };
    input
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn multi_edit_replacement_contents(input: &serde_json::Value) -> Vec<String> {
    input
        .get("edits")
        .and_then(|value| value.as_array())
        .map(|edits| {
            edits
                .iter()
                .map(|edit| {
                    edit.get("new_string")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn multi_edit_old_strings(input: &serde_json::Value) -> Vec<String> {
    input
        .get("edits")
        .and_then(|value| value.as_array())
        .map(|edits| {
            edits
                .iter()
                .map(|edit| {
                    edit.get("old_string")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_allowed_prompts(input: &serde_json::Value) -> Vec<AgentPermissionAllowedPrompt> {
    input
        .get("allowedPrompts")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let tool = item.get("tool")?.as_str()?.to_string();
                    let prompt = item.get("prompt")?.as_str()?.to_string();
                    Some(AgentPermissionAllowedPrompt { tool, prompt })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_ask_questions(input: &serde_json::Value) -> Vec<AgentPermissionQuestion> {
    input
        .get("questions")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let question = item.get("question")?.as_str()?.to_string();
                    let header = item
                        .get("header")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let options = item
                        .get("options")
                        .and_then(|value| value.as_array())
                        .map(|options| {
                            options
                                .iter()
                                .filter_map(|option| {
                                    let label = option.get("label")?.as_str()?.to_string();
                                    let description = option
                                        .get("description")
                                        .and_then(|value| value.as_str())
                                        .unwrap_or_default()
                                        .to_string();
                                    Some(AgentPermissionQuestionOption { label, description })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let multi_select = item
                        .get("multiSelect")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    Some(AgentPermissionQuestion {
                        question,
                        header,
                        options,
                        multi_select,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn input_has_object_fields(input: &serde_json::Value) -> bool {
    input.as_object().is_some_and(|object| !object.is_empty())
}

pub(crate) fn present_agent_permission_request_inner(
    tool_name: &str,
    input: &serde_json::Value,
) -> AgentPermissionPresentation {
    let kind = match tool_name {
        "ExitPlanMode" => "exit_plan",
        "AskUserQuestion" => "ask_user_question",
        _ => "tool",
    };
    let plan = input
        .get("plan")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let allowed_prompts = parse_allowed_prompts(input);
    let questions = parse_ask_questions(input);
    let has_resolved_detail = match kind {
        "exit_plan" => !plan.is_empty() || !allowed_prompts.is_empty(),
        "ask_user_question" => !questions.is_empty(),
        _ => input_has_object_fields(input),
    };

    AgentPermissionPresentation {
        kind: kind.to_string(),
        can_edit_input: is_edit_preview_tool(tool_name),
        can_edit_content: is_direct_content_edit_tool(tool_name),
        can_edit_multi_edit_content: tool_name == "MultiEdit",
        direct_content_edit_label: direct_content_edit_label(tool_name),
        direct_content: direct_content_from_input(tool_name, input),
        multi_edit_replacement_contents: multi_edit_replacement_contents(input),
        multi_edit_old_strings: multi_edit_old_strings(input),
        has_resolved_detail,
        plan,
        allowed_prompts,
        questions,
    }
}

#[tauri::command]
pub fn present_agent_permission_request(
    tool_name: String,
    input: serde_json::Value,
) -> AgentPermissionPresentation {
    present_agent_permission_request_inner(&tool_name, &input)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn respond_agent_permission(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<(), String> {
    crate::infrastructure::agent_session::runtime::respond_agent_permission(
        app,
        session_store,
        handles,
        registry,
        chat_session_id,
        request_id,
        behavior,
        message,
        updated_input,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presents_edit_permission_capabilities() {
        let result = present_agent_permission_request_inner(
            "Edit",
            &serde_json::json!({
                "file_path": "src/main.ts",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        assert_eq!(result.kind, "tool");
        assert!(result.can_edit_input);
        assert!(result.can_edit_content);
        assert!(!result.can_edit_multi_edit_content);
        assert_eq!(
            result.direct_content_edit_label.as_deref(),
            Some("Edit replacement content")
        );
        assert_eq!(result.direct_content, "new");
        assert!(result.has_resolved_detail);
    }

    #[test]
    fn presents_multi_edit_replacements() {
        let result = present_agent_permission_request_inner(
            "MultiEdit",
            &serde_json::json!({
                "edits": [
                    {"old_string": "one", "new_string": "two"},
                    {"old_string": "three", "new_string": "four"}
                ]
            }),
        );

        assert!(result.can_edit_input);
        assert!(!result.can_edit_content);
        assert!(result.can_edit_multi_edit_content);
        assert_eq!(result.multi_edit_old_strings, vec!["one", "three"]);
        assert_eq!(result.multi_edit_replacement_contents, vec!["two", "four"]);
    }

    #[test]
    fn presents_exit_plan_detail() {
        let result = present_agent_permission_request_inner(
            "ExitPlanMode",
            &serde_json::json!({
                "plan": "Plan text",
                "planFilePath": "/tmp/hidden.md",
                "allowedPrompts": [
                    {"tool": "Bash", "prompt": "run tests"}
                ]
            }),
        );

        assert_eq!(result.kind, "exit_plan");
        assert_eq!(result.plan, "Plan text");
        assert_eq!(result.allowed_prompts.len(), 1);
        assert_eq!(result.allowed_prompts[0].prompt, "run tests");
        assert!(result.has_resolved_detail);
    }

    #[test]
    fn presents_ask_user_questions() {
        let result = present_agent_permission_request_inner(
            "AskUserQuestion",
            &serde_json::json!({
                "questions": [{
                    "question": "Pick one",
                    "header": "Choice",
                    "options": [{"label": "A", "description": "Option A"}],
                    "multiSelect": true
                }]
            }),
        );

        assert_eq!(result.kind, "ask_user_question");
        assert_eq!(result.questions.len(), 1);
        assert_eq!(result.questions[0].question, "Pick one");
        assert!(result.questions[0].multi_select);
        assert!(result.has_resolved_detail);
    }
}
