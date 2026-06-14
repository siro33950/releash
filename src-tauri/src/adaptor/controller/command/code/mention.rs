//! ファイルメンション候補列挙の Tauri コマンド。
//!
//! 移行前 `file_mention::list_mentionable_files` は同期コマンドであったため、観測可能な
//! 振る舞いを保つよう同期コマンドのまま usecase へ委譲する。

use std::collections::HashMap;

use serde_json::Value;
use tauri::{AppHandle, State};

use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::mention::MentionReferenceInput;
use crate::infrastructure::agent_session::runtime::codex::configured_cli_path;
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    build_fuzzy_file_search_request, CodexAppServerProcess,
};
use crate::other::AppError;

#[tauri::command]
pub fn list_mentionable_files(
    state: State<'_, AppState>,
    worktree_path: String,
    query: String,
) -> Result<Vec<String>, AppError> {
    state
        .code_usecase
        .list_mentionable_files(&worktree_path, &query)
        .map_err(AppError::from)
}

fn normalize_codex_fuzzy_path(root: &str, path: &str) -> Option<String> {
    let root = root.trim_end_matches(['/', '\\']);
    let mut path = path.trim().replace('\\', "/");
    if path.is_empty() {
        return None;
    }
    let normalized_root = root.replace('\\', "/");
    if !normalized_root.is_empty() && path == normalized_root {
        return None;
    }
    if !normalized_root.is_empty() {
        if let Some(stripped) = path.strip_prefix(&(normalized_root.clone() + "/")) {
            path = stripped.to_string();
        }
    }
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

pub(crate) fn codex_fuzzy_file_paths(response: &Value, root: &str, limit: usize) -> Vec<String> {
    let mut seen = HashMap::<String, ()>::new();
    let mut paths = Vec::new();
    let Some(files) = response.get("files").and_then(Value::as_array) else {
        return paths;
    };
    for item in files {
        let match_type = item
            .get("match_type")
            .or_else(|| item.get("matchType"))
            .and_then(Value::as_str);
        if match_type.is_none_or(|value| value != "file" && value != "directory") {
            continue;
        }
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(mut path) = normalize_codex_fuzzy_path(root, path) else {
            continue;
        };
        if match_type.is_some_and(|value| value == "directory") {
            path = format!("{}/", path.trim_end_matches('/'));
        }
        if seen.insert(path.clone(), ()).is_some() {
            continue;
        }
        paths.push(path);
        if paths.len() >= limit {
            break;
        }
    }
    paths
}

#[tauri::command]
pub async fn read_codex_mentionable_files(
    app: AppHandle,
    worktree_path: String,
    query: String,
) -> Result<Vec<String>, String> {
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        process
            .send(&build_fuzzy_file_search_request(id, &worktree_path, &query))
            .await?;
        let response = process.read_response_result(id).await?;
        Ok(codex_fuzzy_file_paths(&response, &worktree_path, 50))
    }
    .await;
    process.shutdown().await;
    result
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

    #[test]
    fn codex_fuzzy_file_paths_keeps_file_and_directory_matches_in_runtime_order() {
        let result = codex_fuzzy_file_paths(
            &serde_json::json!({
                "files": [
                    { "root": "/repo", "path": "/repo/src/main.rs", "match_type": "file" },
                    { "root": "/repo", "path": "src", "match_type": "directory" },
                    { "root": "/repo", "path": "src/main.rs", "match_type": "file" },
                    { "root": "/repo", "path": "src/lib.rs", "matchType": "file" }
                ]
            }),
            "/repo",
            50,
        );

        assert_eq!(result, vec!["src/main.rs", "src/", "src/lib.rs"]);
    }

    #[test]
    fn codex_fuzzy_file_paths_respects_limit() {
        let result = codex_fuzzy_file_paths(
            &serde_json::json!({
                "files": [
                    { "path": "a.rs", "match_type": "file" },
                    { "path": "b.rs", "match_type": "file" }
                ]
            }),
            "/repo",
            1,
        );

        assert_eq!(result, vec!["a.rs"]);
    }
}
