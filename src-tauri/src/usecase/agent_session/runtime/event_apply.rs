use crate::domain::agent_session::entities::{
    PermissionAllowedPrompt, PermissionQuestion, PermissionQuestionOption, PermissionRequest,
    PermissionRequestBody, PermissionRequestStatus,
};
use crate::domain::agent_session::value_objects::JsonPayload;
use crate::usecase::agent_session::session::{PermissionRequestKindMsg, PermissionRequestMsg};

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
    use crate::usecase::agent_session::runtime::ports::AgentRuntimeProjectionGateway;
    use serde_json::json;

    fn permission_request_msg(request: &PermissionRequest) -> PermissionRequestMsg {
        crate::adaptor::gateway::agent_session::runtime_projection::AgentRuntimeProjectionGatewayV1
            .permission_request(request)
    }

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
