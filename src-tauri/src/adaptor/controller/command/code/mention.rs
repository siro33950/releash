//! ファイルメンション候補列挙の Tauri コマンド。
//!
use std::collections::HashMap;
use std::path::Path;

use tauri::State;

use crate::adaptor::controller::state::AppState;
use crate::adaptor::controller_support::AgentSessionRuntimeState;
use crate::adaptor::protocol::mention::MentionReferenceInput;
use crate::other::AppError;

#[tauri::command]
pub async fn list_mentionable_files(
    state: State<'_, AppState>,
    runtime: State<'_, AgentSessionRuntimeState>,
    worktree_path: String,
    query: String,
    backend_id: Option<String>,
) -> Result<Vec<String>, AppError> {
    match runtime
        .mentionable_files(backend_id.as_deref(), Path::new(&worktree_path), &query, 50)
        .await
    {
        Ok(Some(files)) => return Ok(files),
        Ok(None) => {}
        Err(error) => {
            log::warn!("backend mentionable file search failed; falling back: {error}");
        }
    }
    state
        .code_usecase
        .list_mentionable_files(&worktree_path, &query)
        .map_err(AppError::from)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedMentionToken {
    file_path: String,
    start_line: Option<u32>,
    end_line: Option<u32>,
}

fn parse_mention_line_suffix(text: &str, start: usize) -> (Option<u32>, Option<u32>, usize) {
    let bytes = text.as_bytes();
    let mut cursor = start;
    if bytes.get(cursor) != Some(&b':') || bytes.get(cursor + 1) != Some(&b'L') {
        return (None, None, start);
    }
    cursor += 2;
    let first_start = cursor;
    while bytes.get(cursor).is_some_and(|byte| byte.is_ascii_digit()) {
        cursor += 1;
    }
    if first_start == cursor {
        return (None, None, start);
    }
    let start_line = text[first_start..cursor].parse::<u32>().ok();
    let mut end_line = None;
    if bytes.get(cursor) == Some(&b'-') && bytes.get(cursor + 1) == Some(&b'L') {
        let second_start = cursor + 2;
        cursor = second_start;
        while bytes.get(cursor).is_some_and(|byte| byte.is_ascii_digit()) {
            cursor += 1;
        }
        if second_start < cursor {
            end_line = text[second_start..cursor].parse::<u32>().ok();
        }
    }
    (start_line, end_line, cursor)
}

fn parse_mention_token_at(text: &str, start: usize) -> Option<(ParsedMentionToken, usize)> {
    let bytes = text.as_bytes();
    let mut cursor = start + 1;
    let mut file_path = String::new();
    if bytes.get(cursor) == Some(&b'"') {
        cursor += 1;
        let mut escaped = false;
        let mut closed = false;
        while cursor < text.len() {
            let ch = text[cursor..].chars().next()?;
            let ch_len = ch.len_utf8();
            if escaped {
                file_path.push(ch);
                escaped = false;
                cursor += ch_len;
                continue;
            }
            if ch == '\\' {
                let next = text[cursor + ch_len..].chars().next();
                if matches!(next, Some('"') | Some('\\')) {
                    escaped = true;
                } else {
                    file_path.push(ch);
                }
                cursor += ch_len;
                continue;
            }
            if ch == '"' {
                cursor += ch_len;
                closed = true;
                break;
            }
            file_path.push(ch);
            cursor += ch_len;
        }
        if !closed {
            return None;
        }
    } else {
        while cursor < text.len() {
            let ch = text[cursor..].chars().next()?;
            if ch.is_whitespace() || ch == ':' {
                break;
            }
            file_path.push(ch);
            cursor += ch.len_utf8();
        }
    }
    if file_path.is_empty() {
        return None;
    }
    let (start_line, end_line, end) = parse_mention_line_suffix(text, cursor);
    Some((
        ParsedMentionToken {
            file_path,
            start_line,
            end_line,
        },
        end,
    ))
}

fn parse_mention_tokens(text: &str) -> Vec<ParsedMentionToken> {
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let ch = match text[cursor..].chars().next() {
            Some(ch) => ch,
            None => break,
        };
        if ch != '@' {
            cursor += ch.len_utf8();
            continue;
        }
        if cursor > 0 {
            let previous = text[..cursor].chars().next_back();
            if !previous.is_some_and(char::is_whitespace) {
                cursor += 1;
                continue;
            }
        }
        if let Some((token, end)) = parse_mention_token_at(text, cursor) {
            tokens.push(token);
            cursor = end;
        } else {
            cursor += 1;
        }
    }
    tokens
}

