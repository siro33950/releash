use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use super::{
    ChatMessage, ChatSession, MessagePart, MessageRole, SessionState, SessionStore, SessionSummary,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchResult {
    pub session: SessionSummary,
    pub matched_message_id: String,
    pub matched_role: MessageRole,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskListItem {
    pub tool_use_id: String,
    pub label: String,
    pub status: String,
    pub background: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskListReport {
    pub title: String,
    pub detail: String,
    pub active_count: usize,
    pub completed_count: usize,
    pub total_count: usize,
    pub items: Vec<AgentTaskListItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentThreadSearchMatch {
    pub message_id: String,
    pub match_index: usize,
}

#[derive(Debug, Clone)]
struct TaskListBuilder {
    tool_use_id: String,
    label: String,
    status: String,
    background: bool,
    has_result: bool,
}

pub(crate) fn build_agent_task_list_report_from_parts(
    parts: &[MessagePart],
) -> AgentTaskListReport {
    let mut order: Vec<String> = Vec::new();
    let mut tasks: HashMap<String, TaskListBuilder> = HashMap::new();

    for part in parts {
        match part {
            MessagePart::ToolUse {
                tool, input, id, ..
            } if is_background_task_tool(tool, input) => {
                let background = input
                    .get("run_in_background")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if !tasks.contains_key(id) {
                    order.push(id.clone());
                    tasks.insert(
                        id.clone(),
                        TaskListBuilder {
                            tool_use_id: id.clone(),
                            label: build_task_label(tool, input),
                            status: "running".to_string(),
                            background,
                            has_result: false,
                        },
                    );
                }
            }
            MessagePart::TaskStatus {
                task_tool_use_id,
                status,
                description,
                summary,
            } => {
                let entry = tasks.entry(task_tool_use_id.clone()).or_insert_with(|| {
                    order.push(task_tool_use_id.clone());
                    TaskListBuilder {
                        tool_use_id: task_tool_use_id.clone(),
                        label: description
                            .as_deref()
                            .or(summary.as_deref())
                            .map(truncate_task_label)
                            .unwrap_or_else(|| "Task".to_string()),
                        status: "running".to_string(),
                        background: true,
                        has_result: false,
                    }
                });
                if let Some(description) = description.as_deref().filter(|value| !value.is_empty())
                {
                    entry.label = truncate_task_label(description);
                }
                entry.status = if is_terminal_task_status(status) {
                    status.clone()
                } else {
                    "running".to_string()
                };
            }
            MessagePart::ToolResult {
                tool_use_id: Some(tool_use_id),
                is_error,
                ..
            } => {
                if let Some(entry) = tasks.get_mut(tool_use_id) {
                    entry.has_result = true;
                    if !entry.background && !is_terminal_task_status(&entry.status) {
                        entry.status = if *is_error { "failed" } else { "completed" }.to_string();
                    }
                }
            }
            _ => {}
        }
    }

    let items: Vec<AgentTaskListItem> = order
        .into_iter()
        .filter_map(|id| tasks.remove(&id))
        .map(|task| {
            let status = if !task.background && task.has_result && task.status == "running" {
                "completed".to_string()
            } else {
                task.status
            };
            AgentTaskListItem {
                tool_use_id: task.tool_use_id,
                label: task.label,
                status,
                background: task.background,
            }
        })
        .collect();

    let total_count = items.len();
    let completed_count = items
        .iter()
        .filter(|item| is_terminal_task_status(&item.status))
        .count();
    let active_count = total_count.saturating_sub(completed_count);

    if items.is_empty() {
        return AgentTaskListReport {
            title: "Tasks: none".to_string(),
            detail: "No agent tasks have been observed in this session yet.".to_string(),
            active_count: 0,
            completed_count: 0,
            total_count: 0,
            items,
        };
    }

    const MAX_VISIBLE: usize = 5;
    let hidden_count = items.len().saturating_sub(MAX_VISIBLE);
    let mut detail = items
        .iter()
        .take(MAX_VISIBLE)
        .map(|item| {
            let background = if item.background { " background" } else { "" };
            format!("{}{} - {}", item.status, background, item.label)
        })
        .collect::<Vec<_>>()
        .join("\n");
    if hidden_count > 0 {
        detail.push_str(&format!("\n... {hidden_count} more"));
    }

    AgentTaskListReport {
        title: format!("Tasks: {active_count} active / {completed_count} finished"),
        detail,
        active_count,
        completed_count,
        total_count,
        items,
    }
}

pub(crate) fn search_agent_sessions(
    session_store: &SessionStore,
    data_dir: &Path,
    worktree_path: &str,
    query: &str,
    include_workflow: bool,
    limit: usize,
) -> Result<Vec<SessionSearchResult>, String> {
    let sessions = session_store.list_worktree_sessions_full(data_dir, worktree_path)?;
    let mut results = search_sessions(sessions, query, include_workflow, limit);
    let titles = session_store.session_titles(data_dir)?;
    for result in &mut results {
        if let Some(title) = titles.get(&result.session.id) {
            result.session.first_message = title.clone();
        }
    }
    Ok(results)
}

pub(crate) fn search_agent_session_messages(
    session_store: &SessionStore,
    data_dir: &Path,
    session_id: &str,
    query: &str,
) -> Result<Vec<AgentThreadSearchMatch>, String> {
    let session = session_store
        .load_full_session_for_restore(data_dir, session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    Ok(search_thread_messages(&session.messages, query))
}

fn truncate_task_label(label: &str) -> String {
    const MAX_CHARS: usize = 96;
    let mut truncated: String = label.chars().take(MAX_CHARS).collect();
    if label.chars().count() > MAX_CHARS {
        truncated.push_str("...");
    }
    truncated
}

fn task_input_string(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(truncate_task_label)
}

fn is_background_task_tool(tool: &str, input: &serde_json::Value) -> bool {
    tool == "Task"
        || tool == "Agent"
        || input
            .get("run_in_background")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
}

fn build_task_label(tool: &str, input: &serde_json::Value) -> String {
    let label = task_input_string(input, "description")
        .or_else(|| task_input_string(input, "command"))
        .or_else(|| task_input_string(input, "prompt"))
        .unwrap_or_else(|| tool.to_string());
    match task_input_string(input, "subagent_type") {
        Some(subagent_type) => format!("{label} ({subagent_type})"),
        None => label,
    }
}

fn is_terminal_task_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "stopped")
}

fn message_search_text(message: &ChatMessage) -> String {
    if let Some(parts) = &message.parts {
        let mut text = String::new();
        for part in parts {
            match part {
                MessagePart::Text { content, .. }
                | MessagePart::Error { content, .. }
                | MessagePart::Thinking { content, .. }
                | MessagePart::ToolResult { content, .. } => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(content);
                }
                MessagePart::ToolUse { tool, input, .. } => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(tool);
                    text.push(' ');
                    text.push_str(&input.to_string());
                }
                MessagePart::Permission {
                    request, status, ..
                } => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(status.as_str());
                    text.push(' ');
                    if let Ok(serialized) = serde_json::to_string(request) {
                        text.push_str(&serialized);
                    } else {
                        text.push_str(&request.tool_name);
                    }
                }
                MessagePart::TaskStatus {
                    status,
                    description,
                    summary,
                    ..
                } => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(status);
                    if let Some(description) = description {
                        text.push(' ');
                        text.push_str(description);
                    }
                    if let Some(summary) = summary {
                        text.push(' ');
                        text.push_str(summary);
                    }
                }
                MessagePart::TodoListSnapshot { items } => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    for item in items {
                        text.push_str(if item.completed { "[x] " } else { "[ ] " });
                        text.push_str(&item.text);
                        text.push('\n');
                    }
                }
                MessagePart::SystemNotification { label, detail, .. } => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(label);
                    if let Some(detail) = detail {
                        text.push(' ');
                        text.push_str(detail);
                    }
                }
                MessagePart::Image { .. } | MessagePart::ImageRef { .. } => {}
            }
        }
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
    if let Some(activities) = &message.activities {
        for activity in activities {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&serde_json::to_string(activity).unwrap_or_default());
        }
    }
    text
}

