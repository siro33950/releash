#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionDisplayName(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSessionDisplayNameError {
    Empty,
}

impl AgentSessionDisplayName {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, AgentSessionDisplayNameError> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            return Err(AgentSessionDisplayNameError::Empty);
        }
        Ok(Self(value.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "display_name_test.rs"]
mod display_name_tests;
