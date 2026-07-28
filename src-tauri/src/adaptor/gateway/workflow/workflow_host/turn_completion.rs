//! Turn-completion input helpers.

#[cfg(test)]
use crate::usecase::agent_session::session::MessagePart;

/// Extracts user-visible text from agent message parts for workflow turn completion.
#[cfg(test)]
pub(crate) fn extract_text_from_parts(parts: &[MessagePart]) -> String {
    let mut text = String::new();
    for part in parts {
        if let MessagePart::Text { content, .. } = part {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(content);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_from_parts_combines_text_parts() {
        let parts = vec![
            MessagePart::Thinking {
                content: "thinking...".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "First line".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "Second line".to_string(),
                parent_tool_use_id: None,
            },
        ];

        assert_eq!(extract_text_from_parts(&parts), "First line\nSecond line");
    }

    #[test]
    fn extract_text_from_parts_empty() {
        let parts: Vec<MessagePart> = vec![];

        assert_eq!(extract_text_from_parts(&parts), "");
    }
}
