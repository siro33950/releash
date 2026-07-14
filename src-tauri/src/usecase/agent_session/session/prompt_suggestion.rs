use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

use super::{ChatMessage, MessagePart, MessageRole, SessionReaderPort, DEFAULT_SESSION_PAGE_LIMIT};

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

pub(crate) trait AgentPromptGitStatusGateway: Send + Sync {
    fn suggestion_context(&self, worktree_path: &str) -> Option<GitSuggestionContext>;
}

pub(crate) struct AgentPromptSuggestionUsecase {
    session_reader: Arc<SessionReaderPort>,
    git_status: Arc<dyn AgentPromptGitStatusGateway>,
}

impl AgentPromptSuggestionUsecase {
    pub(crate) fn new(
        session_reader: Arc<SessionReaderPort>,
        git_status: Arc<dyn AgentPromptGitStatusGateway>,
    ) -> Self {
        Self {
            session_reader,
            git_status,
        }
    }

    pub(crate) fn build(
        &self,
        app_data_dir: &Path,
        chat_session_id: &str,
    ) -> Result<Option<AgentPromptSuggestion>, String> {
        let Some(meta) = self
            .session_reader
            .get_session_meta(app_data_dir, chat_session_id)?
        else {
            return Ok(None);
        };
        let git_context = self.git_status.suggestion_context(&meta.worktree_path);
        let messages = self
            .session_reader
            .get_session_page(
                app_data_dir,
                chat_session_id,
                None,
                DEFAULT_SESSION_PAGE_LIMIT,
            )?
            .map(|page| page.messages)
            .unwrap_or_default();

        Ok(build_agent_prompt_suggestion_with_git(
            &messages,
            git_context.as_ref(),
        ))
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

#[cfg(test)]
pub(crate) fn build_agent_prompt_suggestion_inner(
    messages: &[ChatMessage],
) -> Option<AgentPromptSuggestion> {
    build_agent_prompt_suggestion_with_git(messages, None)
}

pub(crate) fn build_agent_prompt_suggestion_with_git(
    messages: &[ChatMessage],
    git_context: Option<&GitSuggestionContext>,
) -> Option<AgentPromptSuggestion> {
    if messages.is_empty() {
        if let Some(suggestion) = git_context.and_then(git_suggestion) {
            return Some(suggestion);
        }
        return Some(AgentPromptSuggestion {
            text: "Review the current repository state and suggest the next step.".to_string(),
            source: "empty_session".to_string(),
        });
    }

    let latest = messages.iter().rev().find(|message| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::{AgentSessionReader, AgentSessionStorageTypes};
    use crate::usecase::agent_session::event_log::AgentSessionEvent;
    use crate::usecase::agent_session::session::{
        ChatSession, ContextCarryState, MessagePageMetadata, PageCursor, SessionAttachment,
        SessionMeta, SessionPage, SessionState, SessionToolOutput, SESSION_BODY_FORMAT_VERSION,
    };

    struct PromptSuggestionStorage;

    impl AgentSessionStorageTypes for PromptSuggestionStorage {
        type Session = ChatSession;
        type Meta = SessionMeta;
        type PageCursor = PageCursor;
        type Page = SessionPage;
        type Message = ChatMessage;
        type MessagePart = MessagePart;
        type Attachment = SessionAttachment;
        type ToolOutput = SessionToolOutput;
        type Event = AgentSessionEvent;
    }

    impl AgentSessionReader for PromptSuggestionStorage {
        fn list_metas(&self, _app_data_dir: &Path) -> Result<Vec<Self::Meta>, String> {
            Ok(Vec::new())
        }

        fn session_title(
            &self,
            _app_data_dir: &Path,
            _session_id: &str,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn session_titles(
            &self,
            _app_data_dir: &Path,
        ) -> Result<std::collections::HashMap<String, String>, String> {
            Ok(std::collections::HashMap::new())
        }

        fn get_session_meta(
            &self,
            _app_data_dir: &Path,
            session_id: &str,
        ) -> Result<Option<Self::Meta>, String> {
            Ok(Some(SessionMeta {
                id: session_id.to_string(),
                worktree_path: "/repo".to_string(),
                state: SessionState::Idle,
                created_at: 1.0,
                updated_at: 1.0,
                agent_session_id: None,
                context_carry: None::<ContextCarryState>,
                permission_mode: "edit".to_string(),
                plan_mode: false,
                selected_model: None,
                permission_profile_id: None,
                backend_id: "claude".to_string(),
                workflow_node_session: false,
                workflow_node_context: None,
                workflow_instructions: Vec::new(),
                agent_read_paths: None,
                context_epoch: None,
                first_message_preview: "Implementation is done.".to_string(),
                message_count: 200,
                body_format_version: SESSION_BODY_FORMAT_VERSION,
            }))
        }

        fn load_full_session_for_restore(
            &self,
            _app_data_dir: &Path,
            _session_id: &str,
        ) -> Result<Option<Self::Session>, String> {
            panic!("prompt suggestion must not full-load session body")
        }

        fn load_previous_human_message_before_agent(
            &self,
            _app_data_dir: &Path,
            _session_id: &str,
            _agent_message_id: &str,
        ) -> Result<Option<Self::Message>, String> {
            panic!("prompt suggestion must not load prompt message")
        }

        fn get_session_page(
            &self,
            _app_data_dir: &Path,
            _session_id: &str,
            _cursor: Option<Self::PageCursor>,
            _limit: usize,
        ) -> Result<Option<Self::Page>, String> {
            Ok(Some(SessionPage {
                messages: vec![message(MessageRole::Agent, "Implementation is done.")],
                message_metadata: vec![MessagePageMetadata {
                    message_id: "m1".to_string(),
                    token_meta: None,
                    run_meta: None,
                }],
                next_cursor: Some(PageCursor(151)),
                has_more: true,
                total_count: 200,
                latest_token_usage: None,
            }))
        }

        fn get_session_attachment(
            &self,
            _app_data_dir: &Path,
            _session_id: &str,
            _attachment_id: &str,
        ) -> Result<Option<Self::Attachment>, String> {
            Ok(None)
        }

        fn get_session_tool_output(
            &self,
            _app_data_dir: &Path,
            _session_id: &str,
            _tool_output_id: &str,
        ) -> Result<Option<Self::ToolOutput>, String> {
            Ok(None)
        }

        fn load_session_events(
            &self,
            _app_data_dir: &Path,
            _session_id: &str,
        ) -> Result<Vec<Self::Event>, String> {
            Ok(Vec::new())
        }
    }

    struct NoopGitStatusGateway;

    impl AgentPromptGitStatusGateway for NoopGitStatusGateway {
        fn suggestion_context(&self, _worktree_path: &str) -> Option<GitSuggestionContext> {
            None
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
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        }
    }

    #[test]
    fn suggestion_uses_latest_page_without_full_session_body() {
        let reader: Arc<SessionReaderPort> = Arc::new(PromptSuggestionStorage);
        let usecase = AgentPromptSuggestionUsecase::new(reader, Arc::new(NoopGitStatusGateway));
        let dir = tempfile::tempdir().unwrap();

        let suggestion = usecase
            .build(dir.path(), "session-1")
            .expect("suggestion lookup")
            .expect("suggestion");

        assert_eq!(
            suggestion.text,
            "Continue with the next useful implementation step."
        );
    }

    #[test]
    fn suggestion_for_empty_session_invites_repo_review() {
        let suggestion = build_agent_prompt_suggestion_inner(&[]).expect("suggestion");

        assert_eq!(
            suggestion.text,
            "Review the current repository state and suggest the next step."
        );
        assert_eq!(suggestion.source, "empty_session");
    }

    #[test]
    fn suggestion_after_test_related_agent_message_runs_tests() {
        let messages = vec![message(
            MessageRole::Agent,
            "The parser fix is ready, but tests may still be failing.",
        )];

        let suggestion = build_agent_prompt_suggestion_inner(&messages).expect("suggestion");

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
        let suggestion =
            build_agent_prompt_suggestion_with_git(&[], Some(&context)).expect("suggestion");

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
        let messages = vec![message(MessageRole::Agent, "Implementation is done.")];
        let suggestion =
            build_agent_prompt_suggestion_with_git(&messages, Some(&context)).expect("suggestion");

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
        let messages = vec![message(
            MessageRole::Agent,
            "Implementation is done, but tests are failing.",
        )];
        let suggestion =
            build_agent_prompt_suggestion_with_git(&messages, Some(&context)).expect("suggestion");

        assert_eq!(
            suggestion.text,
            "Run the relevant tests and fix any failures."
        );
        assert_eq!(suggestion.source, "latest_agent_message");
    }
}
