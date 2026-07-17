use crate::domain::agent_session::entities::{
    MessagePart as DomainMessagePart, PermissionDecision as DomainPermissionDecision,
    PermissionRequest, PermissionRequestBody, PermissionRequestStatus,
    TokenUsage as DomainTokenUsage,
};
use crate::domain::agent_session::value_objects::{
    SystemNotificationType as DomainSystemNotificationType, ToolOutputRef as DomainToolOutputRef,
    ToolOutputSummary as DomainToolOutputSummary,
};
use crate::usecase::agent_session::session::{
    MessagePart, PermissionAllowedPromptMsg, PermissionPartStatus, PermissionQuestionMsg,
    PermissionQuestionOptionMsg, PermissionRequestKindMsg, PermissionRequestMsg,
    SystemNotificationType, TokenUsage, ToolOutputRef, ToolOutputSummary,
};

pub(crate) fn token_usage_from_domain(usage: DomainTokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        context_window_tokens: usage.context_window_tokens,
    }
}

pub(crate) fn parts_from_domain(parts: Vec<DomainMessagePart>) -> Vec<MessagePart> {
    parts.into_iter().map(part_from_domain).collect()
}

pub(crate) fn permission_request_msg(request: &PermissionRequest) -> PermissionRequestMsg {
    let mut msg = PermissionRequestMsg {
        id: request.id.clone(),
        tool_use_id: request.tool_use_id.clone(),
        tool_name: request.tool_name.clone(),
        kind: PermissionRequestKindMsg::ToolApproval,
        input: None,
        plan: None,
        allowed_prompts: Vec::new(),
        questions: Vec::new(),
        title: request.title.clone(),
        display_name: request.display_name.clone(),
        description: request.description.clone(),
        decision_reason: request.decision_reason.clone(),
    };
    match &request.body {
        PermissionRequestBody::ToolApproval { input } => {
            msg.kind = PermissionRequestKindMsg::ToolApproval;
            msg.input = Some(json_payload(input.as_str()));
        }
        PermissionRequestBody::PlanApproval {
            plan,
            allowed_prompts,
        } => {
            msg.kind = PermissionRequestKindMsg::PlanApproval;
            msg.plan = Some(plan.clone());
            msg.allowed_prompts = allowed_prompts
                .iter()
                .map(|prompt| PermissionAllowedPromptMsg {
                    tool: prompt.tool.clone(),
                    prompt: prompt.prompt.clone(),
                })
                .collect();
        }
        PermissionRequestBody::Question { questions } => {
            msg.kind = PermissionRequestKindMsg::Question;
            msg.questions = questions
                .iter()
                .map(|question| PermissionQuestionMsg {
                    question: question.question.clone(),
                    header: question.header.clone(),
                    options: question
                        .options
                        .iter()
                        .map(|option| PermissionQuestionOptionMsg {
                            label: option.label.clone(),
                            description: option.description.clone(),
                        })
                        .collect(),
                    multi_select: question.multi_select,
                })
                .collect();
        }
        PermissionRequestBody::PermissionGrant { requested } => {
            msg.kind = PermissionRequestKindMsg::PermissionGrant;
            msg.input = Some(json_payload(requested.as_str()));
        }
    }
    msg
}

pub(crate) fn pending_permission_request_msg(
    request: &PermissionRequest,
) -> Option<PermissionRequestMsg> {
    matches!(request.status, PermissionRequestStatus::Pending)
        .then(|| permission_request_msg(request))
}

fn part_from_domain(part: DomainMessagePart) -> MessagePart {
    match part {
        DomainMessagePart::Thinking {
            content,
            parent_tool_use_id,
        } => MessagePart::Thinking {
            content,
            parent_tool_use_id,
        },
        DomainMessagePart::Text {
            content,
            parent_tool_use_id,
        } => MessagePart::Text {
            content,
            parent_tool_use_id,
        },
        DomainMessagePart::ToolUse {
            id,
            tool,
            input,
            parent_tool_use_id,
        } => MessagePart::ToolUse {
            id,
            tool,
            input: json_payload(input.as_str()),
            parent_tool_use_id,
        },
        DomainMessagePart::ToolResult {
            content,
            is_error,
            tool_use_id,
            parent_tool_use_id,
            content_ref,
            summary,
        } => MessagePart::ToolResult {
            content,
            is_error,
            tool_use_id,
            parent_tool_use_id,
            content_ref: content_ref.map(tool_output_ref_from_domain),
            summary: summary.map(tool_output_summary_from_domain),
        },
        DomainMessagePart::Error {
            content,
            parent_tool_use_id,
        } => MessagePart::Error {
            content,
            parent_tool_use_id,
        },
        DomainMessagePart::Permission { request } => {
            let (status, answers) = permission_status_and_answers(&request.status);
            MessagePart::Permission {
                parent_tool_use_id: request.parent_tool_use_id.clone(),
                request: permission_request_msg(&request),
                status,
                answers,
            }
        }
        DomainMessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } => MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        },
        DomainMessagePart::TodoListSnapshot { items } => MessagePart::TodoListSnapshot {
            items: items
                .into_iter()
                .map(
                    |item| crate::usecase::agent_session::session::TodoListItem {
                        text: item.text,
                        completed: item.completed,
                    },
                )
                .collect(),
        },
        DomainMessagePart::SystemNotification {
            notification_type,
            status,
            label,
            detail,
            hook_id,
        } => MessagePart::SystemNotification {
            notification_type: system_notification_type_from_domain(notification_type),
            status,
            label,
            detail,
            hook_id,
        },
        DomainMessagePart::Image { data, media_type } => MessagePart::Image { data, media_type },
        DomainMessagePart::ImageRef { attachment } => MessagePart::ImageRef {
            attachment: crate::usecase::agent_session::session::AttachmentRef {
                id: attachment.id,
                media_type: attachment.media_type,
                byte_size: attachment.byte_size,
            },
        },
    }
}

