use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::diff_comment_store::{DiffComment, DiffCommentStore};
use crate::file_mention::MentionReference;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendDiffCommentsResult {
    pub sent_count: usize,
    pub formatted_message: String,
    pub mentions: Vec<MentionReference>,
    pub comment_ids: Vec<String>,
}

pub fn build_mentions_from_comments(comments: &[DiffComment]) -> Vec<MentionReference> {
    comments
        .iter()
        .map(|c| MentionReference {
            file_path: c.file_path.clone(),
            start_line: c.line_number,
            end_line: c.end_line,
        })
        .collect()
}

pub fn format_comment_text(comments: &[DiffComment]) -> String {
    let mut lines: Vec<String> = Vec::new();

    for comment in comments {
        let location = if let (Some(start), Some(end)) = (comment.line_number, comment.end_line) {
            format!("{}:L{}-L{}", comment.file_path, start, end)
        } else if let Some(line) = comment.line_number {
            format!("{}:L{}", comment.file_path, line)
        } else {
            comment.file_path.clone()
        };

        lines.push(format!("@{} {}", location, comment.content));
    }

    lines.join("\n")
}

/// Format comments for sending to agent. Does NOT mark as sent — the frontend
/// should call `mark_diff_comments_sent` after the agent send succeeds.
#[tauri::command]
pub async fn send_diff_comments_to_agent(
    _app: tauri::AppHandle,
    comment_store: tauri::State<'_, Arc<DiffCommentStore>>,
    worktree_name: String,
    comment_ids: Vec<String>,
) -> Result<SendDiffCommentsResult, String> {
    let comments = if comment_ids.is_empty() {
        comment_store.get_unsent(&worktree_name)
    } else {
        comment_store.get_by_ids(&worktree_name, &comment_ids)
    };

    if comments.is_empty() {
        return Err("No comments to send".to_string());
    }

    let mentions = build_mentions_from_comments(&comments);
    let formatted_message = format_comment_text(&comments);
    let sent_ids: Vec<String> = comments.iter().map(|c| c.id.clone()).collect();
    let sent_count = sent_ids.len();

    Ok(SendDiffCommentsResult {
        sent_count,
        formatted_message,
        mentions,
        comment_ids: sent_ids,
    })
}

/// Mark comments as sent after the agent send succeeds.
#[tauri::command]
pub async fn mark_diff_comments_sent(
    app: tauri::AppHandle,
    comment_store: tauri::State<'_, Arc<DiffCommentStore>>,
    worktree_name: String,
    comment_ids: Vec<String>,
) -> Result<(), String> {
    comment_store.mark_sent(&worktree_name, &comment_ids)?;

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    comment_store.save(&data_dir, &worktree_name)?;
    let _ = app.emit("diff-comments-changed", &worktree_name);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_mentions_single_line() {
        let comments = vec![DiffComment {
            id: "1".to_string(),
            file_path: "src/components/Example.tsx".to_string(),
            line_number: Some(42),
            end_line: None,
            content: "ここのロジックは〇〇すべき".to_string(),
            status: "unsent".to_string(),
            created_at: 0.0,
        }];

        let mentions = build_mentions_from_comments(&comments);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].file_path, "src/components/Example.tsx");
        assert_eq!(mentions[0].start_line, Some(42));
        assert_eq!(mentions[0].end_line, None);
    }

    #[test]
    fn build_mentions_range() {
        let comments = vec![DiffComment {
            id: "2".to_string(),
            file_path: "src/components/Example.tsx".to_string(),
            line_number: Some(10),
            end_line: Some(15),
            content: "この範囲のエラーハンドリングが不足".to_string(),
            status: "unsent".to_string(),
            created_at: 0.0,
        }];

        let mentions = build_mentions_from_comments(&comments);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].start_line, Some(10));
        assert_eq!(mentions[0].end_line, Some(15));
    }

    #[test]
    fn format_comment_text_single() {
        let comments = vec![DiffComment {
            id: "1".to_string(),
            file_path: "src/a.ts".to_string(),
            line_number: Some(42),
            end_line: None,
            content: "Comment A".to_string(),
            status: "unsent".to_string(),
            created_at: 0.0,
        }];

        let result = format_comment_text(&comments);
        assert_eq!(result, "@src/a.ts:L42 Comment A");
    }

    #[test]
    fn format_comment_text_multiple() {
        let comments = vec![
            DiffComment {
                id: "1".to_string(),
                file_path: "src/a.ts".to_string(),
                line_number: Some(42),
                end_line: None,
                content: "Comment A".to_string(),
                status: "unsent".to_string(),
                created_at: 0.0,
            },
            DiffComment {
                id: "2".to_string(),
                file_path: "src/b.ts".to_string(),
                line_number: Some(10),
                end_line: Some(15),
                content: "Comment B".to_string(),
                status: "unsent".to_string(),
                created_at: 0.0,
            },
            DiffComment {
                id: "3".to_string(),
                file_path: "src/c.ts".to_string(),
                line_number: None,
                end_line: None,
                content: "Comment C".to_string(),
                status: "unsent".to_string(),
                created_at: 0.0,
            },
        ];

        let result = format_comment_text(&comments);
        assert_eq!(
            result,
            "@src/a.ts:L42 Comment A\n@src/b.ts:L10-L15 Comment B\n@src/c.ts Comment C"
        );
    }

    #[test]
    fn build_mentions_empty() {
        let comments: Vec<DiffComment> = Vec::new();
        let mentions = build_mentions_from_comments(&comments);
        assert!(mentions.is_empty());
    }
}
