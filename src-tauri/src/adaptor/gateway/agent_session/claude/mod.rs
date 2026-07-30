pub(crate) mod convert;
pub(crate) mod models;
pub(crate) mod permission;
pub(crate) mod process_config;
pub(crate) mod session;
pub(crate) mod skills;
pub(crate) mod wire_conversion;

pub(crate) use models::ClaudeBackend;
#[cfg(test)]
pub(crate) use models::CLAUDE_BACKEND_ID;

pub(crate) use crate::infrastructure::agent_session::claude::{process, wire};
