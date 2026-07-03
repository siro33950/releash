use serde_json::{json, Value};

use crate::domain::agent_session::entities::{
    PermissionAllowedPrompt, PermissionQuestion, PermissionQuestionOption, PermissionRequest,
    PermissionRequestBody, PermissionRequestStatus, PermissionResponse, PermissionResponseDecision,
};
use crate::domain::agent_session::value_objects::JsonPayload;

use super::wire::{ClaudeWireMode, TYPE_CONTROL_RESPONSE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClaudePermissionAction {
    Respond { response: Value },
    Prompt { request: Box<PermissionRequest> },
}

pub(crate) fn should_auto_allow(mode: ClaudeWireMode, tool_name: &str) -> bool {
    match mode {
        ClaudeWireMode::BypassPermissions => true,
        ClaudeWireMode::AcceptEdits | ClaudeWireMode::Plan => !is_interactive_tool(tool_name),
        ClaudeWireMode::Default => false,
    }
}

pub(crate) fn is_interactive_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "AskUserQuestion" | "EnterPlanMode" | "ExitPlanMode"
    )
}

pub(crate) fn permission_action_from_can_use_tool(
    request_id: &str,
    request: &Value,
    mode: ClaudeWireMode,
) -> Option<ClaudePermissionAction> {
    let Some(tool_name) = request.get("tool_name").and_then(Value::as_str) else {
        return Some(ClaudePermissionAction::Respond {
            response: deny_control_response(
                request_id,
                "Malformed permission request: missing tool_name".to_string(),
            ),
        });
    };
    let tool_name = tool_name.to_string();
    let input = request.get("input").cloned().unwrap_or_else(|| json!({}));
    if should_auto_allow(mode, &tool_name) {
        return Some(ClaudePermissionAction::Respond {
            response: allow_control_response(request_id, Some(input), None),
        });
    }

    Some(ClaudePermissionAction::Prompt {
        request: Box::new(permission_request_from_can_use_tool(request_id, request)),
    })
}

fn permission_request_from_can_use_tool(request_id: &str, request: &Value) -> PermissionRequest {
    let tool_name = request
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("Tool")
        .to_string();
    let input = request.get("input").cloned().unwrap_or_else(|| json!({}));
    PermissionRequest {
        id: request_id.to_string(),
        tool_use_id: string_field(request, "tool_use_id"),
        parent_tool_use_id: string_field(request, "agent_id"),
        tool_name: tool_name.clone(),
        body: permission_body(&tool_name, input),
        title: string_field(request, "title"),
        display_name: string_field(request, "display_name"),
        description: string_field(request, "description"),
        decision_reason: string_field(request, "decision_reason"),
        status: PermissionRequestStatus::Pending,
    }
}

fn permission_body(tool_name: &str, input: Value) -> PermissionRequestBody {
    match tool_name {
        "ExitPlanMode" => PermissionRequestBody::PlanApproval {
            plan: input
                .get("plan")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            allowed_prompts: allowed_prompts_from_input(&input),
        },
        "AskUserQuestion" => PermissionRequestBody::Question {
            questions: questions_from_input(&input),
        },
        _ => PermissionRequestBody::ToolApproval {
            input: JsonPayload::new_unchecked(input.to_string()),
        },
    }
}