fn build_search_snippet(text: &str, needle: &str) -> String {
    let haystack = text.to_lowercase();
    let needle = needle.to_lowercase();
    let Some(byte_index) = haystack.find(&needle) else {
        return text.chars().take(160).collect();
    };
    let char_index = text[..byte_index].chars().count();
    let start = char_index.saturating_sub(60);
    let end = (char_index + needle.chars().count() + 100).min(text.chars().count());
    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.extend(text.chars().skip(start).take(end - start));
    if end < text.chars().count() {
        snippet.push('…');
    }
    snippet
}

fn count_thread_search_matches(text: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    let haystack = text.to_lowercase();
    let needle = query.to_lowercase();
    let mut count = 0;
    let mut cursor = 0;
    while cursor < haystack.len() {
        let Some(index) = haystack[cursor..].find(&needle) else {
            break;
        };
        count += 1;
        cursor += index + needle.len().max(1);
    }
    count
}

fn search_thread_messages(messages: &[ChatMessage], query: &str) -> Vec<AgentThreadSearchMatch> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    messages
        .iter()
        .flat_map(|message| {
            let count = count_thread_search_matches(&message_search_text(message), query);
            (0..count).map(|match_index| AgentThreadSearchMatch {
                message_id: message.id.clone(),
                match_index,
            })
        })
        .collect()
}

