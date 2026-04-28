use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::LazyLock;

static MENTION_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"@([^\s@:]+(?:\.[^\s@:]+)*)(?::L(\d+)(?:-L(\d+))?)?")
        .expect("invalid mention regex")
});

/// A structured file mention reference passed from the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionReference {
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
}

/// A segment of message text for display: either plain text or a @mention.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DisplayPart {
    Text { value: String },
    Mention { value: String },
}

/// List files in a worktree that match a fuzzy query, respecting .gitignore.
/// Returns up to `limit` results (default 50).
#[tauri::command]
pub fn list_mentionable_files(worktree_path: String, query: String) -> Result<Vec<String>, String> {
    let root = Path::new(&worktree_path);
    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize worktree path: {e}"))?;
    if !canonical_root.is_dir() {
        return Err(format!("Not a directory: {worktree_path}"));
    }

    let walker = ignore::WalkBuilder::new(&canonical_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();

    let query_lower = query.to_lowercase();
    let limit = 50usize;
    let collect_limit = if query_lower.is_empty() {
        200
    } else {
        usize::MAX
    };
    let mut results = Vec::new();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        if let Ok(rel) = path.strip_prefix(&canonical_root) {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if query_lower.is_empty() || fuzzy_match(&rel_str.to_lowercase(), &query_lower) {
                results.push(rel_str);
                if results.len() >= collect_limit {
                    break;
                }
            }
        }
    }

    results.sort();
    results.truncate(limit);
    Ok(results)
}

/// Check whether a regex match at `start` is a valid mention position:
/// the `@` must appear at the beginning of the text or after whitespace.
fn is_valid_mention_position(content: &str, start: usize) -> bool {
    start == 0
        || content
            .as_bytes()
            .get(start - 1)
            .is_some_and(|&b| b.is_ascii_whitespace())
}

