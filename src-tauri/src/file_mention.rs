use serde::Serialize;
use std::path::Path;
use std::sync::LazyLock;

static MENTION_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"@([\w./_\-\[\]]+(?:\.[\w]+)*)(?::L(\d+)(?:-L(\d+))?)?")
        .expect("invalid mention regex")
});

/// A single file mention parsed from message text.
#[derive(Debug, Clone, PartialEq)]
struct ParsedMention {
    file_path: String,
    start_line: Option<u32>,
    end_line: Option<u32>,
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
            let rel_str = rel.to_string_lossy().to_string();
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

/// Parse mention patterns from message text.
/// Recognized formats:
/// - `@filepath`
/// - `@filepath:L10-L20`
/// - `@filepath:L10` (single line)
fn parse_mentions(content: &str) -> Vec<ParsedMention> {
    let mut mentions = Vec::new();
    for caps in MENTION_RE.captures_iter(content) {
        let mat = caps.get(0).unwrap();
        if !is_valid_mention_position(content, mat.start()) {
            continue;
        }
        let file_path = caps[1].to_string();
        let start_line = caps.get(2).and_then(|m| m.as_str().parse::<u32>().ok());
        let end_line = caps.get(3).and_then(|m| m.as_str().parse::<u32>().ok());
        mentions.push(ParsedMention {
            file_path,
            start_line,
            end_line,
        });
    }
    mentions
}

/// Resolve all @mentions in `content`:
/// 1. Parse mentions
/// 2. Read each file (or line range)
/// 3. Build a prompt with <file_context> + original message
///
/// If there are no mentions, returns the content unchanged.
pub fn resolve_mentions_internal(worktree_path: &str, content: &str) -> Result<String, String> {
    let mentions = parse_mentions(content);
    if mentions.is_empty() {
        return Ok(content.to_string());
    }

    let root = Path::new(worktree_path);
    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve worktree root: {e}"))?;
    let mut file_sections = Vec::new();

    for mention in &mentions {
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

        let excerpt = match (mention.start_line, mention.end_line) {
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
                let lines: Vec<&str> = file_content.lines().collect();
                let s = (start as usize).saturating_sub(1);
                lines.get(s).unwrap_or(&"").to_string()
            }
            _ => file_content,
        };

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

/// Resolve @mentions with fallback: if resolution fails or worktree_path is empty,
/// log and return the original content unchanged.
pub fn resolve_mentions_or_fallback(worktree_path: &str, content: &str) -> String {
    if worktree_path.is_empty() {
        return content.to_string();
    }
    resolve_mentions_internal(worktree_path, content).unwrap_or_else(|e| {
        log::warn!("Failed to resolve mentions: {e}");
        content.to_string()
    })
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

    #[test]
    fn parse_mentions_no_mentions() {
        let result = parse_mentions("Hello world");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_mentions_simple_path() {
        let result = parse_mentions("Check @src/main.rs for details");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_path, "src/main.rs");
        assert_eq!(result[0].start_line, None);
        assert_eq!(result[0].end_line, None);
    }

    #[test]
    fn parse_mentions_with_line_range() {
        let result = parse_mentions("See @src/lib.rs:L10-L20");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_path, "src/lib.rs");
        assert_eq!(result[0].start_line, Some(10));
        assert_eq!(result[0].end_line, Some(20));
    }

    #[test]
    fn parse_mentions_single_line() {
        let result = parse_mentions("Look at @src/lib.rs:L42");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_path, "src/lib.rs");
        assert_eq!(result[0].start_line, Some(42));
        assert_eq!(result[0].end_line, None);
    }

    #[test]
    fn parse_mentions_multiple() {
        let result = parse_mentions("Compare @src/a.rs and @src/b.rs:L1-L5");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].file_path, "src/a.rs");
        assert_eq!(result[1].file_path, "src/b.rs");
        assert_eq!(result[1].start_line, Some(1));
        assert_eq!(result[1].end_line, Some(5));
    }

    #[test]
    fn resolve_mentions_no_mentions_returns_content_unchanged() {
        let result = resolve_mentions_internal("/tmp", "Hello world").unwrap();
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn resolve_mentions_reads_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let result =
            resolve_mentions_internal(dir.path().to_str().unwrap(), "Check @test.txt please")
                .unwrap();

        assert!(result.contains("<file_context>"));
        assert!(result.contains(r#"path="test.txt""#));
        assert!(result.contains("line1\nline2\nline3\n"));
        assert!(result.contains("Check @test.txt please"));
    }

    #[test]
    fn resolve_mentions_line_range() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\nline4\nline5\n").unwrap();

        let result =
            resolve_mentions_internal(dir.path().to_str().unwrap(), "See @test.txt:L2-L4").unwrap();

        assert!(result.contains(r#"lines="2-4""#));
        assert!(result.contains("line2\nline3\nline4"));
        assert!(!result.contains("line1"));
        assert!(!result.contains("line5"));
    }

    #[test]
    fn resolve_mentions_single_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let result =
            resolve_mentions_internal(dir.path().to_str().unwrap(), "Look at @test.txt:L2")
                .unwrap();

        assert!(result.contains(r#"lines="2""#));
        assert!(result.contains("line2"));
    }

    #[test]
    fn list_mentionable_files_basic() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("world.txt"), "hello").unwrap();

        // Initialize git repo so .gitignore is respected
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

        let result = list_mentionable_files(
            dir.path().to_str().unwrap().to_string(),
            "mn".to_string(), // fuzzy matches "main.rs"
        )
        .unwrap();

        assert!(result.contains(&"main.rs".to_string()));
        assert!(!result.contains(&"lib.rs".to_string()));
    }

    #[test]
    fn resolve_mentions_missing_file_skips() {
        let dir = tempfile::tempdir().unwrap();
        let result =
            resolve_mentions_internal(dir.path().to_str().unwrap(), "Check @nonexistent.txt");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Check @nonexistent.txt");
    }

    #[test]
    fn resolve_mentions_partial_failure_includes_successful() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("exists.txt"), "content here").unwrap();
        let result = resolve_mentions_internal(
            dir.path().to_str().unwrap(),
            "See @exists.txt and @missing.txt",
        )
        .unwrap();
        assert!(result.contains("content here"));
        assert!(result.contains("<file_context>"));
    }

    #[test]
    fn resolve_mentions_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        // Create a file outside the worktree that the traversal would target
        let parent = dir.path().parent().unwrap();
        let outside_file = parent.join("outside.txt");
        fs::write(&outside_file, "secret").unwrap();

        let result = resolve_mentions_internal(
            dir.path().to_str().unwrap(),
            &format!("Check @../outside.txt"),
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("traversal"), "Error message: {err}");

        // Clean up
        let _ = fs::remove_file(outside_file);
    }

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
    fn parse_mentions_ignores_email_addresses() {
        let result = parse_mentions("Contact user@example.com for details");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_mentions_ignores_mid_word_at() {
        let result = parse_mentions("something@path.rs is not a mention");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_mentions_accepts_after_whitespace() {
        let result = parse_mentions("Check @src/main.rs please");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_path, "src/main.rs");
    }

    #[test]
    fn parse_mentions_accepts_at_start() {
        let result = parse_mentions("@src/lib.rs has the code");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_path, "src/lib.rs");
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
}
