//! Runtime output retention policy.

pub const MAX_OUTPUT_SIZE: usize = 100 * 1024; // 100KB
pub const TRUNCATION_MARKER: &str = "... (truncated)";

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
#[path = "output_limit_test.rs"]
mod output_limit_tests;
