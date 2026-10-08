use super::super::ProviderLifecycleInputError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderLifecycleSlotId(String);

impl ProviderLifecycleSlotId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProviderLifecycleInputError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ProviderLifecycleInputError::Empty("slot_id"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
