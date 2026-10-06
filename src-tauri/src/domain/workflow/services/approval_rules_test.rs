pub(crate) mod tests {
    use super::super::*;

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