fn allowed_prompts_from_input(input: &Value) -> Vec<PermissionAllowedPrompt> {
    let prompts = input
        .get("allowedPrompts")
        .or_else(|| input.get("allowed_prompts"))
        .and_then(Value::as_array);
    prompts
        .into_iter()
        .flatten()
        .filter_map(|prompt| {
            Some(PermissionAllowedPrompt {
                tool: prompt.get("tool")?.as_str()?.to_string(),
                prompt: prompt
                    .get("prompt")
                    .or_else(|| prompt.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect()
}

fn questions_from_input(input: &Value) -> Vec<PermissionQuestion> {
    let Some(questions) = input.get("questions").and_then(Value::as_array) else {
        return Vec::new();
    };
    questions
        .iter()
        .map(|question| PermissionQuestion {
            question: question
                .get("question")
                .or_else(|| question.get("prompt"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            header: string_field(question, "header"),
            options: question
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|option| {
                    Some(PermissionQuestionOption {
                        label: option.get("label")?.as_str()?.to_string(),
                        description: string_field(option, "description"),
                    })
                })
                .collect(),
            multi_select: question
                .get("multi_select")
                .or_else(|| question.get("multiSelect"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
        .collect()
}

pub(crate) fn claude_permission_response(
    response: PermissionResponse,
    original_input: Option<Value>,
) -> Result<Value, String> {
    match response.decision {
        PermissionResponseDecision::Allow {
            updated_input,
            answers,
        } => {
            let original_input_for_questions = original_input.clone();
            let updated_input = match updated_input {
                Some(payload) => Some(serde_json::from_str::<Value>(payload.as_str()).map_err(
                    |error| format!("invalid updated_input JSON for Claude permission: {error}"),
                )?),
                None => original_input,
            };
            let answers = answers
                .map(|payload| {
                    serde_json::from_str::<Value>(payload.as_str()).map_err(|error| {
                        format!("invalid answers JSON for Claude permission: {error}")
                    })
                })
                .transpose()?;
            let updated_input = if answers.is_some() {
                Some(updated_input_with_question_fallback(
                    updated_input,
                    original_input_for_questions.as_ref(),
                ))
            } else {
                updated_input
            };
            Ok(allow_control_response(
                response.request_id,
                updated_input,
                answers,
            ))
        }
        PermissionResponseDecision::Deny { message } => Ok(deny_control_response(
            response.request_id,
            message.unwrap_or_else(|| "User denied".to_string()),
        )),
    }
}

fn updated_input_with_question_fallback(
    updated_input: Option<Value>,
    original_input: Option<&Value>,
) -> Value {
    let mut input = updated_input.unwrap_or_else(|| json!({}));
    let has_questions = input
        .get("questions")
        .and_then(Value::as_array)
        .is_some_and(|questions| !questions.is_empty());
    if !has_questions {
        let questions = original_input
            .and_then(|input| input.get("questions"))
            .cloned()
            .unwrap_or_else(|| json!([]));
        if let Some(object) = input.as_object_mut() {
            object.insert("questions".to_string(), questions);
        } else {
            input = json!({ "questions": questions });
        }
    }
    input
}

pub(crate) fn allow_control_response(
    request_id: impl Into<String>,
    updated_input: Option<Value>,
    answers: Option<Value>,
) -> Value {
    let mut response = json!({ "behavior": "allow" });
    if let Some(updated_input) = updated_input {
        response["updatedInput"] = updated_input;
    }
    if let Some(answers) = answers {
        let questions = response
            .get("updatedInput")
            .and_then(|input| input.get("questions"))
            .cloned()
            .unwrap_or_else(|| json!([]));
        response["updatedInput"] = json!({
            "questions": questions,
            "answers": answers,
        });
    }
    control_success_response(request_id, response)
}

pub(crate) fn deny_control_response(request_id: impl Into<String>, message: String) -> Value {
    control_success_response(
        request_id,
        json!({
            "behavior": "deny",
            "message": message,
        }),
    )
}

fn control_success_response(request_id: impl Into<String>, response: Value) -> Value {
    json!({
        "type": TYPE_CONTROL_RESPONSE,
        "response": {
            "subtype": "success",
            "request_id": request_id.into(),
            "response": response,
        },
    })
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_allow_tableは_design_6_3に従う() {
        for tool in ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"] {
            assert!(should_auto_allow(ClaudeWireMode::BypassPermissions, tool));
            assert!(!should_auto_allow(ClaudeWireMode::AcceptEdits, tool));
            assert!(!should_auto_allow(ClaudeWireMode::Plan, tool));
            assert!(!should_auto_allow(ClaudeWireMode::Default, tool));
        }
        for tool in ["Bash", "Edit", "Read"] {
            assert!(should_auto_allow(ClaudeWireMode::BypassPermissions, tool));
            assert!(should_auto_allow(ClaudeWireMode::AcceptEdits, tool));
            assert!(should_auto_allow(ClaudeWireMode::Plan, tool));
            assert!(!should_auto_allow(ClaudeWireMode::Default, tool));
        }
    }

    #[test]
    fn test_can_use_tool_exit_planは_plan_approvalへ変換する() {
        let action = permission_action_from_can_use_tool(
            "req-1",
            &json!({
                "subtype": "can_use_tool",
                "tool_name": "ExitPlanMode",
                "tool_use_id": "tool-1",
                "input": {
                    "plan": "Do it",
                    "allowedPrompts": [{ "tool": "Bash", "prompt": "cargo test" }]
                }
            }),
            ClaudeWireMode::Default,
        )
        .unwrap();

        let ClaudePermissionAction::Prompt { request } = action else {
            panic!("expected prompt");
        };
        assert_eq!(request.id, "req-1");
        assert_eq!(request.tool_name, "ExitPlanMode");
        assert!(matches!(
            &request.body,
            PermissionRequestBody::PlanApproval { .. }
        ));
    }

    #[test]
    fn test_claude_permission_response_allowは_updated_inputを送る() {
        let value = claude_permission_response(
            PermissionResponse {
                request_id: "req-1".to_string(),
                decision: PermissionResponseDecision::Allow {
                    updated_input: Some(JsonPayload::new_unchecked(
                        r#"{"command":"cargo test"}"#.to_string(),
                    )),
                    answers: None,
                },
            },
            None,
        )
        .unwrap();

        assert_eq!(value["type"], TYPE_CONTROL_RESPONSE);
        assert_eq!(
            value["response"]["response"]["updatedInput"]["command"],
            "cargo test"
        );
    }

    #[test]
    fn test_claude_permission_response_question_answersは_updated_inputへ合成する() {
        let value = claude_permission_response(
            PermissionResponse {
                request_id: "req-q".to_string(),
                decision: PermissionResponseDecision::Allow {
                    updated_input: None,
                    answers: Some(JsonPayload::new_unchecked(
                        r#"[{"question":"Pick one","answer":"A"}]"#.to_string(),
                    )),
                },
            },
            Some(json!({
                "questions": [
                    {
                        "question": "Pick one",
                        "options": [{ "label": "A" }, { "label": "B" }]
                    }
                ],
                "ignored": "not echoed"
            })),
        )
        .unwrap();

        let response = &value["response"]["response"];
        assert_eq!(response["behavior"], "allow");
        assert!(response.get("answers").is_none());
        assert_eq!(
            response["updatedInput"],
            json!({
                "questions": [
                    {
                        "question": "Pick one",
                        "options": [{ "label": "A" }, { "label": "B" }]
                    }
                ],
                "answers": [{ "question": "Pick one", "answer": "A" }]
            })
        );
    }

    #[test]
    fn test_claude_permission_response_question_answersは空_updated_inputでも元_questionsを復元する(
    ) {
        let value = claude_permission_response(
            PermissionResponse {
                request_id: "req-q".to_string(),
                decision: PermissionResponseDecision::Allow {
                    updated_input: Some(JsonPayload::new_unchecked("{}".to_string())),
                    answers: Some(JsonPayload::new_unchecked(
                        r#"[{"question":"Pick one","answer":"A"}]"#.to_string(),
                    )),
                },
            },
            Some(json!({
                "questions": [
                    {
                        "question": "Pick one",
                        "options": [{ "label": "A" }, { "label": "B" }]
                    }
                ]
            })),
        )
        .unwrap();

        assert_eq!(
            value["response"]["response"]["updatedInput"],
            json!({
                "questions": [
                    {
                        "question": "Pick one",
                        "options": [{ "label": "A" }, { "label": "B" }]
                    }
                ],
                "answers": [{ "question": "Pick one", "answer": "A" }]
            })
        );
    }

    #[test]
    fn test_can_use_tool_ask_user_questionは_question_requestへ変換する() {
        let action = permission_action_from_can_use_tool(
            "req-q",
            &json!({
                "subtype": "can_use_tool",
                "tool_name": "AskUserQuestion",
                "input": {
                    "questions": [
                        {
                            "question": "Pick one",
                            "header": "Choice",
                            "multiSelect": false,
                            "options": [{ "label": "A", "description": "first" }]
                        }
                    ]
                }
            }),
            ClaudeWireMode::Default,
        )
        .unwrap();

        let ClaudePermissionAction::Prompt { request } = action else {
            panic!("expected prompt");
        };
        let PermissionRequestBody::Question { questions } = &request.body else {
            panic!("expected question body");
        };
        assert_eq!(questions[0].question, "Pick one");
        assert_eq!(questions[0].header.as_deref(), Some("Choice"));
        assert_eq!(questions[0].options[0].label, "A");
    }

    #[test]
    fn test_can_use_tool_tool_name欠落は_deny応答を返す() {
        let action = permission_action_from_can_use_tool(
            "req-bad",
            &json!({ "subtype": "can_use_tool", "input": {} }),
            ClaudeWireMode::Default,
        )
        .unwrap();

        let ClaudePermissionAction::Respond { response } = action else {
            panic!("expected immediate response");
        };
        assert_eq!(response["response"]["response"]["behavior"], "deny");
    }
}
