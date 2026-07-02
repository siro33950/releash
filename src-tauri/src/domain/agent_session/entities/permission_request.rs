use crate::domain::agent_session::value_objects::JsonPayload;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub id: String,
    pub tool_use_id: Option<String>,
    pub parent_tool_use_id: Option<String>,
    pub tool_name: String,
    pub body: PermissionRequestBody,
    pub title: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub decision_reason: Option<String>,
    pub status: PermissionRequestStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRequestBody {
    ToolApproval {
        input: JsonPayload,
    },
    PlanApproval {
        plan: String,
        allowed_prompts: Vec<PermissionAllowedPrompt>,
    },
    Question {
        questions: Vec<PermissionQuestion>,
    },
    PermissionGrant {
        requested: JsonPayload,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionAllowedPrompt {
    pub tool: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionQuestion {
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<PermissionQuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionQuestionOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRequestStatus {
    Pending,
    Resolved {
        decision: PermissionDecision,
        answers: Option<JsonPayload>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allowed,
    Denied,
    #[allow(dead_code)]
    // issues-1301 G-1: cancellation is part of the domain permission vocabulary; current runtime resolves aborts as turn interruptions.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionResponse {
    pub request_id: String,
    pub decision: PermissionResponseDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionResponseDecision {
    Allow {
        updated_input: Option<JsonPayload>,
        answers: Option<JsonPayload>,
    },
    Deny {
        message: Option<String>,
    },
}
