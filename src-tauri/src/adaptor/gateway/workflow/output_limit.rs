pub(crate) const MAX_OUTPUT_SIZE: usize = 100 * 1024; // 100KB
pub(crate) const TRUNCATION_MARKER: &str = "... (truncated)";

pub(crate) fn truncate_output(text: String) -> String {
    if text.len() <= MAX_OUTPUT_SIZE {
        return text;
    }
    let mut end = MAX_OUTPUT_SIZE;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_string();
    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

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
