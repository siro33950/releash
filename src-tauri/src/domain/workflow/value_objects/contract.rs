use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContractType(String);

impl ContractType {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(WorkflowError::validation("contract type must not be empty"));
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractValidationResult {
    Valid {
        artifact: serde_json::Value,
        result: Option<String>,
    },
    Invalid(ContractViolation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractViolation {
    pub reason: String,
    pub details: String,
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn test_contract_type_emptyを拒否する() {
        assert!(ContractType::new("spec-directory").is_ok());
        assert!(ContractType::new(" ").is_err());
    }
}