fn json_payload(payload: &str) -> serde_json::Value {
    serde_json::from_str(payload).unwrap_or_else(|_| serde_json::Value::String(payload.to_string()))
}

fn permission_status_and_answers(
    status: &PermissionRequestStatus,
) -> (PermissionPartStatus, Option<serde_json::Value>) {
    match status {
        PermissionRequestStatus::Pending => (PermissionPartStatus::Pending, None),
        PermissionRequestStatus::Resolved { decision, answers } => {
            let status = match decision {
                DomainPermissionDecision::Allowed => PermissionPartStatus::Allowed,
                DomainPermissionDecision::Denied => PermissionPartStatus::Denied,
                DomainPermissionDecision::Cancelled => PermissionPartStatus::Cancelled,
            };
            (
                status,
                answers
                    .as_ref()
                    .map(|payload| json_payload(payload.as_str())),
            )
        }
    }
}

fn tool_output_ref_from_domain(value: DomainToolOutputRef) -> ToolOutputRef {
    ToolOutputRef {
        id: value.id,
        byte_size: value.byte_size,
    }
}

fn tool_output_summary_from_domain(value: DomainToolOutputSummary) -> ToolOutputSummary {
    ToolOutputSummary {
        line_count: value.line_count,
        byte_size: value.byte_size,
        is_error: value.is_error,
        truncated: value.truncated,
    }
}

fn system_notification_type_from_domain(
    value: DomainSystemNotificationType,
) -> SystemNotificationType {
    match value {
        DomainSystemNotificationType::Compaction => SystemNotificationType::Compaction,
        DomainSystemNotificationType::SessionRecovery => SystemNotificationType::SessionRecovery,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::entities::{
        PermissionAllowedPrompt, PermissionDecision, PermissionQuestion, PermissionQuestionOption,
    };
    use crate::domain::agent_session::value_objects::JsonPayload;
    use serde_json::json;

    fn request(body: PermissionRequestBody) -> PermissionRequest {
        PermissionRequest {
            id: "req-1".to_string(),
            tool_use_id: Some("tool-1".to_string()),
            parent_tool_use_id: Some("parent-1".to_string()),
            tool_name: "Tool".to_string(),
            body,
            title: Some("Title".to_string()),
            display_name: Some("Display".to_string()),
            description: Some("Description".to_string()),
            decision_reason: Some("Reason".to_string()),
            status: PermissionRequestStatus::Pending,
        }
    }

    #[test]
    fn permission_request_msg_maps_tool_approval() {
        let msg = permission_request_msg(&request(PermissionRequestBody::ToolApproval {
            input: JsonPayload::new_unchecked(r#"{"cmd":"test"}"#.to_string()),
        }));

        assert_eq!(msg.kind, PermissionRequestKindMsg::ToolApproval);
        assert_eq!(msg.input, Some(json!({"cmd": "test"})));
        assert_eq!(msg.title.as_deref(), Some("Title"));
    }

    #[test]
    fn permission_request_msg_maps_plan_approval() {
        let msg = permission_request_msg(&request(PermissionRequestBody::PlanApproval {
            plan: "run tests".to_string(),
            allowed_prompts: vec![PermissionAllowedPrompt {
                tool: "Bash".to_string(),
                prompt: "cargo test".to_string(),
            }],
        }));

        assert_eq!(msg.kind, PermissionRequestKindMsg::PlanApproval);
        assert_eq!(msg.plan.as_deref(), Some("run tests"));
        assert_eq!(msg.allowed_prompts[0].tool, "Bash");
    }

    #[test]
    fn permission_request_msg_maps_question() {
        let msg = permission_request_msg(&request(PermissionRequestBody::Question {
            questions: vec![PermissionQuestion {
                question: "Pick".to_string(),
                header: Some("Choice".to_string()),
                options: vec![PermissionQuestionOption {
                    label: "A".to_string(),
                    description: Some("Option A".to_string()),
                }],
                multi_select: true,
            }],
        }));

        assert_eq!(msg.kind, PermissionRequestKindMsg::Question);
        assert_eq!(msg.questions[0].question, "Pick");
        assert!(msg.questions[0].multi_select);
    }

    #[test]
    fn permission_request_msg_maps_permission_grant() {
        let msg = permission_request_msg(&request(PermissionRequestBody::PermissionGrant {
            requested: JsonPayload::new_unchecked(r#"{"scope":"workspace"}"#.to_string()),
        }));

        assert_eq!(msg.kind, PermissionRequestKindMsg::PermissionGrant);
        assert_eq!(msg.input, Some(json!({"scope": "workspace"})));
    }

    #[test]
    fn permission_status_maps_resolved_decisions() {
        for (decision, expected) in [
            (PermissionDecision::Allowed, PermissionPartStatus::Allowed),
            (PermissionDecision::Denied, PermissionPartStatus::Denied),
            (
                PermissionDecision::Cancelled,
                PermissionPartStatus::Cancelled,
            ),
        ] {
            let (status, answers) =
                permission_status_and_answers(&PermissionRequestStatus::Resolved {
                    decision,
                    answers: Some(JsonPayload::new_unchecked(r#"{"ok":true}"#.to_string())),
                });

            assert_eq!(status, expected);
            assert_eq!(answers, Some(json!({"ok": true})));
        }
    }

    #[test]
    fn json_payload_falls_back_to_string_when_parse_fails() {
        let msg = permission_request_msg(&request(PermissionRequestBody::ToolApproval {
            input: JsonPayload::new_unchecked("not json".to_string()),
        }));

        assert_eq!(msg.input, Some(json!("not json")));
    }
}
