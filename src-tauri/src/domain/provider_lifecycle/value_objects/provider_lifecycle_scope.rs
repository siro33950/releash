use super::super::ProviderLifecycleInputError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderLifecycleScope {
    agent_session_id: String,
}

impl ProviderLifecycleScope {
    pub(crate) fn new(
        agent_session_id: impl Into<String>,
    ) -> Result<Self, ProviderLifecycleInputError> {
        let agent_session_id = non_empty(agent_session_id.into(), "agent_session_id")?;
        Ok(Self { agent_session_id })
    }

    pub(crate) fn agent_session_id(&self) -> &str {
        &self.agent_session_id
    }
}

fn non_empty(value: String, field: &'static str) -> Result<String, ProviderLifecycleInputError> {
    if value.trim().is_empty() {
        Err(ProviderLifecycleInputError::Empty(field))
    } else {
        Ok(value)
    }
}
