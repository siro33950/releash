use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::git::review::get_review_diff;
use crate::thread_store::ThreadStore;

const DEFAULT_REVIEW_PROMPT: &str = include_str!("../resources/prompts/review.txt");
const PER_FILE_REVIEW_TEMPLATE: &str = include_str!("../resources/prompts/review_file.txt");

#[tauri::command]
pub fn get_review_prompt() -> String {
    DEFAULT_REVIEW_PROMPT.to_string()
}

#[derive(Debug, Serialize)]
pub struct PerFileReviewTask {
    pub file_path: String,
    pub prompt: String,
}

#[tauri::command]
pub fn get_per_file_review_tasks(
    worktree_path: String,
    thread_store: State<'_, Arc<ThreadStore>>,
) -> Result<Vec<PerFileReviewTask>, String> {
    let diff = get_review_diff(&worktree_path, None, None, None)
        .map_err(|e| format!("failed to get review diff: {e}"))?;

    if diff.changed_files.is_empty() {
        return Ok(Vec::new());
    }

    // Build change summary JSON (file list + stats)
    let summary_entries: Vec<serde_json::Value> = diff
        .changed_files
        .iter()
        .map(|f| {
            serde_json::json!({
                "path": f.path,
                "status": f.status,
                "stats": { "additions": f.stats.additions, "deletions": f.stats.deletions }
            })
        })
        .collect();
    let change_summary = serde_json::json!({
        "changed_files": summary_entries,
        "stats": {
            "files_changed": diff.stats.files_changed,
            "insertions": diff.stats.insertions,
            "deletions": diff.stats.deletions,
        }
    })
    .to_string();

    // Build per-file tasks
    let tasks: Vec<PerFileReviewTask> = diff
        .changed_files
        .iter()
        .map(|file| {
            // Get existing threads for this file
            let file_threads =
                thread_store.get_filtered(&worktree_path, Some(&file.path), None, None);
            let existing_comments = if file_threads.is_empty() {
                "None".to_string()
            } else {
                file_threads
                    .iter()
                    .map(|t| {
                        let first_content = t
                            .entries
                            .first()
                            .map(|e| e.content.chars().take(200).collect::<String>())
                            .unwrap_or_default();
                        format!(
                            "- [{}] L{}: {} (severity: {}, resolved: {})",
                            t.id,
                            t.line_number,
                            first_content,
                            t.severity.as_deref().unwrap_or("none"),
                            t.resolved,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };

            let prompt = PER_FILE_REVIEW_TEMPLATE
                .replace("{{WORKTREE}}", &worktree_path)
                .replace("{{FILE_PATH}}", &file.path)
                .replace("{{CHANGE_SUMMARY}}", &change_summary)
                .replace("{{EXISTING_COMMENTS}}", &existing_comments);

            PerFileReviewTask {
                file_path: file.path.clone(),
                prompt,
            }
        })
        .collect();

    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_prompt_is_not_empty() {
        let prompt = get_review_prompt();
        assert!(!prompt.is_empty());
        assert!(prompt.contains("You are a code reviewer"));
    }

    #[test]
    fn per_file_template_has_placeholders() {
        assert!(PER_FILE_REVIEW_TEMPLATE.contains("{{WORKTREE}}"));
        assert!(PER_FILE_REVIEW_TEMPLATE.contains("{{FILE_PATH}}"));
        assert!(PER_FILE_REVIEW_TEMPLATE.contains("{{CHANGE_SUMMARY}}"));
        assert!(PER_FILE_REVIEW_TEMPLATE.contains("{{EXISTING_COMMENTS}}"));
    }
}
