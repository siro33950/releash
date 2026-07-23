use std::sync::Arc;

use crate::adaptor::protocol::agent_session_v1::{
    OperationApplicationErrorDtoV1, PermissionResponseCommandErrorDtoV1,
    PermissionResponseCommandOutcomeDtoV1, PermissionResponseLookupErrorDtoV1,
    PermissionResponseOperationViewDtoV1,
};
use crate::domain::agent_session::entities::{PermissionResponse, PermissionResponseDecision};
use crate::domain::agent_session::value_objects::JsonPayload;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{PermissionRequestKindMsg, PermissionRequestMsg};

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

fn present_agent_permission_request_from_msg(
    request: &PermissionRequestMsg,
) -> AgentPermissionPresentation {
    let input = request
        .input
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));
    let mut presentation = present_agent_permission_request_inner(&request.tool_name, &input);
    match request.kind {
        PermissionRequestKindMsg::PlanApproval => {
            presentation.kind = "exit_plan".to_string();
            presentation.plan = request.plan.clone().unwrap_or_default();
            presentation.allowed_prompts = request
                .allowed_prompts
                .iter()
                .map(|prompt| AgentPermissionAllowedPrompt {
                    tool: prompt.tool.clone(),
                    prompt: prompt.prompt.clone(),
                })
                .collect();
            presentation.has_resolved_detail =
                !presentation.plan.is_empty() || !presentation.allowed_prompts.is_empty();
        }
        PermissionRequestKindMsg::Question => {
            presentation.kind = "ask_user_question".to_string();
            presentation.questions = request
                .questions
                .iter()
                .map(|question| AgentPermissionQuestion {
                    question: question.question.clone(),
                    header: question.header.clone().unwrap_or_default(),
                    options: question
                        .options
                        .iter()
                        .map(|option| AgentPermissionQuestionOption {
                            label: option.label.clone(),
                            description: option.description.clone().unwrap_or_default(),
                        })
                        .collect(),
                    multi_select: question.multi_select,
                })
                .collect();
            presentation.has_resolved_detail = !presentation.questions.is_empty();
        }
        PermissionRequestKindMsg::ToolApproval | PermissionRequestKindMsg::PermissionGrant => {}
    }
    presentation
}

