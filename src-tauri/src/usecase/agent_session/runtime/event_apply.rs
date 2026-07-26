use crate::domain::agent_session::entities::{
    MessagePart as DomainMessagePart, PermissionRequest, PermissionRequestBody,
    PermissionRequestStatus, TokenUsage as DomainTokenUsage,
};
#[cfg(test)]
use crate::domain::agent_session::entities::{
    PermissionAllowedPrompt, PermissionQuestion, PermissionQuestionOption,
};
#[cfg(test)]
use crate::domain::agent_session::value_objects::JsonPayload;
use crate::usecase::agent_session::session::{
    MessagePart, PermissionAllowedPromptMsg, PermissionQuestionMsg, PermissionQuestionOptionMsg,
    PermissionRequestKindMsg, PermissionRequestMsg, TokenUsage,
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
    parts
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

#[cfg(test)]
pub(crate) fn pending_permission_request_from_msg(
    msg: &PermissionRequestMsg,
) -> Result<PermissionRequest, String> {
    let body = match msg.kind {
        PermissionRequestKindMsg::ToolApproval => PermissionRequestBody::ToolApproval {
            input: msg
                .input
                .clone()
                .map(json_value_payload)
                .ok_or_else(|| "tool approval permission is missing input".to_string())?,
        },
        PermissionRequestKindMsg::PlanApproval => PermissionRequestBody::PlanApproval {
            plan: msg
                .plan
                .clone()
                .ok_or_else(|| "plan approval permission is missing plan".to_string())?,
            allowed_prompts: msg
                .allowed_prompts
                .iter()
                .map(|prompt| PermissionAllowedPrompt {
                    tool: prompt.tool.clone(),
                    prompt: prompt.prompt.clone(),
                })
                .collect(),
        },
        PermissionRequestKindMsg::Question => PermissionRequestBody::Question {
            questions: msg
                .questions
                .iter()
                .map(|question| PermissionQuestion {
                    question: question.question.clone(),
                    header: question.header.clone(),
                    options: question
                        .options
                        .iter()
                        .map(|option| PermissionQuestionOption {
                            label: option.label.clone(),
                            description: option.description.clone(),
                        })
                        .collect(),
                    multi_select: question.multi_select,
                })
                .collect(),
        },
        PermissionRequestKindMsg::PermissionGrant => PermissionRequestBody::PermissionGrant {
            requested: msg
                .input
                .clone()
                .map(json_value_payload)
                .ok_or_else(|| "permission grant is missing input".to_string())?,
        },
    };
    Ok(PermissionRequest {
        id: msg.id.clone(),
        tool_use_id: msg.tool_use_id.clone(),
        parent_tool_use_id: None,
        tool_name: msg.tool_name.clone(),
        body,
        title: msg.title.clone(),
        display_name: msg.display_name.clone(),
        description: msg.description.clone(),
        decision_reason: msg.decision_reason.clone(),
        status: PermissionRequestStatus::Pending,
    })
}

fn json_payload(payload: &str) -> serde_json::Value {
    serde_json::from_str(payload).expect("domain JsonPayload must be validated at its boundary")
}

#[cfg(test)]
fn json_value_payload(value: serde_json::Value) -> JsonPayload {
    JsonPayload::new_unchecked(
        serde_json::to_string(&value).expect("JSON value serialization cannot fail"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::entities::{
        PermissionAllowedPrompt, PermissionQuestion, PermissionQuestionOption,
    };
    use crate::domain::agent_session::value_objects::JsonPayload;
    use serde_json::json;

    fn request(body: PermissionRequestBody) -> PermissionRequest {
        PermissionRequest {
            id: "req-1".to_string(),
            tool_use_id: Some("tool-1".to_string()),
            // Parent association is owned by MessagePart::Permission, not the
            // compatibility request payload represented by PermissionRequestMsg.
            parent_tool_use_id: None,
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
        let original = request(PermissionRequestBody::ToolApproval {
            input: JsonPayload::new_unchecked(r#"{"cmd":"test"}"#.to_string()),
        });
        let msg = permission_request_msg(&original);

        assert_eq!(msg.kind, PermissionRequestKindMsg::ToolApproval);
        assert_eq!(msg.input, Some(json!({"cmd": "test"})));
        assert_eq!(msg.title.as_deref(), Some("Title"));
        assert_eq!(pending_permission_request_from_msg(&msg).unwrap(), original);
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
    fn permission_request_msg_missing_required_semantics_fails_closed() {
        let mut msg = permission_request_msg(&request(PermissionRequestBody::ToolApproval {
            input: JsonPayload::new_unchecked(r#"{"cmd":"test"}"#.to_string()),
        }));
        msg.input = None;

        assert_eq!(
            pending_permission_request_from_msg(&msg).unwrap_err(),
            "tool approval permission is missing input"
        );
    }
}