pub(crate) fn sync_mentions_with_text_inner(
    text: &str,
    refs: &[MentionReferenceInput],
) -> Option<Vec<MentionReferenceInput>> {
    let mut available = HashMap::<&str, usize>::new();
    for reference in refs {
        *available.entry(reference.file_path.as_str()).or_default() += 1;
    }
    let mut synced = Vec::new();
    for token in parse_mention_tokens(text) {
        let remaining = available.get_mut(token.file_path.as_str());
        let Some(remaining) = remaining else {
            continue;
        };
        if *remaining == 0 {
            continue;
        }
        *remaining -= 1;
        synced.push(MentionReferenceInput {
            file_path: token.file_path,
            start_line: token.start_line,
            end_line: token.end_line,
        });
    }
    if synced.is_empty() {
        None
    } else {
        Some(synced)
    }
}

#[tauri::command]
pub fn sync_mentions_with_text(
    text: String,
    refs: Vec<MentionReferenceInput>,
) -> Option<Vec<MentionReferenceInput>> {
    sync_mentions_with_text_inner(&text, &refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mention(file_path: &str) -> MentionReferenceInput {
        MentionReferenceInput {
            file_path: file_path.to_string(),
            start_line: None,
            end_line: None,
        }
    }

    #[test]
    fn sync_mentions_with_text_uses_text_order_and_line_ranges() {
        let refs = vec![mention("src/a.ts"), mention("src/a.ts")];
        let result =
            sync_mentions_with_text_inner("Check @src/a.ts:L2-L4 and @src/a.ts:L9", &refs).unwrap();

        assert_eq!(
            result,
            vec![
                MentionReferenceInput {
                    file_path: "src/a.ts".to_string(),
                    start_line: Some(2),
                    end_line: Some(4),
                },
                MentionReferenceInput {
                    file_path: "src/a.ts".to_string(),
                    start_line: Some(9),
                    end_line: None,
                },
            ]
        );
    }

    #[test]
    fn sync_mentions_with_text_supports_quoted_paths() {
        let refs = vec![mention("docs/my file.md")];
        let result = sync_mentions_with_text_inner(r#"Read @"docs/my file.md":L3"#, &refs).unwrap();

        assert_eq!(
            result,
            vec![MentionReferenceInput {
                file_path: "docs/my file.md".to_string(),
                start_line: Some(3),
                end_line: None,
            }]
        );
    }

    #[test]
    fn sync_mentions_with_text_preserves_literal_backslashes_in_quoted_paths() {
        let refs = vec![mention(r#"C:\repo\src\main.rs"#)];
        let result =
            sync_mentions_with_text_inner(r#"Read @"C:\repo\src\main.rs":L10"#, &refs).unwrap();

        assert_eq!(
            result,
            vec![MentionReferenceInput {
                file_path: r#"C:\repo\src\main.rs"#.to_string(),
                start_line: Some(10),
                end_line: None,
            }]
        );
    }

    #[test]
    fn sync_mentions_with_text_excludes_deleted_or_unselected_mentions() {
        assert_eq!(
            sync_mentions_with_text_inner("No mentions", &[mention("a")]),
            None
        );
        assert_eq!(
            sync_mentions_with_text_inner("Check @b", &[mention("a")]),
            None
        );
    }
}