#[tauri::command]
pub async fn present_agent_permission_request(
    runtime: tauri::State<'_, Arc<AgentSessionRuntimeUsecase>>,
    chat_session_id: String,
    request_id: String,
) -> Result<AgentPermissionPresentation, String> {
    let request = runtime
        .find_permission_request(&chat_session_id, &request_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Permission request not found: {request_id}"))?;
    Ok(present_agent_permission_request_from_msg(&request))
}

#[tauri::command]
pub async fn report_agent_permission_request_observed(
    runtime: tauri::State<'_, Arc<AgentSessionRuntimeUsecase>>,
    chat_session_id: String,
    request_id: String,
    visible: bool,
) -> Result<(), String> {
    runtime
        .report_permission_request_observed(&chat_session_id, &request_id, visible)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn respond_agent_permission(
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    operation: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::operation::PermissionResponseOperationUsecase>,
    >,
    journal: tauri::State<'_, Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>>,
    operation_id: String,
    chat_session_id: String,
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<PermissionResponseCommandOutcomeDtoV1, PermissionResponseCommandErrorDtoV1> {
    dispatch_durable_permission_response(
        store.inner().as_ref(),
        operation.inner().as_ref(),
        journal.inner().as_ref(),
        operation_id,
        chat_session_id,
        request_id,
        behavior,
        message,
        updated_input,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_durable_permission_response(
    store: &crate::adaptor::gateway::local_event_store::LocalEventStore,
    operation: &crate::usecase::agent_session::operation::PermissionResponseOperationUsecase,
    journal: &crate::usecase::agent_session::operation::CallerAttemptJournal,
    operation_id: String,
    chat_session_id: String,
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<PermissionResponseCommandOutcomeDtoV1, PermissionResponseCommandErrorDtoV1> {
    let response = permission_response_from_command(
        request_id.clone(),
        behavior.clone(),
        message.clone(),
        updated_input.clone(),
    )
    .map_err(|_| PermissionResponseCommandErrorDtoV1::InvalidRequest)?;
    let journal_command = serde_json::json!({
        "schema": "tauri_permission_response_attempt_v1",
        "session_id": chat_session_id,
        "request_id": request_id,
        "behavior": behavior,
        "message": message,
        "updated_input": updated_input,
    })
    .to_string();

    // Saved operations remain replayable after admission closes. The request
    // call below still verifies the immutable exact payload binding.
    let replaying_existing = match operation
        .get_operation(super::session::TAURI_OPERATION_PRINCIPAL, &operation_id)
        .await
    {
        Ok(_) => true,
        Err(
            crate::usecase::agent_session::operation::GetPermissionResponseOperationError::NotFound,
        ) => {
            super::session::ensure_mutation_admission(store)
                .await
                .map_err(permission_admission_error)?;
            false
        }
        Err(
            crate::usecase::agent_session::operation::GetPermissionResponseOperationError::InvalidRequest,
        ) => return Err(PermissionResponseCommandErrorDtoV1::InvalidRequest),
        Err(_) => {
            return Ok(PermissionResponseCommandOutcomeDtoV1::OutcomeUnknown {
                operation_id,
            });
        }
    };
    if !replaying_existing {
        match journal
            .record_attempt_scoped(
                super::session::TAURI_OPERATION_PRINCIPAL,
                crate::domain::local_event::OperationKind::PermissionResponse,
                &operation_id,
                journal_command.as_bytes(),
                Some(&chat_session_id),
            )
            .await
        {
            Ok(_) => {}
            Err(crate::usecase::agent_session::operation::CallerJournalError::OutcomeUnknown) => {
                return Ok(PermissionResponseCommandOutcomeDtoV1::OutcomeUnknown { operation_id });
            }
            Err(
                crate::usecase::agent_session::operation::CallerJournalError::RejectedBeforeCommit,
            ) => {
                return Ok(
                    PermissionResponseCommandOutcomeDtoV1::RejectedBeforeCommit {
                        failure: super::session::caller_journal_failure(
                            "The local caller attempt could not be saved.",
                        )
                        .into(),
                    },
                );
            }
            Err(error) => {
                return Err(permission_admission_error(
                    super::session::caller_journal_application_error(error),
                ));
            }
        }
    }
    let outcome = operation
        .request(
            crate::usecase::agent_session::operation::PermissionResponseOperationRequest {
                principal: super::session::TAURI_OPERATION_PRINCIPAL.to_string(),
                operation_id: operation_id.clone(),
                session_id: chat_session_id,
                response,
            },
        )
        .await
        .map_err(crate::adaptor::presenter::agent_session::permission_response_command_error)?;
    match &outcome {
        crate::usecase::agent_session::operation::PermissionResponseCommandOutcome::Accepted(_) => {
            if let Err(error) = journal
                    .resolve_attempt_if_present(
                    super::session::TAURI_OPERATION_PRINCIPAL,
                    crate::domain::local_event::OperationKind::PermissionResponse,
                    &operation_id,
                    journal_command.as_bytes(),
                    true,
                )
                .await
            {
                log::warn!("permission response caller journal clear failed: {error:?}");
            }
        }
        crate::usecase::agent_session::operation::PermissionResponseCommandOutcome::RejectedBeforeCommit { .. } => {
            if let Err(error) = journal
                    .resolve_attempt_if_present(
                        super::session::TAURI_OPERATION_PRINCIPAL,
                        crate::domain::local_event::OperationKind::PermissionResponse,
                        &operation_id,
                        journal_command.as_bytes(),
                        false,
                    )
                    .await
            {
                log::warn!(
                    "rejected permission response caller journal clear failed: {error:?}"
                );
            }
        }
        crate::usecase::agent_session::operation::PermissionResponseCommandOutcome::OutcomeUnknown { .. } => {}
    }
    Ok(crate::adaptor::presenter::agent_session::permission_response_outcome(outcome))
}

fn permission_admission_error(
    error: OperationApplicationErrorDtoV1,
) -> PermissionResponseCommandErrorDtoV1 {
    match error {
        OperationApplicationErrorDtoV1::MigrationInProgress => {
            PermissionResponseCommandErrorDtoV1::MigrationInProgress
        }
        OperationApplicationErrorDtoV1::ShutdownInProgress => {
            PermissionResponseCommandErrorDtoV1::ShutdownInProgress
        }
        OperationApplicationErrorDtoV1::FeedbackCapacityExceeded => {
            PermissionResponseCommandErrorDtoV1::FeedbackCapacityExceeded
        }
        OperationApplicationErrorDtoV1::Internal { correlation_id } => {
            PermissionResponseCommandErrorDtoV1::Internal { correlation_id }
        }
        other => PermissionResponseCommandErrorDtoV1::Internal {
            correlation_id: format!("permission-admission-{other:?}"),
        },
    }
}

#[tauri::command]
pub async fn get_agent_permission_response_operation(
    operation: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::operation::PermissionResponseOperationUsecase>,
    >,
    operation_id: String,
) -> Result<PermissionResponseOperationViewDtoV1, PermissionResponseLookupErrorDtoV1> {
    operation
        .get_operation(super::session::TAURI_OPERATION_PRINCIPAL, &operation_id)
        .await
        .map(crate::adaptor::presenter::agent_session::permission_response_operation)
        .map_err(crate::adaptor::presenter::agent_session::permission_response_lookup_error)
}

pub(crate) fn permission_response_from_command(
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<PermissionResponse, String> {
    match behavior.as_str() {
        "allow" => {
            if message.is_some() {
                return Err("Allow permission responses cannot include a deny message".to_string());
            }
            let (updated_input, answers) = split_updated_input_and_answers(updated_input)?;
            Ok(PermissionResponse {
                request_id,
                decision: PermissionResponseDecision::Allow {
                    updated_input,
                    answers,
                },
            })
        }
        "deny" => {
            if updated_input.is_some() {
                return Err("Deny permission responses cannot include updated_input".to_string());
            }
            Ok(PermissionResponse {
                request_id,
                decision: PermissionResponseDecision::Deny { message },
            })
        }
        other => Err(format!("Unsupported permission behavior: {other}")),
    }
}

fn split_updated_input_and_answers(
    updated_input: Option<String>,
) -> Result<(Option<JsonPayload>, Option<JsonPayload>), String> {
    let Some(updated_input) = updated_input else {
        return Ok((None, None));
    };
    let mut value = serde_json::from_str::<serde_json::Value>(&updated_input)
        .map_err(|error| format!("Invalid updated_input JSON: {error}"))?;
    let answers = value
        .as_object_mut()
        .and_then(|object| object.remove("answers"))
        .map(|answers| JsonPayload::new_unchecked(answers.to_string()));
    Ok((Some(JsonPayload::new_unchecked(value.to_string())), answers))
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

    #[test]
    fn permission_response_allow_splits_answers_from_updated_input() {
        let response = permission_response_from_command(
            "req-1".to_string(),
            "allow".to_string(),
            None,
            Some(r#"{"command":"test","answers":{"choice":"yes"}}"#.to_string()),
        )
        .unwrap();

        let PermissionResponseDecision::Allow {
            updated_input,
            answers,
        } = response.decision
        else {
            panic!("expected allow response");
        };
        assert_eq!(response.request_id, "req-1");
        assert_eq!(updated_input.unwrap().as_str(), r#"{"command":"test"}"#);
        assert_eq!(answers.unwrap().as_str(), r#"{"choice":"yes"}"#);
    }

    #[test]
    fn permission_response_deny_preserves_message() {
        let response = permission_response_from_command(
            "req-2".to_string(),
            "deny".to_string(),
            Some("no".to_string()),
            None,
        )
        .unwrap();

        assert_eq!(
            response.decision,
            PermissionResponseDecision::Deny {
                message: Some("no".to_string())
            }
        );
    }

    #[test]
    fn permission_response_rejects_unsupported_behavior() {
        let error =
            permission_response_from_command("req-3".to_string(), "maybe".to_string(), None, None)
                .unwrap_err();

        assert!(error.contains("Unsupported permission behavior"));
    }

    #[test]
    fn permission_response_rejects_fields_outside_the_selected_decision() {
        assert!(permission_response_from_command(
            "req-allow".to_string(),
            "allow".to_string(),
            Some("not an allow field".to_string()),
            None,
        )
        .is_err());
        assert!(permission_response_from_command(
            "req-deny".to_string(),
            "deny".to_string(),
            Some("no".to_string()),
            Some("{}".to_string()),
        )
        .is_err());
    }

    #[test]
    fn split_updated_input_rejects_invalid_json() {
        let error = split_updated_input_and_answers(Some("{".to_string())).unwrap_err();

        assert!(error.contains("Invalid updated_input JSON"));
    }
}
