use crate::domain::agent_session::aggregates::session::TerminalOutcome;
use crate::domain::agent_session::entities::{
    InterruptReason, PermissionRequest, PermissionRequestBody, TokenUsage, TurnResult,
    TurnStopReason,
};
use crate::domain::agent_session::value_objects::TurnPhase;
use crate::usecase::agent_session::event_log::{
    AgentTurnFailureSignal, InterruptReason as EventInterruptReason,
    TurnStopReason as EventTurnStopReason, TurnTokenUsage, WorkflowTurnCompleteInput,
};
use crate::usecase::agent_session::runtime::ports::{
    AgentRuntimeProjectionGateway, AgentStallObservedPayload, TerminalEventProjection,
    TerminalProjection,
};
use crate::usecase::agent_session::session::{
    PermissionAllowedPromptMsg, PermissionQuestionMsg, PermissionQuestionOptionMsg,
    PermissionRequestKindMsg, PermissionRequestMsg, TokenUsage as TokenUsageProjection,
};
use crate::usecase::workflow::ports::{
    WorkflowStallClearedNotification, WorkflowStallObservedNotification,
    WorkflowTurnCompleteNotification, WorkflowTurnFailureSignal, WorkflowTurnTokenUsage,
};

#[derive(Debug, Default)]
pub(crate) struct AgentRuntimeProjectionGatewayV1;

impl AgentRuntimeProjectionGateway for AgentRuntimeProjectionGatewayV1 {
    fn terminal_projection(
        &self,
        result: &TurnResult,
        outcome: TerminalOutcome,
    ) -> TerminalProjection {
        let event = match result {
            TurnResult::Completed {
                stop_reason,
                token_usage,
            } => TerminalEventProjection::Completed {
                stop_reason: stop_reason.map(|reason| match reason {
                    TurnStopReason::Refusal => EventTurnStopReason::Refusal,
                }),
                token_usage: token_usage.map(turn_token_usage),
            },
            TurnResult::Failed { token_usage, .. } => TerminalEventProjection::Completed {
                stop_reason: None,
                token_usage: token_usage.map(turn_token_usage),
            },
            TurnResult::Interrupted { reason, error } => TerminalEventProjection::Interrupted {
                reason: match reason {
                    InterruptReason::Abort => EventInterruptReason::Abort,
                    InterruptReason::Timeout => EventInterruptReason::Timeout,
                    InterruptReason::Crash => EventInterruptReason::Crash,
                    InterruptReason::SessionClosed => EventInterruptReason::SessionClosed,
                },
                error: error.clone(),
            },
        };
        TerminalProjection {
            exit_code: outcome.exit_code,
            interrupted: outcome.interrupted,
            pause_queue: outcome.pause_queue,
            session_state: outcome.session_state,
            event,
        }
    }

    fn token_usage(&self, usage: TokenUsage) -> TokenUsageProjection {
        TokenUsageProjection {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            context_window_tokens: usage.context_window_tokens,
        }
    }

    fn permission_request(&self, request: &PermissionRequest) -> PermissionRequestMsg {
        let mut projection = PermissionRequestMsg {
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
                projection.input = Some(json_payload(input.as_str()));
            }
            PermissionRequestBody::PlanApproval {
                plan,
                allowed_prompts,
            } => {
                projection.kind = PermissionRequestKindMsg::PlanApproval;
                projection.plan = Some(plan.clone());
                projection.allowed_prompts = allowed_prompts
                    .iter()
                    .map(|prompt| PermissionAllowedPromptMsg {
                        tool: prompt.tool.clone(),
                        prompt: prompt.prompt.clone(),
                    })
                    .collect();
            }
            PermissionRequestBody::Question { questions } => {
                projection.kind = PermissionRequestKindMsg::Question;
                projection.questions = questions
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
                projection.kind = PermissionRequestKindMsg::PermissionGrant;
                projection.input = Some(json_payload(requested.as_str()));
            }
        }
        projection
    }

    fn pending_permission_request(
        &self,
        request: &PermissionRequest,
    ) -> Option<PermissionRequestMsg> {
        request
            .is_pending()
            .then(|| self.permission_request(request))
    }

    fn workflow_turn_complete(
        &self,
        session_id: &str,
        input: &WorkflowTurnCompleteInput,
    ) -> WorkflowTurnCompleteNotification {
        WorkflowTurnCompleteNotification {
            chat_session_id: session_id.to_string(),
            exit_code: input.exit_code,
            final_text_parts: input.final_text_parts.clone(),
            failure_signal: input.failure_signal.map(|signal| match signal {
                AgentTurnFailureSignal::ModelRefusal => WorkflowTurnFailureSignal::ModelRefusal,
            }),
            token_usage: input.token_usage.map(|usage| WorkflowTurnTokenUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
            }),
            interrupted: input.interrupted,
        }
    }

    fn workflow_stall_observed(
        &self,
        payload: &AgentStallObservedPayload,
    ) -> WorkflowStallObservedNotification {
        WorkflowStallObservedNotification {
            chat_session_id: payload.chat_session_id.clone(),
            turn_phase: match payload.turn_phase {
                TurnPhase::Idle => "idle",
                TurnPhase::Streaming => "streaming",
                TurnPhase::WaitingPermission => "waiting_permission",
            }
            .to_string(),
            idle_secs: payload.idle_secs,
            signal_count: payload.signal_count,
            cap_reached: payload.cap_reached,
        }
    }

    fn workflow_stall_cleared(&self, session_id: &str) -> WorkflowStallClearedNotification {
        WorkflowStallClearedNotification {
            chat_session_id: session_id.to_string(),
        }
    }
}

fn turn_token_usage(usage: TokenUsage) -> TurnTokenUsage {
    TurnTokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

fn json_payload(payload: &str) -> serde_json::Value {
    serde_json::from_str(payload).expect("domain JsonPayload must be validated at its boundary")
}
