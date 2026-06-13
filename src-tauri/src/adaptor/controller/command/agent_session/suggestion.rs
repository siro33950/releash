use std::sync::Arc;

use git2::{Repository, Status, StatusOptions};
use serde::Serialize;

use crate::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::session::{
    ChatMessage, ChatSession, MessagePart, MessageRole, SessionStore,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptSuggestion {
    pub text: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GitSuggestionContext {
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
}

impl GitSuggestionContext {
    fn has_changes(&self) -> bool {
        self.staged_count > 0 || self.unstaged_count > 0 || self.untracked_count > 0
    }
}

fn message_text(message: &ChatMessage) -> String {
    if let Some(parts) = &message.parts {
        let text = parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { content, .. }
                | MessagePart::Thinking { content, .. }
                | MessagePart::Error { content, .. }
                | MessagePart::ToolResult { content, .. } => Some(content.as_str()),
                MessagePart::SystemNotification { label, detail, .. } => {
                    Some(detail.as_deref().unwrap_or(label.as_str()))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() {
            return text;
        }
    }

    let mut text = message.content.clone();
    if let Some(thinking) = &message.thinking {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(thinking);
    }
    text
}

fn has_word(text: &str, words: &[&str]) -> bool {
    let lower = text.to_lowercase();
    words.iter().any(|word| lower.contains(word))
}

fn compact_prompt(text: &str, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut value = compact.chars().take(max_chars).collect::<String>();
    if compact.chars().count() > max_chars {
        value.push_str("...");
    }
    value
}

fn recent_human_prompt_from_sessions(
    sessions: &[ChatSession],
    current_session_id: &str,
) -> Option<String> {
    sessions
        .iter()
        .filter(|session| session.id != current_session_id)
        .flat_map(|session| {
            session
                .messages
                .iter()
                .filter(|message| message.role == MessageRole::Human)
                .map(move |message| (session.updated_at.max(message.timestamp), message))
        })
        .filter_map(|(timestamp, message)| {
            let text = compact_prompt(&message_text(message), 160);
            if text.trim().is_empty() {
                None
            } else {
                Some((timestamp, text))
            }
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, text)| text)
}

fn git_suggestion(context: &GitSuggestionContext) -> Option<AgentPromptSuggestion> {
    if !context.has_changes() {
        return None;
    }
    let (text, source) = if context.staged_count > 0 && context.unstaged_count == 0 {
        (
            "Review the staged changes and draft a concise commit message.",
            "git_staged",
        )
    } else if context.staged_count > 0 {
        (
            "Review the staged and unstaged changes, then suggest what should be committed separately.",
            "git_mixed",
        )
    } else if context.untracked_count > 0 && context.unstaged_count == 0 {
        (
            "Review the untracked files and suggest whether they should be added or ignored.",
            "git_untracked",
        )
    } else {
        (
            "Review the current uncommitted changes and suggest the next implementation step.",
            "git_dirty",
        )
    };
    Some(AgentPromptSuggestion {
        text: text.to_string(),
        source: source.to_string(),
    })
}

fn capture_git_suggestion_context(worktree_path: &str) -> Option<GitSuggestionContext> {
    let repo = Repository::discover(worktree_path).ok()?;
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
    let statuses = repo.statuses(Some(&mut options)).ok()?;
    let mut context = GitSuggestionContext::default();
    for entry in statuses.iter() {
        let status = entry.status();
        if status.contains(Status::INDEX_NEW)
            || status.contains(Status::INDEX_MODIFIED)
            || status.contains(Status::INDEX_DELETED)
            || status.contains(Status::INDEX_RENAMED)
            || status.contains(Status::INDEX_TYPECHANGE)
        {
            context.staged_count += 1;
        }
        if status.contains(Status::WT_MODIFIED)
            || status.contains(Status::WT_DELETED)
            || status.contains(Status::WT_RENAMED)
            || status.contains(Status::WT_TYPECHANGE)
        {
            context.unstaged_count += 1;
        }
        if status.contains(Status::WT_NEW) {
            context.untracked_count += 1;
        }
    }
    Some(context)
}

#[cfg(test)]
pub(crate) fn build_agent_prompt_suggestion_inner(
    session: &ChatSession,
) -> Option<AgentPromptSuggestion> {
    build_agent_prompt_suggestion_with_git(session, None, None)
}

pub(crate) fn build_agent_prompt_suggestion_with_git(
    session: &ChatSession,
    git_context: Option<&GitSuggestionContext>,
    recent_prompt: Option<&str>,
) -> Option<AgentPromptSuggestion> {
    if session.messages.is_empty() {
        if let Some(suggestion) = git_context.and_then(git_suggestion) {
            return Some(suggestion);
        }
        if let Some(prompt) = recent_prompt.filter(|prompt| !prompt.trim().is_empty()) {
            return Some(AgentPromptSuggestion {
                text: format!("Continue a recent request: {prompt}"),
                source: "prompt_history".to_string(),
            });
        }
        return Some(AgentPromptSuggestion {
            text: "Review the current repository state and suggest the next step.".to_string(),
            source: "empty_session".to_string(),
        });
    }

    let latest = session.messages.iter().rev().find(|message| {
        matches!(message.role, MessageRole::Agent | MessageRole::Human)
            && !message_text(message).trim().is_empty()
    })?;
    let text = message_text(latest);

    if latest.role == MessageRole::Human {
        return Some(AgentPromptSuggestion {
            text: "Continue from my previous request.".to_string(),
            source: "latest_user_prompt".to_string(),
        });
    }

    if let Some(suggestion) = git_context.and_then(git_suggestion) {
        if !has_word(&text, &["test", "tests", "failing", "failure", "failed"]) {
            return Some(suggestion);
        }
    }

    let suggestion = if has_word(&text, &["test", "tests", "failing", "failure", "failed"]) {
        "Run the relevant tests and fix any failures."
    } else if has_word(&text, &["plan", "step", "todo", "next"]) {
        "Implement the next step from the plan."
    } else if has_word(&text, &["diff", "review", "changes"]) {
        "Review the current diff and call out anything still missing."
    } else {
        "Continue with the next useful implementation step."
    };

    Some(AgentPromptSuggestion {
        text: suggestion.to_string(),
        source: "latest_agent_message".to_string(),
    })
}

#[tauri::command]
pub fn build_agent_prompt_suggestion(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
) -> Result<Option<AgentPromptSuggestion>, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store.get_session(&data_dir, &chat_session_id)?;
    Ok(session.as_ref().and_then(|session| {
        let git_context = capture_git_suggestion_context(&session.worktree_path);
        let recent_prompt = session_store
            .list_worktree_sessions(&data_dir, &session.worktree_path)
            .ok()
            .and_then(|sessions| recent_human_prompt_from_sessions(&sessions, &session.id));
        build_agent_prompt_suggestion_with_git(
            session,
            git_context.as_ref(),
            recent_prompt.as_deref(),
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::session::SessionState;

    fn session_with_messages(messages: Vec<ChatMessage>) -> ChatSession {
        ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages,
            state: SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
        }
    }

    fn message(role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role,
            content: content.to_string(),
            thinking: None,
            activities: None,
            parts: None,
            timestamp: 1.0,
            mentions: None,
        }
    }

    #[test]
    fn suggestion_for_empty_session_invites_repo_review() {
        let suggestion = build_agent_prompt_suggestion_inner(&session_with_messages(Vec::new()))
            .expect("suggestion");

        assert_eq!(
            suggestion.text,
            "Review the current repository state and suggest the next step."
        );
        assert_eq!(suggestion.source, "empty_session");
    }

    #[test]
    fn suggestion_after_test_related_agent_message_runs_tests() {
        let session = session_with_messages(vec![message(
            MessageRole::Agent,
            "The parser fix is ready, but tests may still be failing.",
        )]);

        let suggestion = build_agent_prompt_suggestion_inner(&session).expect("suggestion");

        assert_eq!(
            suggestion.text,
            "Run the relevant tests and fix any failures."
        );
    }

    #[test]
    fn suggestion_for_empty_dirty_repo_uses_git_context() {
        let context = GitSuggestionContext {
            staged_count: 0,
            unstaged_count: 2,
            untracked_count: 1,
        };
        let suggestion = build_agent_prompt_suggestion_with_git(
            &session_with_messages(Vec::new()),
            Some(&context),
            None,
        )
        .expect("suggestion");

        assert_eq!(
            suggestion.text,
            "Review the current uncommitted changes and suggest the next implementation step."
        );
        assert_eq!(suggestion.source, "git_dirty");
    }

    #[test]
    fn suggestion_for_staged_changes_drafts_commit_message() {
        let context = GitSuggestionContext {
            staged_count: 2,
            unstaged_count: 0,
            untracked_count: 0,
        };
        let suggestion = build_agent_prompt_suggestion_with_git(
            &session_with_messages(vec![message(MessageRole::Agent, "Implementation is done.")]),
            Some(&context),
            None,
        )
        .expect("suggestion");

        assert_eq!(
            suggestion.text,
            "Review the staged changes and draft a concise commit message."
        );
        assert_eq!(suggestion.source, "git_staged");
    }

    #[test]
    fn test_failure_suggestion_takes_priority_over_git_context() {
        let context = GitSuggestionContext {
            staged_count: 2,
            unstaged_count: 0,
            untracked_count: 0,
        };
        let suggestion = build_agent_prompt_suggestion_with_git(
            &session_with_messages(vec![message(
                MessageRole::Agent,
                "Implementation is done, but tests are failing.",
            )]),
            Some(&context),
            None,
        )
        .expect("suggestion");

        assert_eq!(
            suggestion.text,
            "Run the relevant tests and fix any failures."
        );
        assert_eq!(suggestion.source, "latest_agent_message");
    }

    #[test]
    fn empty_session_uses_recent_prompt_history_when_git_is_clean() {
        let suggestion = build_agent_prompt_suggestion_with_git(
            &session_with_messages(Vec::new()),
            None,
            Some("Fix the parser edge case and add tests"),
        )
        .expect("suggestion");

        assert_eq!(
            suggestion.text,
            "Continue a recent request: Fix the parser edge case and add tests"
        );
        assert_eq!(suggestion.source, "prompt_history");
    }

    #[test]
    fn recent_human_prompt_prefers_latest_other_session_message() {
        let mut older = session_with_messages(vec![message(MessageRole::Human, "older request")]);
        older.id = "older".to_string();
        older.updated_at = 10.0;
        older.messages[0].timestamp = 10.0;
        let mut newer = session_with_messages(vec![message(
            MessageRole::Human,
            "newer request with\nextra whitespace",
        )]);
        newer.id = "newer".to_string();
        newer.updated_at = 20.0;
        newer.messages[0].timestamp = 20.0;
        let mut current =
            session_with_messages(vec![message(MessageRole::Human, "current request")]);
        current.id = "current".to_string();
        current.updated_at = 30.0;
        current.messages[0].timestamp = 30.0;

        let prompt =
            recent_human_prompt_from_sessions(&[older, newer, current], "current").expect("prompt");

        assert_eq!(prompt, "newer request with extra whitespace");
    }
}