fn search_sessions(
    sessions: Vec<ChatSession>,
    query: &str,
    include_workflow: bool,
    limit: usize,
) -> Vec<SessionSearchResult> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut results = Vec::new();
    let mut sorted = sessions;
    sorted.sort_by(|a, b| {
        b.updated_at
            .partial_cmp(&a.updated_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for session in sorted {
        if !include_workflow && session.is_workflow_node_session() {
            continue;
        }
        if session.state == SessionState::Archived {
            continue;
        }
        let Some((message, text)) = session.messages.iter().find_map(|message| {
            let text = message_search_text(message);
            text.to_lowercase()
                .contains(&needle)
                .then_some((message, text))
        }) else {
            continue;
        };
        results.push(SessionSearchResult {
            session: session.to_summary(),
            matched_message_id: message.id.clone(),
            matched_role: message.role.clone(),
            snippet: build_search_snippet(&text, &needle),
        });
        if results.len() >= limit.max(1) {
            break;
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::session::{
        ContextCarryState, PermissionPartStatus, DEFAULT_SESSION_PAGE_LIMIT,
    };

    fn human_prompt(id: &str, content: &str, timestamp: f64) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::Human,
            content: content.to_string(),
            timestamp,
            streaming_final_seq: 0,
            thinking: None,
            activities: None,
            parts: Some(vec![MessagePart::Text {
                content: content.to_string(),
                parent_tool_use_id: None,
            }]),
            mentions: None,
        }
    }

    fn agent_response(id: &str, content: &str, timestamp: f64) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: MessageRole::Agent,
            content: content.to_string(),
            timestamp,
            streaming_final_seq: 0,
            thinking: None,
            activities: None,
            parts: Some(vec![MessagePart::Text {
                content: content.to_string(),
                parent_tool_use_id: None,
            }]),
            mentions: None,
        }
    }

    fn session_with_messages(messages: Vec<ChatMessage>) -> ChatSession {
        ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages,
            state: SessionState::Idle,
            created_at: 1.0,
            updated_at: 2.0,
            agent_session_id: None,
            context_carry: Some(ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some("claude".to_string()),
            workflow_node_session: false,
            workflow_node_context: None,
            context_epoch: None,
        }
    }

    #[test]
    fn task_list_report_summarizes_running_and_finished_tasks() {
        let parts = vec![
            MessagePart::ToolUse {
                tool: "Task".to_string(),
                input: serde_json::json!({
                    "description": "Explore parser",
                    "subagent_type": "Explore"
                }),
                id: "task-1".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::TaskStatus {
                task_tool_use_id: "task-1".to_string(),
                status: "completed".to_string(),
                description: None,
                summary: Some("Parser mapped".to_string()),
            },
            MessagePart::ToolUse {
                tool: "Bash".to_string(),
                input: serde_json::json!({
                    "command": "pnpm test",
                    "run_in_background": true
                }),
                id: "task-2".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::TaskStatus {
                task_tool_use_id: "task-2".to_string(),
                status: "progress".to_string(),
                description: Some("Running tests".to_string()),
                summary: None,
            },
        ];

        let report = build_agent_task_list_report_from_parts(&parts);

        assert_eq!(report.title, "Tasks: 1 active / 1 finished");
        assert_eq!(report.active_count, 1);
        assert_eq!(report.completed_count, 1);
        assert_eq!(report.total_count, 2);
        assert!(report
            .detail
            .contains("completed - Explore parser (Explore)"));
        assert!(report.detail.contains("running background - Running tests"));
    }

    #[test]
    fn task_list_report_handles_no_tasks() {
        let report = build_agent_task_list_report_from_parts(&[MessagePart::Text {
            content: "hello".to_string(),
            parent_tool_use_id: None,
        }]);

        assert_eq!(report.title, "Tasks: none");
        assert!(report.items.is_empty());
    }

    #[test]
    fn search_sessions_matches_message_parts_and_excludes_workflow_by_default() {
        let regular = session_with_messages(vec![ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: String::new(),
            thinking: None,
            activities: None,
            parts: Some(vec![MessagePart::Text {
                content: "The parser bug is fixed".to_string(),
                parent_tool_use_id: None,
            }]),
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        }]);
        let mut workflow = regular.clone();
        workflow.id = uuid::Uuid::new_v4().to_string();
        workflow.updated_at = 3.0;
        workflow.workflow_node_session = true;

        let results = search_sessions(vec![workflow, regular], "parser", false, 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matched_message_id, "m1");
        assert_eq!(results[0].snippet, "The parser bug is fixed");
        assert!(!results[0].session.workflow_node_session);
    }

    #[test]
    fn search_thread_messages_returns_each_match_in_message_order() {
        let messages = vec![
            human_prompt("h1", "Please inspect the agent", 10.0),
            agent_response("a1", "The agent found an agent bug", 20.0),
        ];

        let results = search_thread_messages(&messages, "agent");

        assert_eq!(
            results,
            vec![
                AgentThreadSearchMatch {
                    message_id: "h1".to_string(),
                    match_index: 0,
                },
                AgentThreadSearchMatch {
                    message_id: "a1".to_string(),
                    match_index: 0,
                },
                AgentThreadSearchMatch {
                    message_id: "a1".to_string(),
                    match_index: 1,
                },
            ]
        );
    }

    #[test]
    fn search_thread_messages_uses_full_message_search_text() {
        let messages = vec![ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: String::new(),
            thinking: None,
            activities: None,
            parts: Some(vec![MessagePart::ToolResult {
                content: "Parser output included TargetNeedle".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: None,
                summary: None,
            }]),
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        }];

        let results = search_thread_messages(&messages, "targetneedle");

        assert_eq!(
            results,
            vec![AgentThreadSearchMatch {
                message_id: "m1".to_string(),
                match_index: 0,
            }]
        );
    }

    #[test]
    fn search_session_messages_uses_full_session_beyond_latest_page() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let session_id = uuid::Uuid::new_v4().to_string();
        let messages = (0..(DEFAULT_SESSION_PAGE_LIMIT + 2))
            .map(|index| {
                let content = if index == 0 {
                    "needle only in unloaded history".to_string()
                } else {
                    format!("ordinary message {index}")
                };
                human_prompt(&format!("m{index}"), &content, index as f64)
            })
            .collect::<Vec<_>>();
        let mut session = session_with_messages(messages);
        session.id = session_id.clone();
        store
            .save_full_session_for_migration_or_restore(data_dir.path(), &session)
            .unwrap();

        let latest_page = store
            .get_session_page(
                data_dir.path(),
                &session_id,
                None,
                DEFAULT_SESSION_PAGE_LIMIT,
            )
            .unwrap()
            .unwrap();
        assert!(!latest_page
            .messages
            .iter()
            .any(|message| message.id == "m0"));

        let results = search_agent_session_messages(&store, data_dir.path(), &session_id, "needle")
            .expect("search should load full session");

        assert_eq!(
            results,
            vec![AgentThreadSearchMatch {
                message_id: "m0".to_string(),
                match_index: 0,
            }]
        );
    }

    #[test]
    fn search_sessions_excludes_archived_sessions() {
        let mut archived = session_with_messages(vec![ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Human,
            content: "archived parser note".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        }]);
        archived.state = SessionState::Archived;

        let results = search_sessions(vec![archived], "parser", false, 10);

        assert!(results.is_empty());
    }

    #[test]
    fn message_search_text_includes_typed_permission_and_todo_parts() {
        let message = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: String::new(),
            thinking: None,
            activities: None,
            parts: Some(vec![
                MessagePart::Permission {
                    request: super::super::PermissionRequestMsg {
                        id: "perm-1".to_string(),
                        tool_use_id: Some("tool-1".to_string()),
                        tool_name: "Bash".to_string(),
                        kind: super::super::PermissionRequestKindMsg::ToolApproval,
                        input: Some(serde_json::json!({"command":"cargo test"})),
                        plan: None,
                        allowed_prompts: Vec::new(),
                        questions: Vec::new(),
                        title: None,
                        display_name: None,
                        description: None,
                        decision_reason: None,
                    },
                    status: PermissionPartStatus::Pending,
                    answers: None,
                    parent_tool_use_id: None,
                },
                MessagePart::TodoListSnapshot {
                    items: vec![super::super::TodoListItem {
                        text: "Fix parser".to_string(),
                        completed: false,
                    }],
                },
            ]),
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        };

        let text = message_search_text(&message);

        assert!(text.contains("pending"));
        assert!(text.contains("cargo test"));
        assert!(text.contains("[ ] Fix parser"));
    }
}
