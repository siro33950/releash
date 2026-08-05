use super::super::ProviderLifecycleInputError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderLifecycleScope {
    agent_session_id: String,
    workflow_execution_id: String,
    node_execution_id: String,
    attempt: u32,
}

impl ProviderLifecycleScope {
    pub(crate) fn new(
        agent_session_id: impl Into<String>,
        workflow_execution_id: impl Into<String>,
        node_execution_id: impl Into<String>,
        attempt: u32,
    ) -> Result<Self, ProviderLifecycleInputError> {
        let agent_session_id = non_empty(agent_session_id.into(), "agent_session_id")?;
        let workflow_execution_id =
            non_empty(workflow_execution_id.into(), "workflow_execution_id")?;
        let node_execution_id = non_empty(node_execution_id.into(), "node_execution_id")?;
        if attempt == 0 {
            return Err(ProviderLifecycleInputError::InvalidAttempt);
        }
        Ok(Self {
            agent_session_id,
            workflow_execution_id,
            node_execution_id,
            attempt,
        })
    }

    pub(crate) fn agent_session_id(&self) -> &str {
        &self.agent_session_id
    }

    pub(crate) fn workflow_execution_id(&self) -> &str {
        &self.workflow_execution_id
    }

    pub(crate) fn node_execution_id(&self) -> &str {
        &self.node_execution_id
    }

    pub(crate) fn attempt(&self) -> u32 {
        self.attempt
    }
}

fn non_empty(value: String, field: &'static str) -> Result<String, ProviderLifecycleInputError> {
    if value.trim().is_empty() {
        Err(ProviderLifecycleInputError::Empty(field))
    } else {
        Ok(value)
    }
}