/// Subsequence fuzzy match: all characters in `query` appear in `haystack` in order.
fn fuzzy_match(haystack: &str, query: &str) -> bool {
    let mut haystack_chars = haystack.chars();
    for q in query.chars() {
        loop {
            match haystack_chars.next() {
                Some(h) if h == q => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

/// Resolve structured mention references into a file_context block prepended to the content.
/// Each mention is read from the filesystem and inserted as a <file> element.
/// If no mentions are provided, returns the content unchanged.
pub fn resolve_from_references(
    worktree_path: &str,
    content: &str,
    mentions: &[MentionReference],
) -> Result<String, String> {
    if mentions.is_empty() {
        return Ok(content.to_string());
    }

    let root = Path::new(worktree_path);
    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve worktree root: {e}"))?;
    let mut file_sections = Vec::new();

    for mention in mentions {
        let file_path = root.join(&mention.file_path);
        let canonical = match file_path.canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !canonical.starts_with(&canonical_root) {
            return Err(format!(
                "Path traversal rejected: {} resolves outside worktree",
                mention.file_path
            ));
        }

        let file_content = match std::fs::read_to_string(&canonical) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let excerpt = extract_excerpt(&file_content, mention.start_line, mention.end_line);

        let attrs = match (mention.start_line, mention.end_line) {
            (Some(s), Some(e)) => format!(r#" path="{}" lines="{}-{}""#, mention.file_path, s, e),
            (Some(s), None) => format!(r#" path="{}" lines="{}""#, mention.file_path, s),
            _ => format!(r#" path="{}""#, mention.file_path),
        };

        file_sections.push(format!("<file{attrs}>\n{excerpt}\n</file>"));
    }

    if file_sections.is_empty() {
        return Ok(content.to_string());
    }

    let context_block = format!(
        "<file_context>\n{}\n</file_context>",
        file_sections.join("\n")
    );

    Ok(format!("{context_block}\n\n{content}"))
}

/// Resolve mentions with logging fallback.
/// Returns the original content unchanged if mentions is empty or resolution fails.
pub fn resolve_mentions_or_fallback(
    worktree_path: &str,
    content: &str,
    mentions: &[MentionReference],
) -> String {
    if mentions.is_empty() {
        return content.to_string();
    }
    resolve_from_references(worktree_path, content, mentions).unwrap_or_else(|e| {
        log::warn!("Failed to resolve mentions: {e}");
        content.to_string()
    })
}

fn extract_excerpt(file_content: &str, start_line: Option<u32>, end_line: Option<u32>) -> String {
    match (start_line, end_line) {
        (Some(start), Some(end)) => {
            let lines: Vec<&str> = file_content.lines().collect();
            if start == 0 || end < start {
                String::new()
            } else {
                let s = (start as usize) - 1;
                let e = (end as usize).min(lines.len());
                if s >= lines.len() || s >= e {
                    String::new()
                } else {
                    lines[s..e].join("\n")
                }
            }
        }
        (Some(start), None) => {
            if start == 0 {
                String::new()
            } else {
                let lines: Vec<&str> = file_content.lines().collect();
                let s = (start as usize) - 1;
                lines.get(s).unwrap_or(&"").to_string()
            }
        }
        _ => file_content.to_string(),
    }
}

/// Parse message text into display parts, splitting @mentions from plain text.
/// Used by the frontend to render mentions as badges without duplicating parse logic.
#[tauri::command]
pub fn parse_display_mentions(content: String) -> Vec<DisplayPart> {
    let mut parts = Vec::new();
    let mut last_index = 0;

    for mat in MENTION_RE.find_iter(&content) {
        let start = mat.start();
        if !is_valid_mention_position(&content, start) {
            continue;
        }
        if start > last_index {
            parts.push(DisplayPart::Text {
                value: content[last_index..start].to_string(),
            });
        }
        parts.push(DisplayPart::Mention {
            value: mat.as_str().to_string(),
        });
        last_index = mat.end();
    }

    if last_index < content.len() {
        parts.push(DisplayPart::Text {
            value: content[last_index..].to_string(),
        });
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fuzzy_match_basic() {
        assert!(fuzzy_match("src/main.rs", "main"));
        assert!(fuzzy_match("src/main.rs", "src/m"));
        assert!(fuzzy_match("src/components/button.tsx", "bttn"));
        assert!(fuzzy_match("src/components/button.tsx", "btn.t"));
        assert!(!fuzzy_match("src/main.rs", "xyz"));
    }

    #[test]
    fn fuzzy_match_empty_query_matches_all() {
        assert!(fuzzy_match("anything.rs", ""));
    }

    // --- resolve_from_references tests ---

    #[test]
    fn resolve_refs_no_mentions_returns_content_unchanged() {
        let result = resolve_from_references("/tmp", "Hello world", &[]).unwrap();
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn resolve_refs_reads_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let mentions = vec![MentionReference {
            file_path: "test.txt".to_string(),
            start_line: None,
            end_line: None,
        }];
        let result =
            resolve_from_references(dir.path().to_str().unwrap(), "Check please", &mentions)
                .unwrap();

        assert!(result.contains("<file_context>"));
        assert!(result.contains(r#"path="test.txt""#));
        assert!(result.contains("line1\nline2\nline3\n"));
        assert!(result.contains("Check please"));
    }

    #[test]
    fn resolve_refs_line_range() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\nline4\nline5\n").unwrap();

        let mentions = vec![MentionReference {
            file_path: "test.txt".to_string(),
            start_line: Some(2),
            end_line: Some(4),
        }];
        let result =
            resolve_from_references(dir.path().to_str().unwrap(), "See file", &mentions).unwrap();

        assert!(result.contains(r#"lines="2-4""#));
        assert!(result.contains("line2\nline3\nline4"));
        assert!(!result.contains("line1"));
        assert!(!result.contains("line5"));
    }

    #[test]
    fn resolve_refs_single_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let mentions = vec![MentionReference {
            file_path: "test.txt".to_string(),
            start_line: Some(2),
            end_line: None,
        }];
        let result =
            resolve_from_references(dir.path().to_str().unwrap(), "Look at file", &mentions)
                .unwrap();

        assert!(result.contains(r#"lines="2""#));
        assert!(result.contains("line2"));
    }

    #[test]
    fn resolve_refs_japanese_filename() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Gitフロー.md");
        fs::write(&file, "日本語の内容\n2行目\n").unwrap();

        let mentions = vec![MentionReference {
            file_path: "Gitフロー.md".to_string(),
            start_line: None,
            end_line: None,
        }];
        let result =
            resolve_from_references(dir.path().to_str().unwrap(), "確認してください", &mentions)
                .unwrap();

        assert!(result.contains("<file_context>"));
        assert!(result.contains(r#"path="Gitフロー.md""#));
        assert!(result.contains("日本語の内容"));
    }

    #[test]
    fn resolve_refs_missing_file_skips() {
        let dir = tempfile::tempdir().unwrap();
        let mentions = vec![MentionReference {
            file_path: "nonexistent.txt".to_string(),
            start_line: None,
            end_line: None,
        }];
        let result =
            resolve_from_references(dir.path().to_str().unwrap(), "Check", &mentions).unwrap();
        assert_eq!(result, "Check");
    }

    #[test]
    fn resolve_refs_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().parent().unwrap();
        let sibling = tempfile::tempdir_in(parent).unwrap();
        let outside_file = sibling.path().join("outside.txt");
        fs::write(&outside_file, "secret").unwrap();

        let sibling_name = sibling.path().file_name().unwrap().to_str().unwrap();
        let mentions = vec![MentionReference {
            file_path: format!("../{sibling_name}/outside.txt"),
            start_line: None,
            end_line: None,
        }];
        let result = resolve_from_references(dir.path().to_str().unwrap(), "Check", &mentions);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("traversal"), "Error message: {err}");
    }

    // --- list_mentionable_files tests ---

    #[test]
    fn list_mentionable_files_basic() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("world.txt"), "hello").unwrap();

        git2::Repository::init(dir.path()).unwrap();

        let result =
            list_mentionable_files(dir.path().to_str().unwrap().to_string(), String::new())
                .unwrap();

        assert!(result.contains(&"hello.rs".to_string()));
        assert!(result.contains(&"world.txt".to_string()));
    }

    #[test]
    fn list_mentionable_files_fuzzy_filter() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "").unwrap();
        fs::write(dir.path().join("lib.rs"), "").unwrap();
        fs::write(dir.path().join("readme.md"), "").unwrap();

        git2::Repository::init(dir.path()).unwrap();

        let result =
            list_mentionable_files(dir.path().to_str().unwrap().to_string(), "mn".to_string())
                .unwrap();

        assert!(result.contains(&"main.rs".to_string()));
        assert!(!result.contains(&"lib.rs".to_string()));
    }

    // --- parse_display_mentions tests ---

    #[test]
    fn parse_display_mentions_no_mentions() {
        let parts = parse_display_mentions("Hello world".to_string());
        assert_eq!(
            parts,
            vec![DisplayPart::Text {
                value: "Hello world".to_string()
            }]
        );
    }

    #[test]
    fn parse_display_mentions_single() {
        let parts = parse_display_mentions("Check @src/main.rs please".to_string());
        assert_eq!(
            parts,
            vec![
                DisplayPart::Text {
                    value: "Check ".to_string()
                },
                DisplayPart::Mention {
                    value: "@src/main.rs".to_string()
                },
                DisplayPart::Text {
                    value: " please".to_string()
                },
            ]
        );
    }

    #[test]
    fn parse_display_mentions_with_line_range() {
        let parts = parse_display_mentions("See @src/lib.rs:L10-L20".to_string());
        assert_eq!(
            parts,
            vec![
                DisplayPart::Text {
                    value: "See ".to_string()
                },
                DisplayPart::Mention {
                    value: "@src/lib.rs:L10-L20".to_string()
                },
            ]
        );
    }

    #[test]
    fn parse_display_mentions_multiple() {
        let parts = parse_display_mentions("Compare @a.rs and @b.rs end".to_string());
        assert_eq!(
            parts,
            vec![
                DisplayPart::Text {
                    value: "Compare ".to_string()
                },
                DisplayPart::Mention {
                    value: "@a.rs".to_string()
                },
                DisplayPart::Text {
                    value: " and ".to_string()
                },
                DisplayPart::Mention {
                    value: "@b.rs".to_string()
                },
                DisplayPart::Text {
                    value: " end".to_string()
                },
            ]
        );
    }

    #[test]
    fn parse_display_mentions_empty_string() {
        let parts = parse_display_mentions(String::new());
        assert!(parts.is_empty());
    }

    #[test]
    fn parse_display_mentions_ignores_email() {
        let parts = parse_display_mentions("user@example.com says hello".to_string());
        assert_eq!(
            parts,
            vec![DisplayPart::Text {
                value: "user@example.com says hello".to_string()
            }]
        );
    }

    #[test]
    fn parse_display_mentions_mixed_email_and_mention() {
        let parts =
            parse_display_mentions("From user@example.com see @src/main.rs end".to_string());
        assert_eq!(
            parts,
            vec![
                DisplayPart::Text {
                    value: "From user@example.com see ".to_string()
                },
                DisplayPart::Mention {
                    value: "@src/main.rs".to_string()
                },
                DisplayPart::Text {
                    value: " end".to_string()
                },
            ]
        );
    }

    #[test]
    fn parse_display_mentions_japanese_filename() {
        let parts = parse_display_mentions("確認 @docs/Gitフロー.md してください".to_string());
        assert_eq!(
            parts,
            vec![
                DisplayPart::Text {
                    value: "確認 ".to_string()
                },
                DisplayPart::Mention {
                    value: "@docs/Gitフロー.md".to_string()
                },
                DisplayPart::Text {
                    value: " してください".to_string()
                },
            ]
        );
    }

    #[test]
    fn fallback_empty_mentions() {
        let result = resolve_mentions_or_fallback("/tmp", "Hello world", &[]);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn fallback_with_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\n").unwrap();

        let mentions = vec![MentionReference {
            file_path: "test.txt".to_string(),
            start_line: None,
            end_line: None,
        }];
        let result = resolve_mentions_or_fallback(dir.path().to_str().unwrap(), "Check", &mentions);
        assert!(result.contains("<file_context>"));
        assert!(result.contains("Check"));
    }

    #[test]
    fn fallback_missing_file_returns_content() {
        let dir = tempfile::tempdir().unwrap();
        let mentions = vec![MentionReference {
            file_path: "nonexistent.txt".to_string(),
            start_line: None,
            end_line: None,
        }];
        let result = resolve_mentions_or_fallback(dir.path().to_str().unwrap(), "Check", &mentions);
        assert_eq!(result, "Check");
    }
}
