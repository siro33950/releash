pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn truncate_output_within_limit() {
        let text = "hello".to_string();
        assert_eq!(truncate_output(text), "hello");
    }

    #[test]
    fn truncate_output_exceeds_limit_ascii() {
        let text = "a".repeat(MAX_OUTPUT_SIZE + 100);
        let result = truncate_output(text);
        assert!(result.ends_with("... (truncated)"));
        assert!(result.len() <= MAX_OUTPUT_SIZE + 20);
    }

    #[test]
    fn truncate_output_multibyte_boundary() {
        let text = "あ".repeat(MAX_OUTPUT_SIZE);
        let result = truncate_output(text);
        assert!(result.ends_with("... (truncated)"));
        assert!(!result.is_empty());
    }
}
