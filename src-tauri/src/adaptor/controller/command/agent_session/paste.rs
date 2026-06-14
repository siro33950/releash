use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PastedTextBlock {
    pub id: usize,
    pub placeholder: String,
    pub content: String,
}

const LONG_PASTE_CHAR_THRESHOLD: usize = 1200;
const LONG_PASTE_LINE_THRESHOLD: usize = 20;

fn normalize_paste_index(index: usize) -> usize {
    index.max(1)
}

pub(crate) fn should_collapse_pasted_text(content: &str) -> bool {
    if content.trim().is_empty() {
        return false;
    }
    content.chars().count() >= LONG_PASTE_CHAR_THRESHOLD
        || content.lines().count() >= LONG_PASTE_LINE_THRESHOLD
}

pub(crate) fn build_pasted_text_block(
    index: usize,
    content: String,
) -> Result<PastedTextBlock, String> {
    if content.trim().is_empty() {
        return Err("Pasted text is empty".to_string());
    }
    let id = normalize_paste_index(index);
    Ok(PastedTextBlock {
        id,
        placeholder: format!("[Pasted text #{}]", id),
        content,
    })
}

pub(crate) fn expand_pasted_blocks(
    content: String,
    blocks: Vec<PastedTextBlock>,
) -> Result<String, String> {
    let mut expanded = content;
    for block in blocks {
        if block.placeholder.trim().is_empty() {
            return Err("Pasted text placeholder is empty".to_string());
        }
        if !expanded.contains(&block.placeholder) {
            continue;
        }
        let replacement = format!(
            "{}\n\n<pasted_text id=\"{}\">\n{}\n</pasted_text>",
            block.placeholder,
            block.id,
            escape_pasted_text_content(&block.content)
        );
        expanded = expanded.replace(&block.placeholder, &replacement);
    }
    Ok(expanded)
}

fn escape_pasted_text_content(content: &str) -> String {
    content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[tauri::command]
pub fn prepare_pasted_text_block(
    index: usize,
    content: String,
) -> Result<Option<PastedTextBlock>, String> {
    if !should_collapse_pasted_text(&content) {
        return Ok(None);
    }
    build_pasted_text_block(index, content).map(Some)
}

#[tauri::command]
pub fn expand_pasted_text_blocks(
    content: String,
    blocks: Vec<PastedTextBlock>,
) -> Result<String, String> {
    expand_pasted_blocks(content, blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_pasted_text_block_uses_readable_placeholder() {
        let block = build_pasted_text_block(2, "alpha\nbeta".to_string()).unwrap();

        assert_eq!(block.id, 2);
        assert_eq!(block.placeholder, "[Pasted text #2]");
        assert_eq!(block.content, "alpha\nbeta");
    }

    #[test]
    fn should_collapse_pasted_text_uses_rust_thresholds() {
        assert!(!should_collapse_pasted_text("short text"));
        assert!(should_collapse_pasted_text(
            &std::iter::repeat_n("line", LONG_PASTE_LINE_THRESHOLD)
                .collect::<Vec<_>>()
                .join("\n")
        ));
        assert!(should_collapse_pasted_text(
            &"x".repeat(LONG_PASTE_CHAR_THRESHOLD)
        ));
    }

    #[test]
    fn prepare_pasted_text_block_leaves_short_text_unmodified() {
        let block = prepare_pasted_text_block(1, "short text".to_string()).unwrap();

        assert_eq!(block, None);
    }

    #[test]
    fn build_pasted_text_block_rejects_blank_content() {
        let err = build_pasted_text_block(1, " \n\t ".to_string()).unwrap_err();

        assert_eq!(err, "Pasted text is empty");
    }

    #[test]
    fn expand_pasted_blocks_replaces_placeholders_with_full_content() {
        let block = build_pasted_text_block(1, "line 1\nline 2".to_string()).unwrap();
        let expanded =
            expand_pasted_blocks("Please inspect [Pasted text #1]".to_string(), vec![block])
                .unwrap();

        assert_eq!(
            expanded,
            "Please inspect [Pasted text #1]\n\n<pasted_text id=\"1\">\nline 1\nline 2\n</pasted_text>"
        );
    }

    #[test]
    fn expand_pasted_blocks_escapes_xml_like_boundaries() {
        let block =
            build_pasted_text_block(1, "alpha\n</pasted_text>\n& beta".to_string()).unwrap();
        let expanded =
            expand_pasted_blocks("Please inspect [Pasted text #1]".to_string(), vec![block])
                .unwrap();

        assert!(expanded.contains("&lt;/pasted_text&gt;"));
        assert!(expanded.contains("&amp; beta"));
        assert!(!expanded.contains("\n</pasted_text>\n& beta"));
    }

    #[test]
    fn expand_pasted_blocks_ignores_removed_placeholder() {
        let block = build_pasted_text_block(1, "line 1".to_string()).unwrap();
        let expanded = expand_pasted_blocks("No marker".to_string(), vec![block]).unwrap();

        assert_eq!(expanded, "No marker");
    }
}
