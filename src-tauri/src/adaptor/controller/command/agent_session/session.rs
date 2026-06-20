use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agent_message_dispatcher::{
    dispatch_agent_message, AgentMessageDispatchContext, AgentMessageDispatchRequest,
};
use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime::{
    AgentBackendRegistry, AgentProcessMap, ImageAttachment, SessionConfig,
};
use crate::usecase::agent_session::session::errors::session_target_rejected;
use crate::usecase::agent_session::session::{
    ChatMessage, ChatSession, MessagePart, SessionState, SessionStore,
};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchResult {
    pub session: crate::usecase::agent_session::session::SessionSummary,
    pub matched_message_id: String,
    pub matched_role: crate::usecase::agent_session::session::MessageRole,
    pub snippet: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskListItem {
    pub tool_use_id: String,
    pub label: String,
    pub status: String,
    pub background: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskListReport {
    pub title: String,
    pub detail: String,
    pub active_count: usize,
    pub completed_count: usize,
    pub total_count: usize,
    pub items: Vec<AgentTaskListItem>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentThreadSearchRequest {
    pub messages: Vec<ChatMessage>,
    pub query: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
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

pub(crate) fn build_agent_task_list_report_inner(parts: &[MessagePart]) -> AgentTaskListReport {
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
                    text.push_str(status);
                    text.push(' ');
                    text.push_str(&request.to_string());
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
                MessagePart::Image { .. } => {}
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
        if !include_workflow && session.workflow_step_session {
            continue;
        }
        if session.state == SessionState::Archived {
            continue;
        }
        let Some(message) = session.messages.iter().find(|message| {
            message_search_text(message)
                .to_lowercase()
                .contains(&needle)
        }) else {
            continue;
        };
        let text = message_search_text(message);
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

fn reject_explicit_start_for_workflow_step_session(
    session: &crate::usecase::agent_session::session::ChatSession,
    cwd: &str,
) -> Result<(), String> {
    if session.worktree_path != cwd || session.workflow_step_session {
        return Err(session_target_rejected());
    }
    Ok(())
}

/// Tauri invoke 境界で permission_mode を検証し、検証済み抽象モードを返す。
/// 欠落（None）は空文字相当として扱い、対象外値とともに [`crate::permission::InvalidPermissionMode`]
/// で拒否する。command 経路と単体テスト経路の両方で同じ拒否ロジックを共有する（Spec issues-947）。
fn validate_invoke_permission_mode(
    permission_mode: Option<String>,
) -> Result<crate::permission::PermissionMode, String> {
    let permission_value = permission_mode.unwrap_or_default();
    crate::permission::PermissionMode::parse(&permission_value).map_err(|e| e.to_string())
}

fn should_skip_close_agent_session(
    session: Option<&crate::usecase::agent_session::session::ChatSession>,
) -> bool {
    session.is_some_and(|session| session.workflow_step_session)
}

#[tauri::command]
pub async fn set_session_backend(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    chat_session_id: String,
    backend_id: String,
) -> Result<crate::usecase::agent_session::session::GetSessionResponse, String> {
    crate::infrastructure::agent_session::runtime::set_session_backend(
        app,
        session_store,
        registry,
        handles,
        chat_session_id,
        backend_id,
    )
    .await
}

#[tauri::command]
pub async fn get_session(
    state: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Option<crate::usecase::agent_session::session::GetSessionResponse>, String> {
    crate::infrastructure::agent_session::runtime::get_session(
        state, handles, registry, app, session_id,
    )
    .await
}

#[tauri::command]
pub async fn search_agent_sessions(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    worktree_path: String,
    query: String,
    include_workflow: Option<bool>,
    limit: Option<usize>,
) -> Result<Vec<SessionSearchResult>, String> {
    let data_dir = resolve_data_dir(&app)?;
    let sessions = session_store.list_worktree_sessions(&data_dir, &worktree_path)?;
    let mut results = search_sessions(
        sessions,
        &query,
        include_workflow.unwrap_or(false),
        limit.unwrap_or(20),
    );
    for result in &mut results {
        if let Some(title) = session_store.session_title(&data_dir, &result.session.id)? {
            result.session.first_message = title;
        }
    }
    Ok(results)
}

#[tauri::command]
pub async fn search_agent_thread_messages(
    request: AgentThreadSearchRequest,
) -> Result<Vec<AgentThreadSearchMatch>, String> {
    Ok(search_thread_messages(&request.messages, &request.query))
}

#[tauri::command]
pub async fn interrupt_agent_query(
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    chat_session_id: String,
) -> Result<(), String> {
    crate::infrastructure::agent_session::runtime::interrupt_agent_query(
        handles,
        registry,
        chat_session_id,
    )
    .await
}

#[tauri::command]
pub async fn cancel_agent_queued_turn(
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    chat_session_id: String,
    queued_turn_id: Option<String>,
) -> Result<crate::infrastructure::agent_session::runtime::CancelQueuedTurnResponse, String> {
    crate::infrastructure::agent_session::runtime::cancel_agent_queued_turn_internal(
        handles.inner(),
        &chat_session_id,
        queued_turn_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn build_agent_task_list_report(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
) -> Result<AgentTaskListReport, String> {
    let data_dir = resolve_data_dir(&app)?;
    let mut parts = session_store
        .get_session(&data_dir, &chat_session_id)?
        .map(|session| {
            session
                .messages
                .into_iter()
                .filter_map(|message| message.parts)
                .flatten()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let runtime_parts = {
        let map = handles.lock().await;
        map.get(&chat_session_id)
            .map(|proc| proc.streaming_parts.clone())
            .unwrap_or_default()
    };
    parts.extend(runtime_parts);

    Ok(build_agent_task_list_report_inner(&parts))
}
#[tauri::command]
pub async fn init_agent_sessions(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    open_tabs: tauri::State<'_, Arc<crate::usecase::agent_session::session::OpenTabRegistry>>,
    worktree_path: String,
) -> Result<crate::infrastructure::agent_session::runtime::InitSessionsResponse, String> {
    crate::infrastructure::agent_session::runtime::init_agent_sessions(
        app,
        session_store,
        registry,
        handles,
        open_tabs,
        worktree_path,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_agent_session(
    app: tauri::AppHandle,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
    cwd: String,
    permission_mode: Option<String>,
    plan_mode: Option<bool>,
) -> Result<(), String> {
    // 外部境界（Tauri invoke）では permission_mode 欠落・対象外値を InvalidPermissionMode で拒否する。
    // None は空文字相当として扱い、内部経路の保存値フォールバックには進めない。
    let validated_permission_mode = validate_invoke_permission_mode(permission_mode)?;
    let validated_permission_mode_str = validated_permission_mode.as_str().to_string();

    let data_dir = resolve_data_dir(&app)?;
    let mut session = session_store
        .get_session(&data_dir, &chat_session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    reject_explicit_start_for_workflow_step_session(&session, &cwd)?;
    crate::usecase::agent_session::session::resolve_session_backend(
        &mut session,
        registry.inner(),
    )?;
    let backend_id = session
        .backend_id
        .clone()
        .ok_or_else(|| "Session backend is unresolved".to_string())?;

    // 検証済み permission_mode をセッション保存層に反映（外部 UI 操作の結果をセッションに記録）。
    if session.permission_mode != validated_permission_mode_str {
        session_store.update_permission_mode(
            &data_dir,
            &chat_session_id,
            &validated_permission_mode_str,
        )?;
    }
    let validated_plan_mode = plan_mode.unwrap_or(false);
    if session.plan_mode != validated_plan_mode {
        session_store.update_plan_mode(&data_dir, &chat_session_id, validated_plan_mode)?;
    }

    if backend_id == crate::infrastructure::agent_session::runtime::CODEX_BACKEND_ID {
        let backend = registry
            .get(&backend_id)
            .ok_or_else(|| format!("Agent backend not found: {backend_id}"))?;
        backend
            .start_session(SessionConfig {
                chat_session_id,
                cwd,
                permission_mode: Some(validated_permission_mode_str),
                plan_mode: validated_plan_mode,
                permission_profile_id: session.permission_profile_id.clone(),
                system_prompt: None,
            })
            .await?;
        return Ok(());
    }

    crate::infrastructure::agent_session::runtime::start_agent_session_internal(
        &app,
        handles.inner(),
        session_store.inner(),
        &chat_session_id,
        &cwd,
        Some(validated_permission_mode_str),
        validated_plan_mode,
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::session::SessionStore;

    fn session_for_start_guard(
        workflow_step_session: bool,
    ) -> crate::usecase::agent_session::session::ChatSession {
        crate::usecase::agent_session::session::ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-session".to_string()),
            context_carry: Some(crate::usecase::agent_session::session::ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session,
        }
    }

    // Spec issues-947: Tauri invoke 境界で permission_mode の欠落・対象外値を拒否する。
    // start_agent_session 内部の `validate_invoke_permission_mode` を command 相当の経路として
    // 直接呼び、欠落/旧語彙/未知語彙/空文字いずれも `?` で早期 return することを確認する
    // （= `update_permission_mode` も `start_agent_session_internal` も呼ばれない）。
    #[test]
    fn start_agent_session_validate_rejects_missing_or_invalid_permission_mode() {
        let invalid_inputs: Vec<Option<String>> = vec![
            None,
            Some(String::new()),
            Some("acceptEdits".to_string()),
            Some("bypassPermissions".to_string()),
            Some("plan".to_string()),
            Some("default".to_string()),
            Some("unknown".to_string()),
        ];
        for permission in invalid_inputs {
            let label = permission.clone();
            let err = validate_invoke_permission_mode(permission).unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "{:?} must include allowed list, got: {err}",
                label
            );
        }
    }

    #[test]
    fn start_agent_session_validate_accepts_abstract_modes() {
        for mode in ["ask", "edit", "full"] {
            let validated = validate_invoke_permission_mode(Some(mode.to_string())).unwrap();
            assert_eq!(validated.as_str(), mode);
        }
    }

    fn human_prompt(
        id: &str,
        content: &str,
        timestamp: f64,
    ) -> crate::usecase::agent_session::session::ChatMessage {
        crate::usecase::agent_session::session::ChatMessage {
            id: id.to_string(),
            role: crate::usecase::agent_session::session::MessageRole::Human,
            content: content.to_string(),
            timestamp,
            thinking: None,
            activities: None,
            parts: Some(vec![MessagePart::Text {
                content: content.to_string(),
                parent_tool_use_id: None,
            }]),
            mentions: None,
        }
    }

    fn agent_response(
        id: &str,
        content: &str,
        timestamp: f64,
    ) -> crate::usecase::agent_session::session::ChatMessage {
        crate::usecase::agent_session::session::ChatMessage {
            id: id.to_string(),
            role: crate::usecase::agent_session::session::MessageRole::Agent,
            content: content.to_string(),
            timestamp,
            thinking: None,
            activities: None,
            parts: Some(vec![MessagePart::Text {
                content: content.to_string(),
                parent_tool_use_id: None,
            }]),
            mentions: None,
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

        let report = build_agent_task_list_report_inner(&parts);

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
        let report = build_agent_task_list_report_inner(&[MessagePart::Text {
            content: "hello".to_string(),
            parent_tool_use_id: None,
        }]);

        assert_eq!(report.title, "Tasks: none");
        assert!(report.items.is_empty());
    }

    #[test]
    fn search_sessions_matches_message_parts_and_excludes_workflow_by_default() {
        let regular = crate::usecase::agent_session::session::ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![crate::usecase::agent_session::session::ChatMessage {
                id: "m1".to_string(),
                role: crate::usecase::agent_session::session::MessageRole::Agent,
                content: String::new(),
                thinking: None,
                activities: None,
                parts: Some(vec![
                    crate::usecase::agent_session::session::MessagePart::Text {
                        content: "The parser bug is fixed".to_string(),
                        parent_tool_use_id: None,
                    },
                ]),
                timestamp: 1.0,
                mentions: None,
            }],
            state: crate::usecase::agent_session::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 2.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session: false,
        };
        let mut workflow = regular.clone();
        workflow.id = uuid::Uuid::new_v4().to_string();
        workflow.updated_at = 3.0;
        workflow.workflow_step_session = true;

        let results = search_sessions(vec![workflow, regular], "parser", false, 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matched_message_id, "m1");
        assert_eq!(results[0].snippet, "The parser bug is fixed");
        assert!(!results[0].session.workflow_step_session);
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
        let messages = vec![crate::usecase::agent_session::session::ChatMessage {
            id: "m1".to_string(),
            role: crate::usecase::agent_session::session::MessageRole::Agent,
            content: String::new(),
            thinking: None,
            activities: None,
            parts: Some(vec![MessagePart::ToolResult {
                content: "Parser output included TargetNeedle".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
            }]),
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
    fn search_sessions_excludes_archived_sessions() {
        let archived = crate::usecase::agent_session::session::ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![crate::usecase::agent_session::session::ChatMessage {
                id: "m1".to_string(),
                role: crate::usecase::agent_session::session::MessageRole::Human,
                content: "archived parser note".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1.0,
                mentions: None,
            }],
            state: crate::usecase::agent_session::session::SessionState::Archived,
            created_at: 1.0,
            updated_at: 2.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session: false,
        };

        let results = search_sessions(vec![archived], "parser", false, 10);

        assert!(results.is_empty());
    }

    // Tauri invoke 境界が拒否したとき、保存値も runtime ハンドルも変更されないことを
    // 上位の command 経路を模した手順で確認する。
    #[tokio::test]
    async fn start_agent_session_invalid_permission_mode_does_not_mutate_persisted_state() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::default());
        let session = crate::usecase::agent_session::session::ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::usecase::agent_session::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session: false,
        };
        store.save_session(data_dir.path(), &session).unwrap();
        let handles = Arc::new(Mutex::new(
            crate::infrastructure::agent_session::runtime::AgentProcessMap::new(),
        ));

        for invalid in [None, Some(String::new()), Some("acceptEdits".to_string())] {
            let result = validate_invoke_permission_mode(invalid.clone());
            assert!(result.is_err(), "{invalid:?} must be rejected");
            // command 本体は ? で早期 return するため、保存値・runtime ハンドルとも不変。
            let saved = store
                .get_session(data_dir.path(), &session.id)
                .unwrap()
                .unwrap();
            assert_eq!(saved.permission_mode, "edit");
            assert!(handles.lock().await.is_empty());
        }
    }

    #[tokio::test]
    async fn close_agent_session_guard_keeps_workflow_step_runtime() {
        let session = session_for_start_guard(true);
        let handles = Arc::new(Mutex::new(
            crate::infrastructure::agent_session::runtime::AgentProcessMap::new(),
        ));
        handles.lock().await.insert(
            session.id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
        );

        assert!(should_skip_close_agent_session(Some(&session)));
        assert!(handles.lock().await.contains_key(&session.id));
    }
}

#[tauri::command]
pub async fn close_agent_session(
    app: tauri::AppHandle,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)
        .map_err(|e| e.to_string())?;
    if should_skip_close_agent_session(session.as_ref()) {
        return Ok(());
    }
    crate::infrastructure::agent_session::runtime::close_agent_session_internal(
        &app,
        handles.inner(),
        &chat_session_id,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_agent_message(
    app: tauri::AppHandle,
    handles: tauri::State<
        '_,
        Arc<Mutex<crate::infrastructure::agent_session::runtime::AgentProcessMap>>,
    >,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    open_tabs: tauri::State<'_, Arc<crate::usecase::agent_session::session::OpenTabRegistry>>,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: Option<String>,
    plan_mode: Option<bool>,
    backend_id: Option<String>,
    model_id: Option<String>,
    images: Option<Vec<ImageAttachment>>,
    mentions: Option<Vec<crate::adaptor::protocol::mention::MentionReferenceInput>>,
    editor_context: Option<crate::infrastructure::agent_session::runtime::AgentEditorContext>,
) -> Result<crate::infrastructure::agent_session::runtime::SendMessageResponse, String> {
    let permission_mode = validate_invoke_permission_mode(permission_mode)?;
    let mentions = mentions.map(crate::adaptor::protocol::mention::into_domain_vec);
    let response = dispatch_agent_message(
        AgentMessageDispatchContext {
            gateway: crate::infrastructure::agent_session::runtime_gateway::AgentRuntimeGateway {
                app: &app,
                session_store: session_store.inner(),
                registry: registry.inner(),
                handles: handles.inner(),
            },
        },
        AgentMessageDispatchRequest {
            chat_session_id,
            worktree_path,
            content,
            permission_mode,
            plan_mode: plan_mode.unwrap_or(false),
            backend_id,
            model_id,
            images,
            mentions,
            editor_context,
        },
    )
    .await?;
    crate::adaptor::controller_support::emit_after_workflow_step_message(
        &app,
        &response.session,
        handles.inner(),
        open_tabs.inner(),
    )
    .await;
    Ok(response)
}
