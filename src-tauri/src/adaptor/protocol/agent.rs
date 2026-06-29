use serde::{Deserialize, Serialize};

use crate::usecase::agent_session::session::ContextCarryState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSupportedCommandMsg {
    pub name: String,
    pub description: String,
    #[serde(
        rename = "argumentHint",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSupportedCommandsUpdated {
    pub chat_session_id: String,
    pub commands: Vec<AgentSupportedCommandMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionContextCarryUpdated {
    pub chat_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_carry: Option<ContextCarryState>,
    pub updated_at: f64,
}
