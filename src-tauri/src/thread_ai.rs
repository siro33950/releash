use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::thread_store::ThreadStore;

const THREAD_ASK_TEMPLATE: &str = include_str!("../resources/prompts/thread_ask.txt");
const THREAD_ASK_PR_TEMPLATE: &str = include_str!("../resources/prompts/thread_ask_pr.txt");
const THREAD_SUMMARIZE_TEMPLATE: &str = include_str!("../resources/prompts/thread_summarize.txt");
const THREAD_SUMMARIZE_PR_TEMPLATE: &str =
    include_str!("../resources/prompts/thread_summarize_pr.txt");

const CONTEXT_LINES: usize = 20;
const PROCESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Serialize)]
pub struct ThreadAiPrompt {
    pub prompt: String,
    pub thread_id: String,
    pub file_path: String,
}

#[tauri::command]
pub async fn build_thread_ai_prompt(
    store: State<'_, Arc<ThreadStore>>,
    worktree_path: String,
    thread_id: String,
    pr_number: Option<u64>,
) -> Result<ThreadAiPrompt, String> {
    let template = if thread_id.starts_with("pr-comment-") {
        THREAD_ASK_PR_TEMPLATE
    } else {
        THREAD_ASK_TEMPLATE
    };
    let store = Arc::clone(&store);
    build_prompt_with_template(store, worktree_path, thread_id, template, pr_number).await
}

#[tauri::command]
pub async fn build_thread_summarize_prompt(
    store: State<'_, Arc<ThreadStore>>,
    worktree_path: String,
    thread_id: String,
    pr_number: Option<u64>,
) -> Result<ThreadAiPrompt, String> {
    let template = if thread_id.starts_with("pr-comment-") {
        THREAD_SUMMARIZE_PR_TEMPLATE
    } else {
        THREAD_SUMMARIZE_TEMPLATE
    };
    let store = Arc::clone(&store);
    build_prompt_with_template(store, worktree_path, thread_id, template, pr_number).await
}

