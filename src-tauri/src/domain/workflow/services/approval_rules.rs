//! Pure approval input validation.

pub const MAX_APPROVAL_COMMENT_CHARS: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalInputError {
    TooLong { label: &'static str, limit: usize },
}

impl std::fmt::Display for ApprovalInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong { label, limit } => write!(f, "{label} exceeds {limit} characters"),
        }
    }
}

impl std::error::Error for ApprovalInputError {}

pub fn validate_optional_comment_text(
    value: Option<&str>,
    label: &'static str,
) -> Result<(), ApprovalInputError> {
    if value.is_some_and(|text| text.chars().count() > MAX_APPROVAL_COMMENT_CHARS) {
        return Err(ApprovalInputError::TooLong {
            label,
            limit: MAX_APPROVAL_COMMENT_CHARS,
        });
    }
    Ok(())
}

#[cfg(test)]
pub fn should_auto_approve_workflow_approval(
    node_is_waiting_approval: bool,
    approval_auto_approve_enabled: bool,
) -> bool {
    approval_auto_approve_enabled && node_is_waiting_approval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_comment_allows_empty_and_rejects_over_limit() {
        assert!(validate_optional_comment_text(Some(""), "Approve comment").is_ok());
        let over = "a".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);
        assert!(matches!(
            validate_optional_comment_text(Some(&over), "Approve comment"),
            Err(ApprovalInputError::TooLong { .. })
        ));
    }
}