async fn build_prompt_with_template(
    store: Arc<ThreadStore>,
    worktree_path: String,
    thread_id: String,
    template: &str,
    pr_number: Option<u64>,
) -> Result<ThreadAiPrompt, String> {
    let threads = store.get_all(&worktree_path);
    let thread = threads
        .iter()
        .find(|t| t.id == thread_id)
        .ok_or_else(|| format!("Thread not found: {thread_id}"))?;

    let file_path = thread.file_path.clone();
    let line_number = thread.line_number;
    let end_line = thread.end_line;
    let thread_id_owned = thread.id.clone();

    let abs_path = std::path::PathBuf::from(&worktree_path).join(&file_path);
    let code_snippet = {
        let abs_path = abs_path.clone();
        tokio::task::spawn_blocking(move || read_code_snippet(&abs_path, line_number, end_line))
            .await
            .map_err(|e| format!("Failed to read code: {e}"))?
    };

    let line_range = match end_line {
        Some(end) => format!("{}-{}", line_number, end),
        None => line_number.to_string(),
    };

    let diff = get_file_diff_async(&worktree_path, &file_path).await;
    let pr_diff = match pr_number {
        Some(n) => get_pr_file_diff_async(&worktree_path, n, &file_path).await,
        None => None,
    };

    let mut sorted_entries = thread.entries.clone();
    sorted_entries.sort_by(|a, b| {
        a.created_at
            .partial_cmp(&b.created_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let thread_entries = sorted_entries
        .iter()
        .map(|e| {
            let role = if e.is_ai {
                e.author_name.as_deref().unwrap_or("AI")
            } else {
                e.author_name.as_deref().unwrap_or("User")
            };
            format!("**{}**: {}", role, e.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut prompt = template.to_string();
    prompt = expand_conditional_section(&prompt, "PR_DIFF", pr_diff.as_deref());
    prompt = expand_conditional_section(&prompt, "DIFF", diff.as_deref());
    prompt = prompt
        .replace("{{WORKTREE}}", &worktree_path)
        .replace("{{THREAD_ID}}", &thread_id)
        .replace("{{FILE_PATH}}", &file_path)
        .replace("{{LINE_RANGE}}", &line_range)
        .replace("{{CODE_SNIPPET}}", &code_snippet)
        .replace("{{THREAD_ENTRIES}}", &thread_entries);

    Ok(ThreadAiPrompt {
        prompt,
        thread_id: thread_id_owned,
        file_path,
    })
}

fn expand_conditional_section(prompt: &str, name: &str, content: Option<&str>) -> String {
    let open_tag = format!("{{{{#{name}}}}}");
    let close_tag = format!("{{{{/{name}}}}}");
    let var_tag = format!("{{{{{name}}}}}");

    if let Some(value) = content {
        prompt
            .replace(&open_tag, "")
            .replace(&close_tag, "")
            .replace(&var_tag, value)
    } else if let Some(start) = prompt.find(&open_tag) {
        if let Some(end) = prompt.find(&close_tag) {
            let end = end + close_tag.len();
            let mut result = prompt.to_string();
            result.replace_range(start..end, "");
            result
        } else {
            prompt.to_string()
        }
    } else {
        prompt.to_string()
    }
}

fn read_code_snippet(path: &std::path::Path, line_number: u32, end_line: Option<u32>) -> String {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return "(file not found)".to_string(),
    };

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    if total_lines == 0 {
        return "(empty file)".to_string();
    }

    let start_line = (line_number as usize)
        .saturating_sub(1)
        .min(total_lines.saturating_sub(1));
    let end_line = end_line
        .map(|e| (e as usize).min(total_lines))
        .unwrap_or((start_line + 1).min(total_lines))
        .max(start_line + 1);

    let ctx_start = start_line.saturating_sub(CONTEXT_LINES);
    let ctx_end = (end_line + CONTEXT_LINES).min(total_lines);

    lines[ctx_start..ctx_end]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>4} | {}", ctx_start + i + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn get_file_diff_async(worktree_path: &str, file_path: &str) -> Option<String> {
    let output = tokio::time::timeout(
        PROCESS_TIMEOUT,
        tokio::process::Command::new("git")
            .args(["diff", "HEAD", "--", file_path])
            .current_dir(worktree_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    if output.status.success() {
        let diff = String::from_utf8_lossy(&output.stdout).to_string();
        if diff.trim().is_empty() {
            None
        } else {
            Some(diff)
        }
    } else {
        None
    }
}

async fn get_pr_file_diff_async(
    worktree_path: &str,
    pr_number: u64,
    file_path: &str,
) -> Option<String> {
    let output = tokio::time::timeout(
        PROCESS_TIMEOUT,
        tokio::process::Command::new("gh")
            .args([
                "api",
                &format!("repos/{{owner}}/{{repo}}/pulls/{pr_number}/files"),
                "--paginate",
                "--jq",
                &format!(r#".[] | select(.filename == "{file_path}") | .patch // empty"#),
            ])
            .current_dir(worktree_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    if output.status.success() {
        let patch = String::from_utf8_lossy(&output.stdout).to_string();
        if patch.trim().is_empty() {
            None
        } else {
            Some(patch)
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_has_required_placeholders() {
        for template in [THREAD_ASK_TEMPLATE, THREAD_SUMMARIZE_TEMPLATE] {
            assert!(template.contains("{{WORKTREE}}"));
            assert!(template.contains("{{THREAD_ID}}"));
            assert!(template.contains("{{FILE_PATH}}"));
            assert!(template.contains("{{LINE_RANGE}}"));
            assert!(template.contains("{{CODE_SNIPPET}}"));
            assert!(template.contains("{{THREAD_ENTRIES}}"));
            assert!(template.contains("{{#DIFF}}"));
            assert!(template.contains("{{/DIFF}}"));
        }
    }

    #[test]
    fn pr_template_has_required_placeholders() {
        for template in [THREAD_ASK_PR_TEMPLATE, THREAD_SUMMARIZE_PR_TEMPLATE] {
            assert!(template.contains("{{WORKTREE}}"));
            assert!(template.contains("{{THREAD_ID}}"));
            assert!(template.contains("{{FILE_PATH}}"));
            assert!(template.contains("{{LINE_RANGE}}"));
            assert!(template.contains("{{CODE_SNIPPET}}"));
            assert!(template.contains("{{THREAD_ENTRIES}}"));
            assert!(template.contains("{{#DIFF}}"));
            assert!(template.contains("{{/DIFF}}"));
            assert!(template.contains("{{#PR_DIFF}}"));
            assert!(template.contains("{{/PR_DIFF}}"));
        }
    }

    #[test]
    fn expand_conditional_section_with_content() {
        let input = "before\n{{#SEC}}content: {{SEC}}\n{{/SEC}}after";
        let result = expand_conditional_section(input, "SEC", Some("hello"));
        assert_eq!(result, "before\ncontent: hello\nafter");
    }

    #[test]
    fn expand_conditional_section_without_content() {
        let input = "before\n{{#SEC}}content: {{SEC}}\n{{/SEC}}after";
        let result = expand_conditional_section(input, "SEC", None);
        assert_eq!(result, "before\nafter");
    }

    #[test]
    fn read_code_snippet_file_not_found() {
        let snippet = read_code_snippet(std::path::Path::new("/nonexistent/file.rs"), 10, None);
        assert_eq!(snippet, "(file not found)");
    }

    #[test]
    fn read_code_snippet_with_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("test.rs");
        let content: String = (1..=50).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&file, &content).unwrap();

        let snippet = read_code_snippet(&file, 25, None);
        assert!(snippet.contains("line 25"));
        assert!(snippet.contains("line 5")); // 25 - 20
        assert!(snippet.contains("line 45")); // 25 + 20
    }

    #[test]
    fn read_code_snippet_with_range() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("test.rs");
        let content: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&file, &content).unwrap();

        let snippet = read_code_snippet(&file, 30, Some(35));
        assert!(snippet.contains("line 30"));
        assert!(snippet.contains("line 35"));
        assert!(snippet.contains("line 10")); // 30 - 20
        assert!(snippet.contains("line 55")); // 35 + 20
    }

    #[test]
    fn read_code_snippet_near_start() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("test.rs");
        let content: String = (1..=50).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&file, &content).unwrap();

        let snippet = read_code_snippet(&file, 3, None);
        assert!(snippet.contains("line 1")); // clamped to start
        assert!(snippet.contains("line 3"));
    }

    #[tokio::test]
    async fn get_file_diff_nonexistent_dir() {
        let result = get_file_diff_async("/nonexistent/path", "file.rs").await;
        assert!(result.is_none());
    }
}
