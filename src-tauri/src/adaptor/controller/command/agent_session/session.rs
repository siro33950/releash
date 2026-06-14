use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use tauri::Emitter;
use tokio::sync::Mutex;

use crate::agent_message_dispatcher::{
    dispatch_agent_message, AgentMessageDispatchContext, AgentMessageDispatchRequest,
};
use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime::codex::configured_cli_path;
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    build_account_rate_limits_read_request, build_account_read_request,
    build_account_usage_read_request, build_app_list_request,
    build_collaboration_mode_list_request, build_config_read_request,
    build_config_requirements_read_request, build_hooks_list_request,
    build_mcp_server_status_list_request, build_model_provider_capabilities_read_request,
    build_permission_profile_list_request, build_plugin_list_request,
    build_thread_goal_clear_request, build_thread_goal_get_request, build_thread_goal_set_request,
    build_thread_list_request, build_thread_read_request,
    build_thread_realtime_list_voices_request, build_thread_search_request,
    build_thread_turn_items_list_request, build_thread_turns_list_request, CodexAppServerProcess,
    METHOD_THREAD_REALTIME_APPEND_AUDIO, METHOD_THREAD_REALTIME_APPEND_TEXT,
    METHOD_THREAD_REALTIME_START, METHOD_THREAD_REALTIME_STOP,
};
use crate::infrastructure::agent_session::runtime::{
    AgentBackendRegistry, AgentProcessMap, AgentReviewTarget, ImageAttachment, SessionConfig,
    SessionHandle, TurnPhase, CODEX_BACKEND_ID,
};
use crate::usecase::agent_session::session::errors::session_target_rejected;
use crate::usecase::agent_session::session::{
    add_message_internal, ChatMessage, ChatSession, MessagePart, MessageRole, SessionState,
    SessionStore,
};
use crate::workflow::engine::WorkflowEngine;

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
pub struct AgentCopyResponse {
    pub title: String,
    pub detail: String,
    pub content: String,
    pub ordinal: usize,
    pub message_id: String,
    pub suggested_path: String,
    pub code_blocks: Vec<AgentCopyCodeBlock>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCopyCodeBlock {
    pub index: usize,
    pub language: Option<String>,
    pub label: String,
    pub content: String,
    pub line_count: usize,
    pub suggested_path: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCopyWriteResult {
    pub title: String,
    pub detail: String,
    pub path: String,
    pub byte_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextCompactResult {
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackgroundTerminalCleanResult {
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCodexShellCommandResult {
    pub title: String,
    pub detail: String,
    pub command: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCodexRealtimeResult {
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewStartResult {
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCodexAccountStatusResult {
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCodexRuntimeInventoryResult {
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCodexGoal {
    pub objective: String,
    pub status: String,
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub time_used_seconds: u64,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCodexGoalResult {
    pub title: String,
    pub detail: String,
    pub goal: Option<AgentCodexGoal>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCodexPermissionProfile {
    pub id: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentExportTranscriptResult {
    pub title: String,
    pub detail: String,
    pub content: String,
    pub path: Option<String>,
    pub suggested_path: Option<String>,
    pub message_count: usize,
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

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptHistoryEntry {
    pub text: String,
    pub scope: String,
    pub session_id: Option<String>,
    pub worktree_path: Option<String>,
    pub timestamp: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptHistorySearchRequest {
    pub chat_session_id: String,
    pub worktree_path: String,
    pub query: String,
    pub scope: Option<String>,
    pub local_history: Option<Vec<String>>,
    pub limit: Option<usize>,
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

fn parse_copy_ordinal(raw: Option<&str>) -> Result<usize, String> {
    const ERROR: &str = "Copy response index must be a positive number.";
    let raw = raw.unwrap_or_default().trim();
    if raw.is_empty() {
        return Ok(1);
    }
    let parts = raw.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 1 {
        return Err(ERROR.to_string());
    }
    let ordinal = parts[0].parse::<usize>().map_err(|_| ERROR.to_string())?;
    if ordinal == 0 {
        return Err(ERROR.to_string());
    }
    Ok(ordinal)
}

fn copyable_agent_text(message: &ChatMessage) -> Option<String> {
    if message.role != crate::usecase::agent_session::session::MessageRole::Agent {
        return None;
    }
    let text = message
        .parts
        .as_ref()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Text { content, .. } => Some(content.as_str()),
                    _ => None,
                })
                .collect::<String>()
        })
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            if message.content.trim().is_empty() {
                None
            } else {
                Some(message.content.clone())
            }
        })?;
    Some(text)
}

fn parse_copy_code_blocks(content: &str) -> Vec<AgentCopyCodeBlock> {
    let mut blocks = Vec::new();
    let mut current_language: Option<String> = None;
    let mut current_content = String::new();
    let mut in_block = false;

    for raw_line in content.split_inclusive('\n') {
        let line_body = raw_line.trim_end_matches(['\r', '\n']);
        let trimmed = line_body.trim_start();
        if trimmed.starts_with("```") {
            if in_block {
                let block_content = std::mem::take(&mut current_content);
                let line_count = block_content.lines().count().max(1);
                let index = blocks.len() + 1;
                let language = current_language.take();
                let language_label = language.as_deref().unwrap_or("plain text").to_string();
                blocks.push(AgentCopyCodeBlock {
                    index,
                    language,
                    label: format!("Block {index} ({language_label}, {line_count} lines)"),
                    content: block_content,
                    line_count,
                    suggested_path: String::new(),
                });
                in_block = false;
            } else {
                let info = trimmed.trim_start_matches('`').trim();
                current_language = info
                    .split_whitespace()
                    .next()
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());
                current_content.clear();
                in_block = true;
            }
            continue;
        }
        if in_block {
            current_content.push_str(raw_line);
        }
    }

    blocks
}

fn copy_code_block_extension(language: Option<&str>) -> &'static str {
    match language.unwrap_or_default().to_ascii_lowercase().as_str() {
        "bash" | "sh" | "shell" | "shellscript" | "zsh" => "sh",
        "c++" | "cpp" => "cpp",
        "csharp" | "cs" => "cs",
        "go" | "golang" => "go",
        "javascript" | "js" | "jsx" => "js",
        "json" => "json",
        "markdown" | "md" => "md",
        "python" | "py" => "py",
        "rust" | "rs" => "rs",
        "typescript" | "ts" | "tsx" => "ts",
        "yaml" | "yml" => "yml",
        _ => "txt",
    }
}

fn suggested_agent_copy_response_path(message_id: &str) -> String {
    format!(
        "snippets/releash-response-{}.md",
        sanitized_export_session_id(message_id)
    )
}

fn suggested_agent_copy_code_block_path(message_id: &str, block: &AgentCopyCodeBlock) -> String {
    format!(
        "snippets/releash-response-{}-block-{}.{}",
        sanitized_export_session_id(message_id),
        block.index,
        copy_code_block_extension(block.language.as_deref())
    )
}

pub(crate) fn build_agent_copy_response_inner(
    session: &ChatSession,
    raw: Option<&str>,
    exclude_message_id: Option<&str>,
) -> Result<AgentCopyResponse, String> {
    let ordinal = parse_copy_ordinal(raw)?;
    let mut matches = session
        .messages
        .iter()
        .rev()
        .filter(|message| exclude_message_id != Some(message.id.as_str()))
        .filter_map(|message| copyable_agent_text(message).map(|content| (message, content)));
    let (message, content) = matches.nth(ordinal - 1).ok_or_else(|| {
        if ordinal == 1 {
            "No completed agent response is available to copy.".to_string()
        } else {
            format!("No completed agent response #{ordinal} is available to copy.")
        }
    })?;
    let mut code_blocks = parse_copy_code_blocks(&content);
    for block in &mut code_blocks {
        block.suggested_path = suggested_agent_copy_code_block_path(&message.id, block);
    }
    Ok(AgentCopyResponse {
        title: if ordinal == 1 {
            "Copied latest response".to_string()
        } else {
            format!("Copied response #{ordinal}")
        },
        detail: if ordinal == 1 {
            "Copied the latest completed agent response.".to_string()
        } else {
            format!("Copied the {ordinal}th latest completed agent response.")
        },
        suggested_path: suggested_agent_copy_response_path(&message.id),
        code_blocks,
        content,
        ordinal,
        message_id: message.id.clone(),
    })
}

pub(crate) fn write_agent_copy_selection_to_file_inner(
    worktree_path: &str,
    raw_path: &str,
    content: &str,
) -> Result<AgentCopyWriteResult, String> {
    if content.is_empty() {
        return Err("No copy selection content is available to write.".to_string());
    }
    let path = resolve_export_transcript_path(worktree_path, raw_path)?;
    std::fs::write(&path, content).map_err(|e| format!("Failed to write copy selection: {e}"))?;
    let path = path.to_string_lossy().to_string();
    Ok(AgentCopyWriteResult {
        title: "Copy selection written".to_string(),
        detail: format!("Wrote {} bytes to {path}", content.len()),
        path,
        byte_count: content.len(),
    })
}

fn resolve_export_transcript_path(worktree_path: &str, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Export filename is required.".to_string());
    }
    if raw.contains('\0') || raw.contains('\n') || raw.contains('\r') {
        return Err("Export filename contains unsupported characters".to_string());
    }
    let candidate = Path::new(raw);
    if candidate.is_absolute() {
        return Err("Export filename must be relative to the worktree".to_string());
    }

    let mut clean = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("Export filename must stay inside the worktree".to_string());
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err("Export filename must be relative to the worktree".to_string());
            }
        }
    }
    if clean.as_os_str().is_empty() || clean.file_name().is_none() {
        return Err("Export filename is required.".to_string());
    }

    let canonical_root = Path::new(worktree_path)
        .canonicalize()
        .map_err(|e| format!("Failed to resolve worktree root: {e}"))?;
    let target = canonical_root.join(clean);
    if target.exists() {
        return Err(format!(
            "Export target already exists: {}",
            target.display()
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| "Export target has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create export directory: {e}"))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| format!("Failed to resolve export directory: {e}"))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err("Export filename must stay inside the worktree".to_string());
    }
    Ok(target)
}

fn build_agent_transcript_content(session: &ChatSession) -> String {
    let mut content = String::new();
    content.push_str("# Releash Agent Transcript\n\n");
    content.push_str(&format!("Session: {}\n", session.id));
    content.push_str(&format!("Worktree: {}\n", session.worktree_path));
    content.push_str(&format!("Messages: {}\n\n", session.messages.len()));

    for message in &session.messages {
        let text = message_search_text(message);
        content.push_str(&format!(
            "## {} - {} - {}\n\n",
            role_label(&message.role),
            message.id,
            message.timestamp
        ));
        if text.trim().is_empty() {
            content.push_str("(no text content)\n\n");
        } else {
            content.push_str(text.trim());
            content.push_str("\n\n");
        }
    }
    content
}

fn sanitized_export_session_id(session_id: &str) -> String {
    let mut sanitized = String::new();
    let mut last_was_separator = false;
    for ch in session_id.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if matches!(ch, '-' | '_' | '.') {
            sanitized.push(ch);
            last_was_separator = false;
        } else if !last_was_separator {
            sanitized.push('-');
            last_was_separator = true;
        }
    }
    let trimmed = sanitized.trim_matches(['-', '_', '.']);
    if trimmed.is_empty() {
        "session".to_string()
    } else {
        trimmed.to_string()
    }
}

fn suggested_agent_transcript_export_path(session: &ChatSession) -> String {
    format!(
        "transcripts/releash-agent-{}.md",
        sanitized_export_session_id(&session.id)
    )
}

pub(crate) fn build_agent_export_transcript_inner(
    session: &ChatSession,
    raw: Option<&str>,
) -> Result<AgentExportTranscriptResult, String> {
    let content = build_agent_transcript_content(session);
    let suggested_path = suggested_agent_transcript_export_path(session);
    let filename = raw.unwrap_or_default().trim();
    if filename.is_empty() {
        return Ok(AgentExportTranscriptResult {
            title: "Transcript ready".to_string(),
            detail: format!(
                "Prepared {} message{} for clipboard export.",
                session.messages.len(),
                if session.messages.len() == 1 { "" } else { "s" }
            ),
            content,
            path: None,
            suggested_path: Some(suggested_path),
            message_count: session.messages.len(),
        });
    }

    let path = resolve_export_transcript_path(&session.worktree_path, filename)?;
    std::fs::write(&path, &content).map_err(|e| format!("Failed to write transcript: {e}"))?;
    Ok(AgentExportTranscriptResult {
        title: "Transcript exported".to_string(),
        detail: format!(
            "Wrote {} message transcript to {}",
            session.messages.len(),
            path.display()
        ),
        content,
        path: Some(path.to_string_lossy().to_string()),
        suggested_path: Some(filename.to_string()),
        message_count: session.messages.len(),
    })
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

fn human_prompt_text(message: &ChatMessage) -> Option<String> {
    if message.role != crate::usecase::agent_session::session::MessageRole::Human {
        return None;
    }
    let text = message_search_text(message);
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        None
    } else {
        Some(compact)
    }
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

fn normalize_prompt_history_scope(scope: Option<&str>) -> Result<&'static str, String> {
    match scope.unwrap_or("session").trim().to_lowercase().as_str() {
        "" | "session" => Ok("session"),
        "project" | "worktree" => Ok("project"),
        "all" | "global" => Ok("all"),
        other => Err(format!(
            "Unknown prompt history scope: {other}. Available: session, project, all"
        )),
    }
}

fn prompt_history_scope_matches(
    scope: &str,
    session: &ChatSession,
    chat_session_id: &str,
    worktree_path: &str,
) -> bool {
    match scope {
        "session" => session.id == chat_session_id,
        "project" => session.worktree_path == worktree_path,
        "all" => true,
        _ => false,
    }
}

fn search_prompt_history_entries(
    sessions: Vec<ChatSession>,
    chat_session_id: &str,
    worktree_path: &str,
    query: &str,
    scope: &str,
    local_history: Vec<String>,
    limit: usize,
) -> Vec<AgentPromptHistoryEntry> {
    let needle = query.trim().to_lowercase();
    let max_results = limit.max(1);
    let mut candidates = Vec::new();

    for (index, prompt) in local_history.into_iter().rev().enumerate() {
        let text = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        candidates.push(AgentPromptHistoryEntry {
            text,
            scope: "session".to_string(),
            session_id: Some(chat_session_id.to_string()),
            worktree_path: Some(worktree_path.to_string()),
            timestamp: f64::MAX - index as f64,
        });
    }

    for session in sessions {
        if session.workflow_step_session || session.state == SessionState::Archived {
            continue;
        }
        if !prompt_history_scope_matches(scope, &session, chat_session_id, worktree_path) {
            continue;
        }
        for message in &session.messages {
            let Some(text) = human_prompt_text(message) else {
                continue;
            };
            candidates.push(AgentPromptHistoryEntry {
                text,
                scope: if session.id == chat_session_id {
                    "session".to_string()
                } else if session.worktree_path == worktree_path {
                    "project".to_string()
                } else {
                    "all".to_string()
                },
                session_id: Some(session.id.clone()),
                worktree_path: Some(session.worktree_path.clone()),
                timestamp: message.timestamp,
            });
        }
    }

    candidates.sort_by(|a, b| {
        b.timestamp
            .partial_cmp(&a.timestamp)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for entry in candidates {
        if !needle.is_empty() && !entry.text.to_lowercase().contains(&needle) {
            continue;
        }
        if !seen.insert(entry.text.clone()) {
            continue;
        }
        results.push(entry);
        if results.len() >= max_results {
            break;
        }
    }
    results
}

fn role_label(role: &crate::usecase::agent_session::session::MessageRole) -> &'static str {
    match role {
        crate::usecase::agent_session::session::MessageRole::Human => "Human",
        crate::usecase::agent_session::session::MessageRole::Agent => "Agent",
        crate::usecase::agent_session::session::MessageRole::System => "System",
    }
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
pub async fn search_agent_prompt_history(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    request: AgentPromptHistorySearchRequest,
) -> Result<Vec<AgentPromptHistoryEntry>, String> {
    let scope = normalize_prompt_history_scope(request.scope.as_deref())?;
    let data_dir = resolve_data_dir(&app)?;
    let sessions = if scope == "all" {
        session_store.list_all_sessions(&data_dir)?
    } else {
        session_store.list_worktree_sessions(&data_dir, &request.worktree_path)?
    };
    Ok(search_prompt_history_entries(
        sessions,
        &request.chat_session_id,
        &request.worktree_path,
        &request.query,
        scope,
        request.local_history.unwrap_or_default(),
        request.limit.unwrap_or(20),
    ))
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
pub async fn build_agent_copy_response(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    raw: Option<String>,
    exclude_message_id: Option<String>,
) -> Result<AgentCopyResponse, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    build_agent_copy_response_inner(&session, raw.as_deref(), exclude_message_id.as_deref())
}

#[tauri::command]
pub fn write_agent_copy_selection_to_file(
    worktree_path: String,
    raw_path: String,
    content: String,
) -> Result<AgentCopyWriteResult, String> {
    write_agent_copy_selection_to_file_inner(&worktree_path, &raw_path, &content)
}

#[tauri::command]
pub async fn build_agent_export_transcript(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    raw: Option<String>,
) -> Result<AgentExportTranscriptResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    build_agent_export_transcript_inner(&session, raw.as_deref())
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
pub async fn compact_agent_context(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
) -> Result<AgentContextCompactResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let backend_id = session
        .backend_id
        .clone()
        .or_else(|| registry.resolve_default_id().ok())
        .ok_or_else(|| "Agent backend is not configured".to_string())?;
    let backend = registry
        .get(&backend_id)
        .ok_or_else(|| format!("Agent backend not found: {backend_id}"))?;

    backend
        .compact_session(&SessionHandle {
            chat_session_id: chat_session_id.clone(),
            backend_id: backend_id.clone(),
        })
        .await?;

    Ok(AgentContextCompactResult {
        title: "Context compaction started".to_string(),
        detail: format!("Requested runtime compaction for {backend_id}."),
    })
}

#[tauri::command]
pub async fn clean_codex_background_terminals(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
) -> Result<AgentBackgroundTerminalCleanResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let backend_id = session
        .backend_id
        .as_deref()
        .unwrap_or(crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID);
    if backend_id != CODEX_BACKEND_ID {
        return Err("Codex background terminal cleanup requires a Codex session".to_string());
    }
    if session.workflow_step_session {
        return Err("Workflow step sessions cannot clean Codex background terminals".to_string());
    }
    let backend = registry
        .get(CODEX_BACKEND_ID)
        .ok_or_else(|| "Codex backend is not configured".to_string())?;

    backend
        .clean_background_terminals(&SessionHandle {
            chat_session_id: chat_session_id.clone(),
            backend_id: CODEX_BACKEND_ID.to_string(),
        })
        .await?;

    Ok(AgentBackgroundTerminalCleanResult {
        title: "Codex background terminals cleaned".to_string(),
        detail: "Requested runtime cleanup for this thread's background terminals.".to_string(),
    })
}

#[tauri::command]
pub async fn run_codex_shell_command(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
    content: String,
) -> Result<AgentCodexShellCommandResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let backend_id = session
        .backend_id
        .as_deref()
        .unwrap_or(crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID);
    if backend_id != CODEX_BACKEND_ID {
        return Err("Codex shell command requires a Codex session".to_string());
    }
    if session.workflow_step_session {
        return Err("Workflow step sessions cannot run Codex shell commands".to_string());
    }
    let command =
        crate::adaptor::controller::command::agent_session::shell::parse_agent_runtime_shell_command(
            &content,
        )?
        .ok_or_else(|| "Type a shell command that starts with ! before running Codex shell command.".to_string())?;
    let backend = registry
        .get(CODEX_BACKEND_ID)
        .ok_or_else(|| "Codex backend is not configured".to_string())?;

    backend
        .run_shell_command(
            &SessionHandle {
                chat_session_id: chat_session_id.clone(),
                backend_id: CODEX_BACKEND_ID.to_string(),
            },
            &command,
        )
        .await?;

    Ok(AgentCodexShellCommandResult {
        title: "Codex shell command sent".to_string(),
        detail: format!("Runtime shell command sent for this thread: {command}"),
        command,
    })
}

fn ensure_codex_interactive_session(session: &ChatSession, action: &str) -> Result<(), String> {
    let backend_id = session
        .backend_id
        .as_deref()
        .unwrap_or(crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID);
    if backend_id != CODEX_BACKEND_ID {
        return Err(format!("{action} requires a Codex session"));
    }
    if session.workflow_step_session {
        return Err(format!("Workflow step sessions cannot {action}"));
    }
    Ok(())
}

#[tauri::command]
pub async fn start_codex_realtime_text_session(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
    content: String,
) -> Result<AgentCodexRealtimeResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    ensure_codex_interactive_session(&session, "start Codex realtime text")?;
    let backend = registry
        .get(CODEX_BACKEND_ID)
        .ok_or_else(|| "Codex backend is not configured".to_string())?;
    let prompt = content.trim();
    backend
        .start_realtime_text_session(
            &SessionHandle {
                chat_session_id: chat_session_id.clone(),
                backend_id: CODEX_BACKEND_ID.to_string(),
            },
            (!prompt.is_empty()).then_some(prompt),
        )
        .await?;

    Ok(AgentCodexRealtimeResult {
        title: "Codex realtime text started".to_string(),
        detail: if prompt.is_empty() {
            "Started a runtime realtime text session for this thread.".to_string()
        } else {
            "Started a runtime realtime text session with the current composer draft as prompt."
                .to_string()
        },
    })
}

#[tauri::command]
pub async fn append_codex_realtime_text(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
    content: String,
) -> Result<AgentCodexRealtimeResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    ensure_codex_interactive_session(&session, "append Codex realtime text")?;
    let text = content.trim();
    if text.is_empty() {
        return Err("Type realtime text in the composer before appending.".to_string());
    }
    let backend = registry
        .get(CODEX_BACKEND_ID)
        .ok_or_else(|| "Codex backend is not configured".to_string())?;
    backend
        .append_realtime_text(
            &SessionHandle {
                chat_session_id: chat_session_id.clone(),
                backend_id: CODEX_BACKEND_ID.to_string(),
            },
            text,
        )
        .await?;

    Ok(AgentCodexRealtimeResult {
        title: "Codex realtime text sent".to_string(),
        detail: "Appended the current composer draft to the runtime realtime session.".to_string(),
    })
}

#[tauri::command]
pub async fn stop_codex_realtime_session(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
) -> Result<AgentCodexRealtimeResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    ensure_codex_interactive_session(&session, "stop Codex realtime")?;
    let backend = registry
        .get(CODEX_BACKEND_ID)
        .ok_or_else(|| "Codex backend is not configured".to_string())?;
    backend
        .stop_realtime_session(&SessionHandle {
            chat_session_id: chat_session_id.clone(),
            backend_id: CODEX_BACKEND_ID.to_string(),
        })
        .await?;

    Ok(AgentCodexRealtimeResult {
        title: "Codex realtime stopped".to_string(),
        detail: "Requested runtime realtime stop for this thread.".to_string(),
    })
}

#[tauri::command]
pub async fn start_codex_uncommitted_changes_review(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    target_type: Option<String>,
    target_value: Option<String>,
) -> Result<AgentReviewStartResult, String> {
    let target = parse_codex_review_target(target_type.as_deref(), target_value.as_deref())?;
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let backend_id = session
        .backend_id
        .as_deref()
        .unwrap_or(crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID);
    if backend_id != CODEX_BACKEND_ID {
        return Err("Codex review requires a Codex session".to_string());
    }
    if session.workflow_step_session {
        return Err("Workflow step sessions cannot start Codex review".to_string());
    }

    let turn_phase = {
        let map = handles.lock().await;
        map.get(&chat_session_id).map(|proc| proc.turn_phase)
    };
    if matches!(
        turn_phase,
        Some(TurnPhase::Streaming | TurnPhase::WaitingPermission)
    ) {
        return Err("Codex review cannot start while the agent is busy".to_string());
    }

    let backend = registry
        .get(CODEX_BACKEND_ID)
        .ok_or_else(|| "Codex backend is not configured".to_string())?;
    let agent_message = add_message_internal(
        &session_store,
        &data_dir,
        &chat_session_id,
        MessageRole::Agent,
        "",
        None,
        None,
    )?;

    backend
        .review(
            &SessionHandle {
                chat_session_id: chat_session_id.clone(),
                backend_id: CODEX_BACKEND_ID.to_string(),
            },
            &agent_message.id,
            target.clone(),
        )
        .await?;

    Ok(AgentReviewStartResult {
        title: "Codex review started".to_string(),
        detail: format!(
            "Runtime review started for {}.",
            codex_review_target_label(&target)
        ),
    })
}

fn parse_codex_review_target(
    target_type: Option<&str>,
    target_value: Option<&str>,
) -> Result<AgentReviewTarget, String> {
    let target_type = target_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("uncommittedChanges");
    let target_value = target_value
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match target_type {
        "uncommittedChanges" => Ok(AgentReviewTarget::UncommittedChanges),
        "baseBranch" => Ok(AgentReviewTarget::BaseBranch {
            branch: target_value
                .ok_or_else(|| "Codex base branch review requires a branch name".to_string())?
                .to_string(),
        }),
        "commit" => Ok(AgentReviewTarget::Commit {
            sha: target_value
                .ok_or_else(|| "Codex commit review requires a commit SHA".to_string())?
                .to_string(),
        }),
        "custom" => Ok(AgentReviewTarget::Custom {
            instructions: target_value
                .ok_or_else(|| "Codex custom review requires instructions".to_string())?
                .to_string(),
        }),
        other => Err(format!("Unsupported Codex review target: {other}")),
    }
}

fn codex_review_target_label(target: &AgentReviewTarget) -> String {
    match target {
        AgentReviewTarget::UncommittedChanges => "uncommitted changes".to_string(),
        AgentReviewTarget::BaseBranch { branch } => format!("base branch {branch}"),
        AgentReviewTarget::Commit { sha } => format!("commit {sha}"),
        AgentReviewTarget::Custom { .. } => "custom instructions".to_string(),
    }
}

fn saved_codex_thread_id_for_runtime_action<'a>(
    session: &'a ChatSession,
    action_label: &str,
) -> Result<&'a str, String> {
    let backend_id = session
        .backend_id
        .as_deref()
        .unwrap_or(crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID);
    if backend_id != CODEX_BACKEND_ID {
        return Err(format!("{action_label} requires a Codex session"));
    }
    if session.workflow_step_session {
        return Err(format!("Workflow step sessions cannot use {action_label}"));
    }
    session
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!("Codex thread is not ready; start or resume the session before using {action_label}.")
        })
}

fn saved_codex_thread_id_for_goal(session: &ChatSession) -> Result<&str, String> {
    saved_codex_thread_id_for_runtime_action(session, "Codex goal management")
}

fn parse_codex_goal(value: Option<&serde_json::Value>) -> Option<AgentCodexGoal> {
    let goal = value?;
    Some(AgentCodexGoal {
        objective: goal
            .get("objective")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        status: goal
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("active")
            .to_string(),
        token_budget: goal.get("tokenBudget").and_then(serde_json::Value::as_u64),
        tokens_used: goal
            .get("tokensUsed")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        time_used_seconds: goal
            .get("timeUsedSeconds")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
    })
}

fn codex_goal_result(title: &str, goal: Option<AgentCodexGoal>) -> AgentCodexGoalResult {
    let detail = match goal.as_ref() {
        Some(goal) => {
            let budget = goal
                .token_budget
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string());
            format!(
                "Status: {}\nTokens: {} / {}\nElapsed: {}s\n{}",
                goal.status, goal.tokens_used, budget, goal.time_used_seconds, goal.objective
            )
        }
        None => "No Codex goal is set for this thread.".to_string(),
    };
    AgentCodexGoalResult {
        title: title.to_string(),
        detail,
        goal,
    }
}

fn emit_codex_goal_event(
    app: &tauri::AppHandle,
    chat_session_id: &str,
    thread_id: &str,
    goal: Option<&AgentCodexGoal>,
) {
    let payload = match goal {
        Some(goal) => serde_json::json!({
            "type": "codex_goal_updated",
            "chat_session_id": chat_session_id,
            "thread_id": thread_id,
            "goal": goal,
        }),
        None => serde_json::json!({
            "type": "codex_goal_cleared",
            "chat_session_id": chat_session_id,
            "thread_id": thread_id,
        }),
    };
    let _ = app.emit("agent-sdk-message", &payload);
}

async fn with_codex_goal_process(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    f: impl FnOnce(u64, &str) -> serde_json::Value,
) -> Result<(String, serde_json::Value), String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let thread_id = saved_codex_thread_id_for_goal(&session)?.to_string();

    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        process.send(&f(id, &thread_id)).await?;
        process.read_response_result(id).await
    }
    .await;
    process.shutdown().await;
    result.map(|value| (thread_id, value))
}

#[tauri::command]
pub async fn read_codex_thread_goal(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
) -> Result<AgentCodexGoalResult, String> {
    let app_for_emit = app.clone();
    let chat_session_id_for_emit = chat_session_id.clone();
    let (thread_id, result) =
        with_codex_goal_process(app, session_store, chat_session_id, |id, thread_id| {
            build_thread_goal_get_request(id, thread_id)
        })
        .await?;
    let goal = parse_codex_goal(result.get("goal"));
    emit_codex_goal_event(
        &app_for_emit,
        &chat_session_id_for_emit,
        &thread_id,
        goal.as_ref(),
    );
    Ok(codex_goal_result("Codex goal", goal))
}

#[tauri::command]
pub async fn set_codex_thread_goal(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    objective: Option<String>,
    status: Option<String>,
    token_budget: Option<u64>,
) -> Result<AgentCodexGoalResult, String> {
    if objective
        .as_deref()
        .is_some_and(|value| value.trim().chars().count() > 4000)
    {
        return Err("Codex goal objective must be at most 4000 characters".to_string());
    }
    let app_for_emit = app.clone();
    let chat_session_id_for_emit = chat_session_id.clone();
    let (thread_id, result) =
        with_codex_goal_process(app, session_store, chat_session_id, |id, thread_id| {
            build_thread_goal_set_request(
                id,
                thread_id,
                objective.as_deref(),
                status.as_deref(),
                token_budget,
            )
        })
        .await?;
    let goal = parse_codex_goal(result.get("goal"));
    emit_codex_goal_event(
        &app_for_emit,
        &chat_session_id_for_emit,
        &thread_id,
        goal.as_ref(),
    );
    Ok(codex_goal_result("Codex goal updated", goal))
}

#[tauri::command]
pub async fn clear_codex_thread_goal(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
) -> Result<AgentCodexGoalResult, String> {
    let app_for_emit = app.clone();
    let chat_session_id_for_emit = chat_session_id.clone();
    let (thread_id, _) =
        with_codex_goal_process(app, session_store, chat_session_id, |id, thread_id| {
            build_thread_goal_clear_request(id, thread_id)
        })
        .await?;
    emit_codex_goal_event(&app_for_emit, &chat_session_id_for_emit, &thread_id, None);
    Ok(codex_goal_result("Codex goal cleared", None))
}

#[tauri::command]
pub async fn read_codex_account_status(
    app: tauri::AppHandle,
) -> Result<AgentCodexAccountStatusResult, String> {
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;

        let account_id = process.next_request_id();
        process
            .send(&build_account_read_request(account_id))
            .await?;
        let account = process.read_response_result(account_id).await?;

        let usage_id = process.next_request_id();
        process
            .send(&build_account_usage_read_request(usage_id))
            .await?;
        let usage = process.read_response_result(usage_id).await?;

        let limits_id = process.next_request_id();
        process
            .send(&build_account_rate_limits_read_request(limits_id))
            .await?;
        let rate_limits = process.read_response_result(limits_id).await?;

        Ok(build_codex_account_status_result(
            &account,
            &usage,
            &rate_limits,
        ))
    }
    .await;
    process.shutdown().await;
    result
}

#[tauri::command]
pub async fn read_codex_realtime_voices_report(
    app: tauri::AppHandle,
) -> Result<AgentCodexRuntimeInventoryResult, String> {
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        process
            .send(&build_thread_realtime_list_voices_request(id))
            .await?;
        let voices = process.read_response_result(id).await?;
        Ok(build_codex_realtime_voices_report_result(&voices))
    }
    .await;
    process.shutdown().await;
    result
}

#[tauri::command]
pub async fn read_codex_runtime_config_report(
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<AgentCodexRuntimeInventoryResult, String> {
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;

        let config_id = process.next_request_id();
        process
            .send(&build_config_read_request(config_id, &worktree_path, true))
            .await?;
        let config = process.read_response_result(config_id).await?;

        let requirements_id = process.next_request_id();
        process
            .send(&build_config_requirements_read_request(requirements_id))
            .await?;
        let requirements = process.read_response_result(requirements_id).await?;

        Ok(build_codex_runtime_config_report_result(
            &config,
            &requirements,
            &worktree_path,
        ))
    }
    .await;
    process.shutdown().await;
    result
}

#[tauri::command]
pub async fn read_codex_runtime_capabilities_report(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    worktree_path: String,
) -> Result<AgentCodexRuntimeInventoryResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store.get_session(&data_dir, &chat_session_id)?;
    let thread_id = session
        .as_ref()
        .and_then(|session| session.agent_session_id.clone());

    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;

        let capabilities_id = process.next_request_id();
        process
            .send(&build_model_provider_capabilities_read_request(
                capabilities_id,
            ))
            .await?;
        let capabilities = process.read_response_result(capabilities_id).await?;

        let modes_id = process.next_request_id();
        process
            .send(&build_collaboration_mode_list_request(modes_id))
            .await?;
        let collaboration_modes = process.read_response_result(modes_id).await?;

        let mut app_pages = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..10 {
            let id = process.next_request_id();
            process
                .send(&build_app_list_request(
                    id,
                    thread_id.as_deref(),
                    cursor.as_deref(),
                ))
                .await?;
            let page = process.read_response_result(id).await?;
            cursor = page
                .get("nextCursor")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            app_pages.push(page);
            if cursor.is_none() {
                break;
            }
        }

        let plugins_id = process.next_request_id();
        process
            .send(&build_plugin_list_request(plugins_id, &worktree_path))
            .await?;
        let plugins = process.read_response_result(plugins_id).await?;

        Ok(build_codex_runtime_capabilities_report_result(
            &capabilities,
            &collaboration_modes,
            &app_pages,
            &plugins,
        ))
    }
    .await;
    process.shutdown().await;
    result
}

#[tauri::command]
pub async fn read_codex_hooks_report(
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<AgentCodexRuntimeInventoryResult, String> {
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        process
            .send(&build_hooks_list_request(id, &worktree_path))
            .await?;
        let hooks = process.read_response_result(id).await?;
        Ok(build_codex_hooks_report_result(&hooks))
    }
    .await;
    process.shutdown().await;
    result
}

#[tauri::command]
pub async fn read_codex_thread_history_report(
    app: tauri::AppHandle,
    worktree_path: String,
    query: Option<String>,
) -> Result<AgentCodexRuntimeInventoryResult, String> {
    let query = query.unwrap_or_default();
    let query = query.trim();
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let mut cursor: Option<String> = None;
        let mut pages = Vec::new();
        for _ in 0..5 {
            let id = process.next_request_id();
            let request = if query.is_empty() {
                build_thread_list_request(id, &worktree_path, cursor.as_deref())
            } else {
                build_thread_search_request(id, query, cursor.as_deref())
            };
            process.send(&request).await?;
            let page = process.read_response_result(id).await?;
            cursor = page
                .get("nextCursor")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            pages.push(page);
            if cursor.is_none() {
                break;
            }
        }
        Ok(build_codex_thread_history_report_result(
            &pages,
            &worktree_path,
            query,
        ))
    }
    .await;
    process.shutdown().await;
    result
}

async fn read_codex_turn_items_pages(
    process: &mut CodexAppServerProcess,
    thread_id: &str,
    turn_id: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let mut cursor: Option<String> = None;
    let mut items = Vec::new();
    for _ in 0..10 {
        let id = process.next_request_id();
        process
            .send(&build_thread_turn_items_list_request(
                id,
                thread_id,
                turn_id,
                cursor.as_deref(),
            ))
            .await?;
        let page = process.read_response_result(id).await?;
        if let Some(data) = page.get("data").and_then(serde_json::Value::as_array) {
            items.extend(data.iter().cloned());
        }
        cursor = page
            .get("nextCursor")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok(items)
}

async fn read_codex_turn_pages(
    process: &mut CodexAppServerProcess,
    thread_id: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let mut cursor: Option<String> = None;
    let mut turns = Vec::new();
    for _ in 0..20 {
        let id = process.next_request_id();
        process
            .send(&build_thread_turns_list_request(
                id,
                thread_id,
                cursor.as_deref(),
            ))
            .await?;
        let page = process.read_response_result(id).await?;
        if let Some(data) = page.get("data").and_then(serde_json::Value::as_array) {
            for turn in data {
                let mut turn = turn.clone();
                let items_loaded = turn
                    .get("itemsView")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|view| view == "full")
                    && turn
                        .get("items")
                        .and_then(serde_json::Value::as_array)
                        .is_some();
                if !items_loaded {
                    if let Some(turn_id) = turn.get("id").and_then(serde_json::Value::as_str) {
                        let items =
                            read_codex_turn_items_pages(process, thread_id, turn_id).await?;
                        if let Some(turn_obj) = turn.as_object_mut() {
                            turn_obj.insert("items".to_string(), serde_json::Value::Array(items));
                            turn_obj.insert(
                                "itemsView".to_string(),
                                serde_json::Value::String("full".to_string()),
                            );
                        }
                    }
                }
                turns.push(turn);
            }
        }
        cursor = page
            .get("nextCursor")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok(turns)
}

#[tauri::command]
pub async fn read_codex_thread_transcript_report(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
) -> Result<AgentCodexRuntimeInventoryResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let thread_id =
        saved_codex_thread_id_for_runtime_action(&session, "Codex thread transcript")?.to_string();

    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        process
            .send(&build_thread_read_request(id, &thread_id, false))
            .await?;
        let response = process.read_response_result(id).await?;
        let turns = read_codex_turn_pages(&mut process, &thread_id).await?;
        Ok(build_codex_thread_transcript_report_result(
            &response, turns,
        ))
    }
    .await;
    process.shutdown().await;
    result
}

#[tauri::command]
pub async fn read_codex_permission_profiles_report(
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<AgentCodexRuntimeInventoryResult, String> {
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let mut cursor: Option<String> = None;
        let mut pages = Vec::new();
        for _ in 0..20 {
            let id = process.next_request_id();
            process
                .send(&build_permission_profile_list_request(
                    id,
                    &worktree_path,
                    cursor.as_deref(),
                ))
                .await?;
            let page = process.read_response_result(id).await?;
            cursor = page
                .get("nextCursor")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            pages.push(page);
            if cursor.is_none() {
                break;
            }
        }
        Ok(build_codex_permission_profiles_report_result(&pages))
    }
    .await;
    process.shutdown().await;
    result
}

#[tauri::command]
pub async fn read_codex_permission_profiles(
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<AgentCodexPermissionProfile>, String> {
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let mut cursor: Option<String> = None;
        let mut pages = Vec::new();
        for _ in 0..20 {
            let id = process.next_request_id();
            process
                .send(&build_permission_profile_list_request(
                    id,
                    &worktree_path,
                    cursor.as_deref(),
                ))
                .await?;
            let page = process.read_response_result(id).await?;
            cursor = page
                .get("nextCursor")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            pages.push(page);
            if cursor.is_none() {
                break;
            }
        }
        Ok(collect_codex_permission_profiles(&pages))
    }
    .await;
    process.shutdown().await;
    result
}

#[tauri::command]
pub async fn read_codex_mcp_status_report(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
) -> Result<AgentCodexRuntimeInventoryResult, String> {
    let data_dir = resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let thread_id = session.agent_session_id.as_deref();

    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let mut cursor: Option<String> = None;
        let mut pages = Vec::new();
        for _ in 0..20 {
            let id = process.next_request_id();
            process
                .send(&build_mcp_server_status_list_request(
                    id,
                    thread_id,
                    cursor.as_deref(),
                ))
                .await?;
            let page = process.read_response_result(id).await?;
            cursor = page
                .get("nextCursor")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            pages.push(page);
            if cursor.is_none() {
                break;
            }
        }
        Ok(build_codex_mcp_status_report_result(&pages))
    }
    .await;
    process.shutdown().await;
    result
}

fn build_codex_account_status_result(
    account: &serde_json::Value,
    usage: &serde_json::Value,
    rate_limits: &serde_json::Value,
) -> AgentCodexAccountStatusResult {
    let mut lines = Vec::new();
    lines.push("Account".to_string());
    lines.extend(summarize_codex_account(account));
    lines.push(String::new());
    lines.push("Token usage".to_string());
    lines.extend(summarize_codex_token_usage(usage));
    lines.push(String::new());
    lines.push("Rate limits".to_string());
    lines.extend(summarize_codex_rate_limits(rate_limits));

    AgentCodexAccountStatusResult {
        title: "Codex account usage".to_string(),
        detail: lines.join("\n"),
    }
}

fn config_field<'a>(config: &'a serde_json::Value, snake: &str, camel: &str) -> Option<&'a str> {
    config
        .get(snake)
        .or_else(|| config.get(camel))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn config_bool_field(config: &serde_json::Value, snake: &str, camel: &str) -> Option<bool> {
    config
        .get(snake)
        .or_else(|| config.get(camel))
        .and_then(serde_json::Value::as_bool)
}

fn format_codex_config_layer_source(source: &serde_json::Value) -> String {
    let source_type = source
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    match source_type {
        "system" | "user" | "legacyManagedConfigTomlFromFile" => {
            let file = source
                .get("file")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("(unknown file)");
            let profile = source
                .get("profile")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty());
            match profile {
                Some(profile) => format!("{source_type} {file} profile={profile}"),
                None => format!("{source_type} {file}"),
            }
        }
        "project" => {
            let folder = source
                .get("dotCodexFolder")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("(unknown .codex)");
            format!("project {folder}")
        }
        "mdm" => {
            let domain = source
                .get("domain")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("(unknown domain)");
            let key = source
                .get("key")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("(unknown key)");
            format!("mdm {domain}/{key}")
        }
        "enterpriseManaged" => {
            let name = source
                .get("name")
                .and_then(serde_json::Value::as_str)
                .or_else(|| source.get("id").and_then(serde_json::Value::as_str))
                .unwrap_or("(unknown managed layer)");
            format!("enterpriseManaged {name}")
        }
        other => other.to_string(),
    }
}

fn format_codex_config_metadata(metadata: &serde_json::Value) -> String {
    let source = metadata.get("name").unwrap_or(metadata);
    let source = format_codex_config_layer_source(source);
    let version = metadata
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    match version {
        Some(version) => format!("{source} v{version}"),
        None => source,
    }
}

fn codex_config_origin_line(origins: &serde_json::Value, key: &str) -> Option<String> {
    origins
        .get(key)
        .map(format_codex_config_metadata)
        .map(|origin| format!("- {key}: {origin}"))
}

fn codex_requirement_summary(label: &str, value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    if let Some(items) = value.as_array() {
        return Some(format!("{label}: {} allowed", items.len()));
    }
    if let Some(object) = value.as_object() {
        return Some(format!("{label}: {} entrie(s)", object.len()));
    }
    if let Some(boolean) = value.as_bool() {
        return Some(format!("{label}: {}", yes_no(boolean)));
    }
    if let Some(text) = value.as_str() {
        return Some(format!("{label}: {}", truncate_middle(text, 120)));
    }
    Some(format!(
        "{label}: {}",
        truncate_middle(&value.to_string(), 120)
    ))
}

fn build_codex_runtime_config_report_result(
    config_response: &serde_json::Value,
    requirements_response: &serde_json::Value,
    worktree_path: &str,
) -> AgentCodexRuntimeInventoryResult {
    let config = config_response.get("config").unwrap_or(config_response);
    let origins = config_response
        .get("origins")
        .unwrap_or(&serde_json::Value::Null);
    let layers = config_response
        .get("layers")
        .and_then(serde_json::Value::as_array);
    let requirements = requirements_response.get("requirements");

    let mut lines = vec![format!("Effective Codex config for {worktree_path}")];
    push_string_line(&mut lines, "Model", config_field(config, "model", "model"));
    push_string_line(
        &mut lines,
        "Review model",
        config_field(config, "review_model", "reviewModel"),
    );
    push_string_line(
        &mut lines,
        "Model provider",
        config_field(config, "model_provider", "modelProvider"),
    );
    push_string_line(
        &mut lines,
        "Approval policy",
        config_field(config, "approval_policy", "approvalPolicy"),
    );
    push_string_line(
        &mut lines,
        "Sandbox mode",
        config_field(config, "sandbox_mode", "sandboxMode"),
    );
    push_string_line(
        &mut lines,
        "Permission profile",
        config_field(config, "permission_profile", "permissionProfile"),
    );
    if let Some(enabled) = config_bool_field(
        config,
        "model_supports_reasoning_summaries",
        "modelSupportsReasoningSummaries",
    ) {
        lines.push(format!("Reasoning summaries: {}", yes_no(enabled)));
    }

    if let Some(origin_object) = origins.as_object() {
        let mut origin_keys = [
            "model",
            "review_model",
            "model_provider",
            "approval_policy",
            "sandbox_mode",
            "permission_profile",
        ]
        .into_iter()
        .filter_map(|key| codex_config_origin_line(origins, key))
        .collect::<Vec<_>>();
        if origin_keys.is_empty() {
            origin_keys = origin_object
                .keys()
                .take(8)
                .filter_map(|key| codex_config_origin_line(origins, key))
                .collect();
        }
        if !origin_keys.is_empty() {
            lines.push(String::new());
            lines.push("Origins".to_string());
            lines.extend(origin_keys);
            if origin_object.len() > 8 {
                lines.push(format!(
                    "- {} more origin entrie(s)",
                    origin_object.len().saturating_sub(8)
                ));
            }
        }
    }

    lines.push(String::new());
    match layers {
        Some(layers) => {
            lines.push(format!("Layers: {}", layers.len()));
            for layer in layers.iter().take(12) {
                let source = layer
                    .get("name")
                    .map(format_codex_config_layer_source)
                    .unwrap_or_else(|| "unknown".to_string());
                let version = layer
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                let disabled = layer
                    .get("disabledReason")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty());
                match disabled {
                    Some(reason) => lines.push(format!(
                        "- {source} v{version} disabled: {}",
                        truncate_middle(reason, 120)
                    )),
                    None => lines.push(format!("- {source} v{version}")),
                }
            }
            if layers.len() > 12 {
                lines.push(format!(
                    "- {} more layer(s)",
                    layers.len().saturating_sub(12)
                ));
            }
        }
        None => lines.push("Layers: not included by runtime response".to_string()),
    }

    lines.push(String::new());
    lines.push("Requirements".to_string());
    if let Some(requirements) = requirements.filter(|value| !value.is_null()) {
        let fields = [
            ("Approval policies", "allowedApprovalPolicies"),
            ("Sandbox modes", "allowedSandboxModes"),
            ("Permission profiles", "allowedPermissionProfiles"),
            ("Default permissions", "defaultPermissions"),
            ("Web search modes", "allowedWebSearchModes"),
            ("Managed hooks only", "allowManagedHooksOnly"),
            ("Appshots", "allowAppshots"),
            ("Feature requirements", "featureRequirements"),
            ("Network", "network"),
        ];
        let mut added = 0usize;
        for (label, key) in fields {
            if let Some(line) = codex_requirement_summary(label, requirements.get(key)) {
                lines.push(line);
                added += 1;
            }
        }
        if added == 0 {
            lines.push("No enforced requirements returned.".to_string());
        }
    } else {
        lines.push("No requirements configured.".to_string());
    }

    AgentCodexRuntimeInventoryResult {
        title: "Codex runtime config".to_string(),
        detail: lines.join("\n"),
    }
}

fn json_bool(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn json_string<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn format_codex_collaboration_mode(mode: &serde_json::Value) -> String {
    let name = json_string(mode, "name").unwrap_or("(unnamed mode)");
    let mode_kind = json_string(mode, "mode").unwrap_or("default");
    let model = json_string(mode, "model").unwrap_or("default model");
    let reasoning = json_string(mode, "reasoning_effort")
        .or_else(|| json_string(mode, "reasoningEffort"))
        .unwrap_or("default reasoning");
    format!("- {name}: mode={mode_kind}, model={model}, reasoning={reasoning}")
}

fn format_codex_app_line(app: &serde_json::Value) -> String {
    let id = json_string(app, "id").unwrap_or("(unknown app)");
    let name = json_string(app, "name").unwrap_or(id);
    let enabled = yes_no(json_bool(app, "isEnabled"));
    let accessible = yes_no(json_bool(app, "isAccessible"));
    let plugins = app
        .get("pluginDisplayNames")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .take(3)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty());
    let description = json_string(app, "description")
        .or_else(|| {
            app.get("appMetadata")
                .and_then(|metadata| json_string(metadata, "seoDescription"))
        })
        .map(|value| truncate_middle(value, 120));

    let mut line = format!("- {name} ({id}): enabled={enabled}, accessible={accessible}");
    if let Some(plugins) = plugins {
        line.push_str(&format!(", plugins={plugins}"));
    }
    if let Some(description) = description {
        line.push_str(&format!(" - {description}"));
    }
    line
}

fn collect_codex_apps(pages: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    pages
        .iter()
        .filter_map(|page| page.get("data").and_then(serde_json::Value::as_array))
        .flat_map(|items| items.iter())
        .collect()
}

fn format_codex_plugin_line(plugin: &serde_json::Value) -> String {
    let id = json_string(plugin, "id").unwrap_or("(unknown plugin)");
    let name = json_string(plugin, "name").unwrap_or(id);
    let source = plugin
        .get("source")
        .and_then(|source| json_string(source, "type"))
        .or_else(|| json_string(plugin, "source"))
        .unwrap_or("unknown");
    let installed = yes_no(json_bool(plugin, "installed"));
    let enabled = yes_no(json_bool(plugin, "enabled"));
    let version = json_string(plugin, "localVersion");
    let availability = plugin
        .get("availability")
        .and_then(|availability| json_string(availability, "type"))
        .or_else(|| json_string(plugin, "availability"))
        .unwrap_or("unknown");

    let mut line = format!(
        "- {name} ({id}): installed={installed}, enabled={enabled}, source={source}, availability={availability}"
    );
    if let Some(version) = version {
        line.push_str(&format!(", version={version}"));
    }
    line
}

fn build_codex_runtime_capabilities_report_result(
    capabilities: &serde_json::Value,
    collaboration_modes: &serde_json::Value,
    app_pages: &[serde_json::Value],
    plugins: &serde_json::Value,
) -> AgentCodexRuntimeInventoryResult {
    let mut lines = vec!["Model provider capabilities".to_string()];
    lines.push(format!(
        "- Namespace tools: {}",
        yes_no(json_bool(capabilities, "namespaceTools"))
    ));
    lines.push(format!(
        "- Image generation: {}",
        yes_no(json_bool(capabilities, "imageGeneration"))
    ));
    lines.push(format!(
        "- Web search: {}",
        yes_no(json_bool(capabilities, "webSearch"))
    ));

    let modes = collaboration_modes
        .get("data")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    lines.push(String::new());
    lines.push(format!("Collaboration modes: {}", modes.len()));
    if modes.is_empty() {
        lines.push("No collaboration modes returned.".to_string());
    } else {
        for mode in modes.iter().take(12) {
            lines.push(format_codex_collaboration_mode(mode));
        }
        if modes.len() > 12 {
            lines.push(format!("- {} more mode(s)", modes.len() - 12));
        }
    }

    let apps = collect_codex_apps(app_pages);
    let enabled_apps = apps
        .iter()
        .filter(|app| json_bool(app, "isEnabled"))
        .count();
    let accessible_apps = apps
        .iter()
        .filter(|app| json_bool(app, "isAccessible"))
        .count();
    lines.push(String::new());
    lines.push(format!(
        "Apps/connectors: {} total, {enabled_apps} enabled, {accessible_apps} accessible",
        apps.len()
    ));
    if app_pages.len() >= 10
        && app_pages
            .last()
            .and_then(|page| page.get("nextCursor"))
            .and_then(serde_json::Value::as_str)
            .is_some()
    {
        lines.push("Apps result truncated after 10 pages.".to_string());
    }
    if apps.is_empty() {
        lines.push("No apps returned.".to_string());
    } else {
        for app in apps.iter().take(12) {
            lines.push(format_codex_app_line(app));
        }
        if apps.len() > 12 {
            lines.push(format!("- {} more app(s)", apps.len() - 12));
        }
    }

    let marketplaces = plugins
        .get("marketplaces")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let plugin_entries: Vec<_> = marketplaces
        .iter()
        .filter_map(|marketplace| {
            marketplace
                .get("plugins")
                .and_then(serde_json::Value::as_array)
        })
        .flat_map(|items| items.iter())
        .collect();
    let installed_plugins = plugin_entries
        .iter()
        .filter(|plugin| json_bool(plugin, "installed"))
        .count();
    let enabled_plugins = plugin_entries
        .iter()
        .filter(|plugin| json_bool(plugin, "enabled"))
        .count();
    let load_errors = plugins
        .get("marketplaceLoadErrors")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let featured = plugins
        .get("featuredPluginIds")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    lines.push(String::new());
    lines.push(format!(
        "Plugins: {} total, {installed_plugins} installed, {enabled_plugins} enabled, {} marketplace(s), {featured} featured, {load_errors} load error(s)",
        plugin_entries.len(),
        marketplaces.len()
    ));
    for marketplace in marketplaces.iter().take(8) {
        let name = json_string(marketplace, "name").unwrap_or("(unknown marketplace)");
        let display = marketplace
            .get("interface")
            .and_then(|interface| json_string(interface, "displayName"));
        let plugin_count = marketplace
            .get("plugins")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        match display {
            Some(display) => lines.push(format!(
                "- Marketplace {name}: {display}, {plugin_count} plugin(s)"
            )),
            None => lines.push(format!("- Marketplace {name}: {plugin_count} plugin(s)")),
        }
    }
    if plugin_entries.is_empty() {
        lines.push("No plugins returned from local/workspace marketplaces.".to_string());
    } else {
        for plugin in plugin_entries.iter().take(12) {
            lines.push(format_codex_plugin_line(plugin));
        }
        if plugin_entries.len() > 12 {
            lines.push(format!("- {} more plugin(s)", plugin_entries.len() - 12));
        }
    }
    if let Some(errors) = plugins
        .get("marketplaceLoadErrors")
        .and_then(serde_json::Value::as_array)
    {
        for error in errors.iter().take(5) {
            let path = json_string(error, "marketplacePath").unwrap_or("(unknown marketplace)");
            let message = json_string(error, "message").unwrap_or("unknown error");
            lines.push(format!(
                "- Marketplace load error: {path}: {}",
                truncate_middle(message, 120)
            ));
        }
    }

    AgentCodexRuntimeInventoryResult {
        title: "Codex runtime capabilities".to_string(),
        detail: lines.join("\n"),
    }
}

fn codex_voice_list(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn codex_voice_list_line(label: &str, values: &[String]) -> String {
    if values.is_empty() {
        format!("{label}: none returned")
    } else {
        format!("{label}: {}", values.join(", "))
    }
}

fn build_codex_realtime_voices_report_result(
    response: &serde_json::Value,
) -> AgentCodexRuntimeInventoryResult {
    let voices = response.get("voices").unwrap_or(response);
    let default_v1 = voices
        .get("defaultV1")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let default_v2 = voices
        .get("defaultV2")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let v1 = codex_voice_list(voices, "v1");
    let v2 = codex_voice_list(voices, "v2");
    let lines = [
        "Codex realtime voices are runtime-provided and experimental.".to_string(),
        format!("Default v1: {default_v1}"),
        format!("Default v2: {default_v2}"),
        codex_voice_list_line("v1 voices", &v1),
        codex_voice_list_line("v2 voices", &v2),
        format!(
            "Transport methods: {}, {}, {}, {}",
            METHOD_THREAD_REALTIME_START,
            METHOD_THREAD_REALTIME_APPEND_AUDIO,
            METHOD_THREAD_REALTIME_APPEND_TEXT,
            METHOD_THREAD_REALTIME_STOP
        ),
        "Desktop audio capture and WebRTC session lifecycle are not connected yet.".to_string(),
    ];

    AgentCodexRuntimeInventoryResult {
        title: "Codex realtime voices".to_string(),
        detail: lines.join("\n"),
    }
}

fn trim_for_report(value: &str, max_chars: usize) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

fn codex_thread_title(thread: &serde_json::Value) -> String {
    thread
        .get("name")
        .and_then(serde_json::Value::as_str)
        .or_else(|| thread.get("preview").and_then(serde_json::Value::as_str))
        .map(|value| trim_for_report(value, 96))
        .filter(|value| !value.is_empty())
        .or_else(|| {
            thread
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "(untitled thread)".to_string())
}

fn codex_thread_string_field(thread: &serde_json::Value, key: &str) -> String {
    thread
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".to_string())
}

fn codex_thread_line(thread: &serde_json::Value, snippet: Option<&str>) -> String {
    let title = codex_thread_title(thread);
    let status = codex_thread_string_field(thread, "status");
    let source = codex_thread_string_field(thread, "source");
    let cwd = codex_thread_string_field(thread, "cwd");
    let id = codex_thread_string_field(thread, "id");
    let updated_at = thread
        .get("updatedAt")
        .and_then(serde_json::Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let mut line = format!("- {title} [{status}, {source}, updated {updated_at}]\n  {id}\n  {cwd}");
    if let Some(snippet) = snippet
        .map(|value| trim_for_report(value, 160))
        .filter(|value| !value.is_empty())
    {
        line.push_str(&format!("\n  {snippet}"));
    }
    line
}

fn collect_codex_thread_history_entries<'a>(
    pages: &'a [serde_json::Value],
    worktree_path: &str,
    is_search: bool,
) -> Vec<(&'a serde_json::Value, Option<&'a str>)> {
    let mut entries = Vec::new();
    for page in pages {
        let Some(data) = page.get("data").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for item in data {
            let (thread, snippet) = if is_search {
                (
                    item.get("thread").unwrap_or(item),
                    item.get("snippet").and_then(serde_json::Value::as_str),
                )
            } else {
                (item, None)
            };
            if thread
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|cwd| cwd == worktree_path)
            {
                entries.push((thread, snippet));
            }
            if entries.len() >= 20 {
                return entries;
            }
        }
    }
    entries
}

fn build_codex_thread_history_report_result(
    pages: &[serde_json::Value],
    worktree_path: &str,
    query: &str,
) -> AgentCodexRuntimeInventoryResult {
    let is_search = !query.trim().is_empty();
    let entries = collect_codex_thread_history_entries(pages, worktree_path, is_search);
    let mut lines = vec![if is_search {
        format!(
            "Codex runtime thread search: {} result(s) for \"{}\" in {worktree_path}",
            entries.len(),
            query.trim()
        )
    } else {
        format!(
            "Codex runtime threads: {} latest thread(s) in {worktree_path}",
            entries.len()
        )
    }];

    if entries.is_empty() {
        lines.push("No Codex runtime threads returned for this worktree.".to_string());
    } else {
        for (thread, snippet) in entries {
            lines.push(codex_thread_line(thread, snippet));
        }
    }

    AgentCodexRuntimeInventoryResult {
        title: if is_search {
            "Codex thread search".to_string()
        } else {
            "Codex thread history".to_string()
        },
        detail: lines.join("\n"),
    }
}

fn codex_user_input_text(input: &serde_json::Value) -> Option<String> {
    match input.get("type").and_then(serde_json::Value::as_str) {
        Some("text") => input
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(|value| trim_for_report(value, 220)),
        Some("image") => Some("[image]".to_string()),
        _ => None,
    }
}

fn codex_thread_item_summary(item: &serde_json::Value) -> Option<String> {
    let item_type = item.get("type").and_then(serde_json::Value::as_str)?;
    match item_type {
        "userMessage" => {
            let text = item
                .get("content")
                .and_then(serde_json::Value::as_array)
                .map(|content| {
                    content
                        .iter()
                        .filter_map(codex_user_input_text)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            Some(format!(
                "User: {}",
                if text.trim().is_empty() {
                    "(empty message)".to_string()
                } else {
                    text
                }
            ))
        }
        "agentMessage" => item
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(|text| format!("Agent: {}", trim_for_report(text, 260))),
        "reasoning" => {
            let summary_count = item
                .get("summary")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let content_count = item
                .get("content")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            Some(format!(
                "Reasoning: {summary_count} summary fragment(s), {content_count} content fragment(s)"
            ))
        }
        "commandExecution" => {
            let command = item
                .get("command")
                .and_then(serde_json::Value::as_str)
                .map(|value| trim_for_report(value, 160))
                .unwrap_or_else(|| "(unknown command)".to_string());
            let status = item
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            Some(format!("Tool command [{status}]: {command}"))
        }
        "fileChange" => {
            let status = item
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let change_count = item
                .get("changes")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            Some(format!(
                "Tool file change [{status}]: {change_count} change(s)"
            ))
        }
        "mcpToolCall" | "dynamicToolCall" => {
            let status = item
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let tool = item
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .or_else(|| item.get("name").and_then(serde_json::Value::as_str))
                .unwrap_or(item_type);
            Some(format!("Tool call [{status}]: {tool}"))
        }
        "plan" => item
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(|text| format!("Plan: {}", trim_for_report(text, 220))),
        _ => Some(format!("{item_type}: persisted item")),
    }
}

fn build_codex_thread_transcript_report_result(
    response: &serde_json::Value,
    turns: Vec<serde_json::Value>,
) -> AgentCodexRuntimeInventoryResult {
    let thread = response.get("thread").unwrap_or(response);
    let thread_id = codex_thread_string_field(thread, "id");
    let title = codex_thread_title(thread);
    let item_count = turns
        .iter()
        .filter_map(|turn| turn.get("items").and_then(serde_json::Value::as_array))
        .map(Vec::len)
        .sum::<usize>();
    let mut lines = vec![
        format!("Thread: {title}"),
        format!("ID: {thread_id}"),
        format!("Turns: {}, persisted item(s): {item_count}", turns.len()),
    ];

    if turns.is_empty() {
        lines.push("No persisted turns returned by Codex runtime.".to_string());
    } else {
        for (turn_index, turn) in turns.iter().take(12).enumerate() {
            let status = turn
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let turn_id = turn
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            lines.push(String::new());
            lines.push(format!("Turn {} [{status}] {turn_id}", turn_index + 1));
            if let Some(items) = turn.get("items").and_then(serde_json::Value::as_array) {
                for summary in items.iter().filter_map(codex_thread_item_summary).take(24) {
                    lines.push(format!("- {summary}"));
                }
                if items.len() > 24 {
                    lines.push(format!("- ... {} more item(s)", items.len() - 24));
                }
            } else {
                lines.push("- Items were not loaded.".to_string());
            }
        }
        if turns.len() > 12 {
            lines.push(String::new());
            lines.push(format!("... {} more turn(s)", turns.len() - 12));
        }
    }

    AgentCodexRuntimeInventoryResult {
        title: "Codex thread transcript".to_string(),
        detail: lines.join("\n"),
    }
}

fn build_codex_hooks_report_result(hooks: &serde_json::Value) -> AgentCodexRuntimeInventoryResult {
    let entries: Vec<_> = hooks
        .get("data")
        .and_then(serde_json::Value::as_array)
        .map(|entries| entries.iter().collect())
        .unwrap_or_default();
    let total_hooks = entries
        .iter()
        .filter_map(|entry| entry.get("hooks").and_then(serde_json::Value::as_array))
        .map(Vec::len)
        .sum::<usize>();
    let enabled_hooks = entries
        .iter()
        .filter_map(|entry| entry.get("hooks").and_then(serde_json::Value::as_array))
        .flat_map(|hooks| hooks.iter())
        .filter(|hook| {
            hook.get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
        })
        .count();
    let warning_count = entries
        .iter()
        .filter_map(|entry| entry.get("warnings").and_then(serde_json::Value::as_array))
        .map(Vec::len)
        .sum::<usize>();
    let error_count = entries
        .iter()
        .filter_map(|entry| entry.get("errors").and_then(serde_json::Value::as_array))
        .map(Vec::len)
        .sum::<usize>();

    let mut lines = vec![format!(
        "Hooks: {total_hooks} total, {enabled_hooks} enabled, {} disabled",
        total_hooks.saturating_sub(enabled_hooks)
    )];
    if warning_count > 0 || error_count > 0 {
        lines.push(format!(
            "Diagnostics: {warning_count} warning(s), {error_count} error(s)"
        ));
    }

    for entry in &entries {
        lines.push(String::new());
        let cwd = entry
            .get("cwd")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("(unknown cwd)");
        lines.push(cwd.to_string());

        if let Some(hooks) = entry.get("hooks").and_then(serde_json::Value::as_array) {
            if hooks.is_empty() {
                lines.push("- No hooks configured.".to_string());
            } else {
                for hook in hooks {
                    lines.push(format_codex_hook_line(hook));
                }
            }
        } else {
            lines.push("- No hooks returned.".to_string());
        }

        if let Some(warnings) = entry.get("warnings").and_then(serde_json::Value::as_array) {
            for warning in warnings.iter().filter_map(serde_json::Value::as_str) {
                lines.push(format!("- Warning: {warning}"));
            }
        }
        if let Some(errors) = entry.get("errors").and_then(serde_json::Value::as_array) {
            for error in errors {
                let path = error
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("(unknown path)");
                let message = error
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown error");
                lines.push(format!("- Error: {path}: {message}"));
            }
        }
    }

    if entries.is_empty() {
        lines.push("No hook inventory returned.".to_string());
    }

    AgentCodexRuntimeInventoryResult {
        title: "Codex hooks".to_string(),
        detail: lines.join("\n"),
    }
}

fn format_codex_hook_line(hook: &serde_json::Value) -> String {
    let event = hook
        .get("eventName")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let handler = hook
        .get("handlerType")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let source = hook
        .get("source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let trust = hook
        .get("trustStatus")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let enabled = hook
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let matcher = hook
        .get("matcher")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let command = hook
        .get("command")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let key = hook
        .get("key")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("(unknown hook)");
    let status = hook
        .get("statusMessage")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());

    let mut line = format!(
        "- {event} {handler} [{source}, {trust}, {}]",
        if enabled { "enabled" } else { "disabled" }
    );
    if let Some(matcher) = matcher {
        line.push_str(&format!(" matcher={matcher}"));
    }
    if let Some(command) = command {
        line.push_str(&format!(" command={}", truncate_middle(command, 120)));
    } else {
        line.push_str(&format!(" key={key}"));
    }
    if let Some(status) = status {
        line.push_str(&format!(" status={status}"));
    }
    line
}

fn build_codex_permission_profiles_report_result(
    pages: &[serde_json::Value],
) -> AgentCodexRuntimeInventoryResult {
    let profiles: Vec<_> = pages
        .iter()
        .filter_map(|page| page.get("data").and_then(serde_json::Value::as_array))
        .flat_map(|items| items.iter())
        .collect();
    let mut lines = vec![format!("Permission profiles: {}", profiles.len())];
    if pages.len() >= 20
        && pages
            .last()
            .and_then(|page| page.get("nextCursor"))
            .and_then(serde_json::Value::as_str)
            .is_some()
    {
        lines.push("Result truncated after 20 pages.".to_string());
    }
    if profiles.is_empty() {
        lines.push("No permission profiles returned.".to_string());
    } else {
        for profile in profiles {
            lines.push(format_codex_permission_profile_line(profile));
        }
    }

    AgentCodexRuntimeInventoryResult {
        title: "Codex permission profiles".to_string(),
        detail: lines.join("\n"),
    }
}

fn collect_codex_permission_profiles(
    pages: &[serde_json::Value],
) -> Vec<AgentCodexPermissionProfile> {
    pages
        .iter()
        .filter_map(|page| page.get("data").and_then(serde_json::Value::as_array))
        .flat_map(|items| items.iter())
        .filter_map(|profile| {
            let id = profile
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let description = profile
                .get("description")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            Some(AgentCodexPermissionProfile {
                id: id.to_string(),
                description,
            })
        })
        .collect()
}

fn format_codex_permission_profile_line(profile: &serde_json::Value) -> String {
    let id = profile
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("(unknown profile)");
    let description = profile
        .get("description")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    match description {
        Some(description) => format!("- {id}: {}", truncate_middle(description, 160)),
        None => format!("- {id}"),
    }
}

fn build_codex_mcp_status_report_result(
    pages: &[serde_json::Value],
) -> AgentCodexRuntimeInventoryResult {
    let servers: Vec<_> = pages
        .iter()
        .filter_map(|page| page.get("data").and_then(serde_json::Value::as_array))
        .flat_map(|items| items.iter())
        .collect();
    let total_tools = servers
        .iter()
        .map(|server| {
            server
                .get("tools")
                .and_then(serde_json::Value::as_object)
                .map(serde_json::Map::len)
                .unwrap_or(0)
        })
        .sum::<usize>();
    let total_resources = servers
        .iter()
        .map(|server| {
            server
                .get("resources")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        })
        .sum::<usize>();
    let total_templates = servers
        .iter()
        .map(|server| {
            server
                .get("resourceTemplates")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        })
        .sum::<usize>();

    let mut lines = vec![format!(
        "MCP servers: {}, tools: {total_tools}, resources: {total_resources}, templates: {total_templates}",
        servers.len()
    )];
    if pages.len() >= 20
        && pages
            .last()
            .and_then(|page| page.get("nextCursor"))
            .and_then(serde_json::Value::as_str)
            .is_some()
    {
        lines.push("Result truncated after 20 pages.".to_string());
    }

    if servers.is_empty() {
        lines.push("No MCP server status returned.".to_string());
    } else {
        for server in servers {
            lines.push(format_codex_mcp_server_line(server));
            lines.extend(format_codex_mcp_tool_lines(server, 5));
        }
    }

    AgentCodexRuntimeInventoryResult {
        title: "Codex MCP status".to_string(),
        detail: lines.join("\n"),
    }
}

fn format_codex_mcp_server_line(server: &serde_json::Value) -> String {
    let name = server
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("(unknown server)");
    let auth = server
        .get("authStatus")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let title = server
        .get("serverInfo")
        .and_then(|info| info.get("title"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            server
                .get("serverInfo")
                .and_then(|info| info.get("name"))
                .and_then(serde_json::Value::as_str)
        });
    let version = server
        .get("serverInfo")
        .and_then(|info| info.get("version"))
        .and_then(serde_json::Value::as_str);
    let tools = server
        .get("tools")
        .and_then(serde_json::Value::as_object)
        .map(serde_json::Map::len)
        .unwrap_or(0);
    let resources = server
        .get("resources")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let templates = server
        .get("resourceTemplates")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);

    let mut line = format!(
        "- {name}: auth={auth}, tools={tools}, resources={resources}, templates={templates}"
    );
    if let Some(title) = title.filter(|value| !value.trim().is_empty()) {
        line.push_str(&format!(", title={title}"));
    }
    if let Some(version) = version.filter(|value| !value.trim().is_empty()) {
        line.push_str(&format!(", version={version}"));
    }
    line
}

fn format_codex_mcp_tool_lines(server: &serde_json::Value, limit: usize) -> Vec<String> {
    let Some(tools) = server.get("tools").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    let mut entries: Vec<_> = tools.iter().collect();
    entries.sort_by_key(|(left, _)| *left);

    let mut lines = Vec::new();
    for (name, tool) in entries.iter().take(limit) {
        let title = tool
            .get("title")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                tool.get("description")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
            });
        if let Some(title) = title {
            lines.push(format!("  - {name}: {}", truncate_middle(title, 100)));
        } else {
            lines.push(format!("  - {name}"));
        }
    }
    if entries.len() > limit {
        lines.push(format!("  - ... {} more tool(s)", entries.len() - limit));
    }
    lines
}

fn summarize_codex_account(response: &serde_json::Value) -> Vec<String> {
    let mut lines = Vec::new();
    let requires_openai_auth = response
        .get("requiresOpenaiAuth")
        .and_then(serde_json::Value::as_bool)
        .map(|value| if value { "yes" } else { "no" })
        .unwrap_or("unknown");
    lines.push(format!("Requires OpenAI auth: {requires_openai_auth}"));

    let Some(account) = response.get("account").filter(|value| !value.is_null()) else {
        lines.push("Signed in account: none".to_string());
        return lines;
    };

    let account_type = account
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    lines.push(format!("Signed in account: {account_type}"));
    push_string_line(
        &mut lines,
        "Email",
        account.get("email").and_then(serde_json::Value::as_str),
    );
    push_string_line(
        &mut lines,
        "Plan",
        account.get("planType").and_then(serde_json::Value::as_str),
    );
    lines
}

fn summarize_codex_token_usage(usage: &serde_json::Value) -> Vec<String> {
    let summary = usage.get("summary").unwrap_or(&serde_json::Value::Null);
    let mut lines = Vec::new();
    push_u64_line(
        &mut lines,
        "Lifetime tokens",
        summary
            .get("lifetimeTokens")
            .and_then(serde_json::Value::as_u64),
    );
    push_u64_line(
        &mut lines,
        "Peak daily tokens",
        summary
            .get("peakDailyTokens")
            .and_then(serde_json::Value::as_u64),
    );
    push_u64_line(
        &mut lines,
        "Current streak days",
        summary
            .get("currentStreakDays")
            .and_then(serde_json::Value::as_u64),
    );
    push_u64_line(
        &mut lines,
        "Longest streak days",
        summary
            .get("longestStreakDays")
            .and_then(serde_json::Value::as_u64),
    );
    push_u64_line(
        &mut lines,
        "Longest running turn seconds",
        summary
            .get("longestRunningTurnSec")
            .and_then(serde_json::Value::as_u64),
    );

    if let Some(buckets) = usage
        .get("dailyUsageBuckets")
        .and_then(serde_json::Value::as_array)
    {
        if let Some(latest) = buckets.last() {
            let date = latest
                .get("startDate")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown date");
            let tokens = latest
                .get("tokens")
                .and_then(serde_json::Value::as_u64)
                .map(format_count)
                .unwrap_or_else(|| "unknown".to_string());
            lines.push(format!(
                "Latest daily bucket: {date} ({tokens} tokens, {} buckets)",
                buckets.len()
            ));
        } else {
            lines.push("Daily buckets: none".to_string());
        }
    }

    if lines.is_empty() {
        lines.push("No token usage summary returned.".to_string());
    }
    lines
}

fn summarize_codex_rate_limits(rate_limits: &serde_json::Value) -> Vec<String> {
    let mut snapshots = Vec::new();
    if let Some(by_id) = rate_limits
        .get("rateLimitsByLimitId")
        .and_then(serde_json::Value::as_object)
    {
        let mut entries: Vec<_> = by_id.iter().collect();
        entries.sort_by_key(|(left, _)| *left);
        for (id, snapshot) in entries {
            snapshots.push((id.as_str(), snapshot));
        }
    }
    if snapshots.is_empty() {
        if let Some(snapshot) = rate_limits.get("rateLimits") {
            snapshots.push(("default", snapshot));
        }
    }

    let mut lines = Vec::new();
    for (index, (fallback_id, snapshot)) in snapshots.into_iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(summarize_codex_rate_limit_snapshot(fallback_id, snapshot));
    }

    if lines.is_empty() {
        lines.push("No rate-limit snapshot returned.".to_string());
    }
    lines
}

fn summarize_codex_rate_limit_snapshot(
    fallback_id: &str,
    snapshot: &serde_json::Value,
) -> Vec<String> {
    let label = snapshot
        .get("limitName")
        .and_then(serde_json::Value::as_str)
        .or_else(|| snapshot.get("limitId").and_then(serde_json::Value::as_str))
        .unwrap_or(fallback_id);
    let mut lines = vec![format!("- {label}")];

    push_string_line(
        &mut lines,
        "  Plan",
        snapshot.get("planType").and_then(serde_json::Value::as_str),
    );
    if let Some(reached) = snapshot
        .get("rateLimitReachedType")
        .and_then(serde_json::Value::as_str)
    {
        lines.push(format!("  Reached: {reached}"));
    }
    if let Some(primary) = snapshot.get("primary") {
        push_rate_limit_window(&mut lines, "  Primary", primary);
    }
    if let Some(secondary) = snapshot.get("secondary") {
        push_rate_limit_window(&mut lines, "  Secondary", secondary);
    }
    if let Some(credits) = snapshot.get("credits") {
        push_credits_snapshot(&mut lines, credits);
    }
    if let Some(individual) = snapshot.get("individualLimit") {
        push_individual_limit_snapshot(&mut lines, individual);
    }
    lines
}

fn push_rate_limit_window(lines: &mut Vec<String>, label: &str, window: &serde_json::Value) {
    if window.is_null() {
        return;
    }
    let Some(used_percent) = window
        .get("usedPercent")
        .and_then(serde_json::Value::as_u64)
    else {
        return;
    };
    let mut detail = format!("{label}: {used_percent}% used");
    if let Some(duration) = window
        .get("windowDurationMins")
        .and_then(serde_json::Value::as_u64)
    {
        detail.push_str(&format!(", window {duration} min"));
    }
    if let Some(resets_at) = window.get("resetsAt").and_then(serde_json::Value::as_u64) {
        detail.push_str(&format!(", resets at unix {resets_at}"));
    }
    lines.push(detail);
}

fn push_credits_snapshot(lines: &mut Vec<String>, credits: &serde_json::Value) {
    if credits.is_null() {
        return;
    }
    let has_credits = credits
        .get("hasCredits")
        .and_then(serde_json::Value::as_bool)
        .map(|value| if value { "yes" } else { "no" });
    let unlimited = credits
        .get("unlimited")
        .and_then(serde_json::Value::as_bool)
        .map(|value| if value { "yes" } else { "no" });
    let balance = credits
        .get("balance")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unavailable");
    lines.push(format!(
        "  Credits: has={}, unlimited={}, balance={balance}",
        has_credits.unwrap_or("unknown"),
        unlimited.unwrap_or("unknown")
    ));
}

fn push_individual_limit_snapshot(lines: &mut Vec<String>, limit: &serde_json::Value) {
    if limit.is_null() {
        return;
    }
    let used = limit
        .get("used")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let max = limit
        .get("limit")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let remaining = limit
        .get("remainingPercent")
        .and_then(serde_json::Value::as_u64)
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "unknown".to_string());
    let resets = limit
        .get("resetsAt")
        .and_then(serde_json::Value::as_u64)
        .map(|value| format!(", resets at unix {value}"))
        .unwrap_or_default();
    lines.push(format!(
        "  Individual limit: {used} / {max}, {remaining} remaining{resets}"
    ));
}

fn push_u64_line(lines: &mut Vec<String>, label: &str, value: Option<u64>) {
    if let Some(value) = value {
        lines.push(format!("{label}: {}", format_count(value)));
    }
}

fn push_string_line(lines: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        lines.push(format!("{label}: {value}"));
    }
}

fn truncate_middle(value: &str, max_chars: usize) -> String {
    let total = value.chars().count();
    if total <= max_chars || max_chars <= 3 {
        return value.to_string();
    }
    let keep = max_chars - 3;
    let head = keep / 2;
    let tail = keep - head;
    let prefix: String = value.chars().take(head).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(tail)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    formatted.chars().rev().collect()
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

    if backend_id == crate::infrastructure::agent_session::runtime::CODEX_BACKEND_ID {
        let backend = registry
            .get(&backend_id)
            .ok_or_else(|| format!("Agent backend not found: {backend_id}"))?;
        backend
            .start_session(SessionConfig {
                chat_session_id,
                cwd,
                permission_mode: Some(validated_permission_mode_str),
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
            permission_mode: "edit".to_string(),
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session,
        }
    }

    #[test]
    fn parses_codex_review_targets() {
        assert_eq!(
            parse_codex_review_target(None, None).expect("target"),
            AgentReviewTarget::UncommittedChanges
        );
        assert_eq!(
            parse_codex_review_target(Some("baseBranch"), Some(" main ")).expect("target"),
            AgentReviewTarget::BaseBranch {
                branch: "main".to_string()
            }
        );
        assert_eq!(
            parse_codex_review_target(Some("commit"), Some("abc123")).expect("target"),
            AgentReviewTarget::Commit {
                sha: "abc123".to_string()
            }
        );
        assert_eq!(
            parse_codex_review_target(Some("custom"), Some("Review security")).expect("target"),
            AgentReviewTarget::Custom {
                instructions: "Review security".to_string()
            }
        );
        assert!(parse_codex_review_target(Some("baseBranch"), Some(" ")).is_err());
        assert!(parse_codex_review_target(Some("unknown"), None).is_err());
    }

    #[test]
    fn start_agent_session_guard_rejects_workflow_step_session_before_runtime_start() {
        let session = session_for_start_guard(true);
        let handles = crate::infrastructure::agent_session::runtime::AgentProcessMap::new();

        let result = reject_explicit_start_for_workflow_step_session(&session, "/repo");

        assert_eq!(result.unwrap_err(), session_target_rejected());
        assert!(handles.is_empty());
    }

    #[test]
    fn start_agent_session_guard_allows_regular_session_in_matching_worktree() {
        let session = session_for_start_guard(false);

        assert!(reject_explicit_start_for_workflow_step_session(&session, "/repo").is_ok());
    }

    #[test]
    fn codex_account_status_report_summarizes_usage_and_limits() {
        let report = build_codex_account_status_result(
            &serde_json::json!({
                "requiresOpenaiAuth": false,
                "account": {
                    "type": "chatgpt",
                    "email": "dev@example.com",
                    "planType": "pro"
                }
            }),
            &serde_json::json!({
                "summary": {
                    "lifetimeTokens": 12345,
                    "peakDailyTokens": 2000,
                    "currentStreakDays": 3,
                    "longestStreakDays": 5,
                    "longestRunningTurnSec": 90
                },
                "dailyUsageBuckets": [
                    { "startDate": "2026-06-12", "tokens": 100 },
                    { "startDate": "2026-06-13", "tokens": 250 }
                ]
            }),
            &serde_json::json!({
                "rateLimits": {
                    "limitId": "codex",
                    "limitName": "Codex",
                    "planType": "pro",
                    "primary": {
                        "usedPercent": 50,
                        "windowDurationMins": 300,
                        "resetsAt": 1780000000
                    },
                    "credits": {
                        "hasCredits": true,
                        "unlimited": false,
                        "balance": "$10.00"
                    },
                    "individualLimit": {
                        "used": "$5.00",
                        "limit": "$20.00",
                        "remainingPercent": 75,
                        "resetsAt": 1780000000
                    }
                }
            }),
        );

        assert_eq!(report.title, "Codex account usage");
        assert!(report.detail.contains("Requires OpenAI auth: no"));
        assert!(report.detail.contains("Signed in account: chatgpt"));
        assert!(report.detail.contains("Email: dev@example.com"));
        assert!(report.detail.contains("Plan: pro"));
        assert!(report.detail.contains("Lifetime tokens: 12,345"));
        assert!(report
            .detail
            .contains("Latest daily bucket: 2026-06-13 (250 tokens, 2 buckets)"));
        assert!(report.detail.contains("- Codex"));
        assert!(report.detail.contains("Primary: 50% used"));
        assert!(report.detail.contains("Credits: has=yes"));
        assert!(report.detail.contains("Individual limit: $5.00 / $20.00"));
    }

    #[test]
    fn codex_realtime_voices_report_summarizes_runtime_voice_catalog() {
        let report = build_codex_realtime_voices_report_result(&serde_json::json!({
            "voices": {
                "defaultV1": "alloy",
                "defaultV2": "marin",
                "v1": ["alloy", "echo"],
                "v2": ["marin", "verse"]
            }
        }));

        assert_eq!(report.title, "Codex realtime voices");
        assert!(report.detail.contains("Default v1: alloy"));
        assert!(report.detail.contains("Default v2: marin"));
        assert!(report.detail.contains("v1 voices: alloy, echo"));
        assert!(report.detail.contains("v2 voices: marin, verse"));
        assert!(report.detail.contains("thread/realtime/start"));
        assert!(report.detail.contains("thread/realtime/appendAudio"));
        assert!(report
            .detail
            .contains("Desktop audio capture and WebRTC session lifecycle are not connected yet."));
    }

    #[test]
    fn codex_runtime_config_report_summarizes_layers_and_requirements() {
        let report = build_codex_runtime_config_report_result(
            &serde_json::json!({
                "config": {
                    "model": "gpt-5-codex",
                    "review_model": "gpt-5-codex-high",
                    "model_provider": "openai",
                    "approval_policy": "on-request",
                    "sandbox_mode": "workspace-write",
                    "api_key": "do-not-render"
                },
                "origins": {
                    "model": {
                        "name": { "type": "user", "file": "/home/dev/.codex/config.toml", "profile": null },
                        "version": "1"
                    },
                    "approval_policy": {
                        "name": { "type": "project", "dotCodexFolder": "/repo/.codex" },
                        "version": "2"
                    }
                },
                "layers": [
                    {
                        "name": { "type": "user", "file": "/home/dev/.codex/config.toml", "profile": null },
                        "version": "1",
                        "config": { "api_key": "do-not-render" },
                        "disabledReason": null
                    },
                    {
                        "name": { "type": "project", "dotCodexFolder": "/repo/.codex" },
                        "version": "2",
                        "config": {},
                        "disabledReason": "not trusted for this workspace"
                    }
                ]
            }),
            &serde_json::json!({
                "requirements": {
                    "allowedApprovalPolicies": ["on-request", "never"],
                    "allowedSandboxModes": ["workspace-write"],
                    "allowedPermissionProfiles": { ":workspace": true },
                    "defaultPermissions": ":workspace",
                    "allowManagedHooksOnly": false
                }
            }),
            "/repo",
        );

        assert_eq!(report.title, "Codex runtime config");
        assert!(report.detail.contains("Effective Codex config for /repo"));
        assert!(report.detail.contains("Model: gpt-5-codex"));
        assert!(report.detail.contains("Approval policy: on-request"));
        assert!(report
            .detail
            .contains("- model: user /home/dev/.codex/config.toml v1"));
        assert!(report.detail.contains("- project /repo/.codex v2 disabled"));
        assert!(report.detail.contains("Approval policies: 2 allowed"));
        assert!(report.detail.contains("Default permissions: :workspace"));
        assert!(!report.detail.contains("do-not-render"));
    }

    #[test]
    fn codex_runtime_capabilities_report_summarizes_inventory() {
        let report = build_codex_runtime_capabilities_report_result(
            &serde_json::json!({
                "namespaceTools": true,
                "imageGeneration": false,
                "webSearch": true
            }),
            &serde_json::json!({
                "data": [
                    {
                        "name": "Code",
                        "mode": "agent",
                        "model": "gpt-5-codex",
                        "reasoning_effort": "medium"
                    }
                ]
            }),
            &[serde_json::json!({
                "data": [
                    {
                        "id": "github",
                        "name": "GitHub",
                        "description": "Repository integration",
                        "isEnabled": true,
                        "isAccessible": true,
                        "pluginDisplayNames": ["GitHub Plugin"]
                    }
                ],
                "nextCursor": null
            })],
            &serde_json::json!({
                "marketplaces": [
                    {
                        "name": "local",
                        "path": "/repo/.codex/plugins.json",
                        "interface": { "displayName": "Local marketplace" },
                        "plugins": [
                            {
                                "id": "plugin-github",
                                "name": "GitHub Plugin",
                                "source": { "type": "local" },
                                "installed": true,
                                "enabled": true,
                                "localVersion": "1.0.0",
                                "availability": { "type": "available" }
                            }
                        ]
                    }
                ],
                "marketplaceLoadErrors": [],
                "featuredPluginIds": []
            }),
        );

        assert_eq!(report.title, "Codex runtime capabilities");
        assert!(report.detail.contains("Namespace tools: yes"));
        assert!(report.detail.contains("Image generation: no"));
        assert!(report.detail.contains("Web search: yes"));
        assert!(report.detail.contains("Collaboration modes: 1"));
        assert!(report
            .detail
            .contains("- Code: mode=agent, model=gpt-5-codex, reasoning=medium"));
        assert!(report
            .detail
            .contains("Apps/connectors: 1 total, 1 enabled, 1 accessible"));
        assert!(report
            .detail
            .contains("GitHub (github): enabled=yes, accessible=yes"));
        assert!(report
            .detail
            .contains("Plugins: 1 total, 1 installed, 1 enabled"));
        assert!(report
            .detail
            .contains("GitHub Plugin (plugin-github): installed=yes, enabled=yes"));
    }

    #[test]
    fn codex_goal_result_summarizes_runtime_goal() {
        let goal = parse_codex_goal(Some(&serde_json::json!({
            "objective": "Finish native parity",
            "status": "paused",
            "tokenBudget": 50000,
            "tokensUsed": 1200,
            "timeUsedSeconds": 30
        })))
        .expect("goal");
        let result = codex_goal_result("Codex goal", Some(goal));

        assert_eq!(result.title, "Codex goal");
        assert_eq!(
            result.goal.as_ref().unwrap().objective,
            "Finish native parity"
        );
        assert_eq!(result.goal.as_ref().unwrap().status, "paused");
        assert!(result.detail.contains("Status: paused"));
        assert!(result.detail.contains("Tokens: 1200 / 50000"));
        assert!(result.detail.contains("Finish native parity"));
    }

    #[test]
    fn codex_thread_history_report_filters_and_summarizes_runtime_threads() {
        let report = build_codex_thread_history_report_result(
            &[serde_json::json!({
                "data": [
                    {
                        "snippet": "matched parser bug discussion",
                        "thread": {
                            "id": "thr_keep",
                            "name": "Fix parser bug",
                            "preview": "initial parser request",
                            "cwd": "/repo",
                            "status": "idle",
                            "source": "appServer",
                            "updatedAt": 1780000000
                        }
                    },
                    {
                        "snippet": "other repo",
                        "thread": {
                            "id": "thr_skip",
                            "preview": "other",
                            "cwd": "/other",
                            "status": "idle",
                            "source": "appServer",
                            "updatedAt": 1780000001
                        }
                    }
                ]
            })],
            "/repo",
            "parser",
        );

        assert_eq!(report.title, "Codex thread search");
        assert!(report.detail.contains("1 result(s)"));
        assert!(report.detail.contains("Fix parser bug"));
        assert!(report.detail.contains("thr_keep"));
        assert!(report.detail.contains("matched parser bug discussion"));
        assert!(!report.detail.contains("thr_skip"));
    }

    #[test]
    fn codex_thread_transcript_report_summarizes_persisted_turn_items() {
        let report = build_codex_thread_transcript_report_result(
            &serde_json::json!({
                "thread": {
                    "id": "thr_123",
                    "name": "Fix parser bug",
                    "preview": "Fix parser",
                    "cwd": "/repo",
                    "status": "idle",
                    "source": "appServer",
                    "updatedAt": 1780000000
                }
            }),
            vec![serde_json::json!({
                    "id": "turn_1",
                    "status": "completed",
                    "items": [
                        {
                            "id": "item_user",
                            "type": "userMessage",
                            "content": [{ "type": "text", "text": "Fix the parser bug" }]
                        },
                        {
                            "id": "item_agent",
                            "type": "agentMessage",
                            "text": "The parser bug is fixed."
                        },
                        {
                            "id": "item_cmd",
                            "type": "commandExecution",
                            "status": "completed",
                            "command": "cargo test"
                        }
                    ]
            })],
        );

        assert_eq!(report.title, "Codex thread transcript");
        assert!(report.detail.contains("Thread: Fix parser bug"));
        assert!(report.detail.contains("Turns: 1, persisted item(s): 3"));
        assert!(report.detail.contains("User: Fix the parser bug"));
        assert!(report.detail.contains("Agent: The parser bug is fixed."));
        assert!(report
            .detail
            .contains("Tool command [completed]: cargo test"));
    }

    #[test]
    fn saved_codex_thread_id_for_goal_requires_codex_thread() {
        let mut session = session_for_start_guard(false);
        session.backend_id = Some(CODEX_BACKEND_ID.to_string());
        session.agent_session_id = Some(" thr_123 ".to_string());

        assert_eq!(saved_codex_thread_id_for_goal(&session).unwrap(), "thr_123");

        session.agent_session_id = Some(" ".to_string());
        assert!(saved_codex_thread_id_for_goal(&session)
            .unwrap_err()
            .contains("thread is not ready"));
    }

    #[test]
    fn codex_hooks_report_summarizes_runtime_inventory() {
        let report = build_codex_hooks_report_result(&serde_json::json!({
            "data": [{
                "cwd": "/repo",
                "hooks": [
                    {
                        "key": "project-pre",
                        "eventName": "preToolUse",
                        "handlerType": "command",
                        "matcher": "Edit",
                        "command": "cargo fmt --check && cargo clippy -- -D warnings",
                        "source": "project",
                        "trustStatus": "trusted",
                        "enabled": true
                    },
                    {
                        "key": "user-stop",
                        "eventName": "stop",
                        "handlerType": "prompt",
                        "source": "user",
                        "trustStatus": "untrusted",
                        "enabled": false,
                        "statusMessage": "disabled by config"
                    }
                ],
                "warnings": ["hook warning"],
                "errors": [{ "path": "/repo/.codex/hooks.toml", "message": "bad hook" }]
            }]
        }));

        assert_eq!(report.title, "Codex hooks");
        assert!(report
            .detail
            .contains("Hooks: 2 total, 1 enabled, 1 disabled"));
        assert!(report.detail.contains("/repo"));
        assert!(report
            .detail
            .contains("- preToolUse command [project, trusted, enabled] matcher=Edit"));
        assert!(report.detail.contains("command=cargo fmt --check"));
        assert!(report.detail.contains("status=disabled by config"));
        assert!(report.detail.contains("Warning: hook warning"));
        assert!(report
            .detail
            .contains("Error: /repo/.codex/hooks.toml: bad hook"));
    }

    #[test]
    fn codex_permission_profiles_report_summarizes_runtime_profiles() {
        let mut pages = vec![serde_json::json!({
            "data": [
                { "id": ":workspace", "description": "Workspace write profile" },
                { "id": "readonly", "description": null }
            ],
            "nextCursor": null
        })];
        let report = build_codex_permission_profiles_report_result(&pages);

        assert_eq!(report.title, "Codex permission profiles");
        assert!(report.detail.contains("Permission profiles: 2"));
        assert!(report
            .detail
            .contains("- :workspace: Workspace write profile"));
        assert!(report.detail.contains("- readonly"));

        pages = (0..20)
            .map(|_| serde_json::json!({ "data": [], "nextCursor": "more" }))
            .collect();
        let truncated = build_codex_permission_profiles_report_result(&pages);
        assert!(truncated
            .detail
            .contains("Result truncated after 20 pages."));
    }

    #[test]
    fn codex_mcp_status_report_summarizes_servers_and_tools() {
        let report = build_codex_mcp_status_report_result(&[serde_json::json!({
            "data": [{
                "name": "docs",
                "serverInfo": {
                    "name": "docs",
                    "title": "Docs MCP",
                    "version": "1.2.3",
                    "description": null,
                    "icons": null,
                    "websiteUrl": null
                },
                "tools": {
                    "search": {
                        "name": "search",
                        "description": "Search docs",
                        "inputSchema": {}
                    },
                    "fetch": {
                        "name": "fetch",
                        "title": "Fetch document",
                        "inputSchema": {}
                    }
                },
                "resources": [{ "name": "guide", "uri": "file://guide" }],
                "resourceTemplates": [{ "name": "page", "uriTemplate": "file://{page}" }],
                "authStatus": "oAuth"
            }],
            "nextCursor": null
        })]);

        assert_eq!(report.title, "Codex MCP status");
        assert!(report
            .detail
            .contains("MCP servers: 1, tools: 2, resources: 1, templates: 1"));
        assert!(report
            .detail
            .contains("- docs: auth=oAuth, tools=2, resources=1, templates=1"));
        assert!(report.detail.contains("title=Docs MCP"));
        assert!(report.detail.contains("version=1.2.3"));
        assert!(report.detail.contains("  - fetch: Fetch document"));
        assert!(report.detail.contains("  - search: Search docs"));
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

    fn prompt_history_session(
        id: &str,
        worktree_path: &str,
        state: crate::usecase::agent_session::session::SessionState,
        messages: Vec<crate::usecase::agent_session::session::ChatMessage>,
    ) -> crate::usecase::agent_session::session::ChatSession {
        crate::usecase::agent_session::session::ChatSession {
            id: id.to_string(),
            worktree_path: worktree_path.to_string(),
            messages,
            state,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session: false,
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
    fn copy_response_selects_nth_latest_agent_text() {
        let session = prompt_history_session(
            "current",
            "/repo",
            crate::usecase::agent_session::session::SessionState::Idle,
            vec![
                agent_response("a1", "old response", 10.0),
                human_prompt("h1", "continue", 20.0),
                agent_response("a2", "new response", 30.0),
            ],
        );

        let result = build_agent_copy_response_inner(&session, Some("2"), None).unwrap();

        assert_eq!(result.content, "old response");
        assert_eq!(result.ordinal, 2);
        assert_eq!(result.message_id, "a1");
        assert_eq!(result.suggested_path, "snippets/releash-response-a1.md");
    }

    #[test]
    fn copy_response_extracts_code_blocks_for_picker() {
        let session = prompt_history_session(
            "current",
            "/repo",
            crate::usecase::agent_session::session::SessionState::Idle,
            vec![agent_response(
                "a1",
                "Use this:\n```rust\nfn main() {}\n```\nThen:\n```ts\nconst x = 1;\nconsole.log(x);\n```",
                10.0,
            )],
        );

        let result = build_agent_copy_response_inner(&session, None, None).unwrap();

        assert_eq!(result.code_blocks.len(), 2);
        assert_eq!(result.code_blocks[0].index, 1);
        assert_eq!(result.code_blocks[0].language.as_deref(), Some("rust"));
        assert_eq!(result.code_blocks[0].content, "fn main() {}\n");
        assert_eq!(result.code_blocks[0].line_count, 1);
        assert_eq!(
            result.code_blocks[0].suggested_path,
            "snippets/releash-response-a1-block-1.rs"
        );
        assert_eq!(result.code_blocks[1].language.as_deref(), Some("ts"));
        assert_eq!(
            result.code_blocks[1].content,
            "const x = 1;\nconsole.log(x);\n"
        );
        assert_eq!(result.code_blocks[1].line_count, 2);
        assert_eq!(
            result.code_blocks[1].suggested_path,
            "snippets/releash-response-a1-block-2.ts"
        );
    }

    #[test]
    fn copy_selection_write_writes_relative_file_inside_worktree() {
        let tmp = tempfile::tempdir().unwrap();

        let result = write_agent_copy_selection_to_file_inner(
            &tmp.path().to_string_lossy(),
            "snippets/block.rs",
            "fn main() {}\n",
        )
        .unwrap();

        assert_eq!(result.title, "Copy selection written");
        assert_eq!(result.byte_count, "fn main() {}\n".len());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("snippets").join("block.rs")).unwrap(),
            "fn main() {}\n"
        );
    }

    #[test]
    fn copy_selection_write_rejects_existing_or_outside_target() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("existing.rs"), "old").unwrap();

        let existing = write_agent_copy_selection_to_file_inner(
            &tmp.path().to_string_lossy(),
            "existing.rs",
            "new",
        )
        .unwrap_err();
        let outside = write_agent_copy_selection_to_file_inner(
            &tmp.path().to_string_lossy(),
            "../outside.rs",
            "new",
        )
        .unwrap_err();

        assert!(existing.contains("already exists"));
        assert!(outside.contains("inside the worktree"));
    }

    #[test]
    fn copy_response_skips_excluded_streaming_message() {
        let session = prompt_history_session(
            "current",
            "/repo",
            crate::usecase::agent_session::session::SessionState::Active,
            vec![
                agent_response("finished", "finished response", 10.0),
                agent_response("streaming", "in-progress response", 20.0),
            ],
        );

        let result = build_agent_copy_response_inner(&session, None, Some("streaming")).unwrap();

        assert_eq!(result.content, "finished response");
        assert_eq!(result.message_id, "finished");
    }

    #[test]
    fn copy_response_rejects_invalid_ordinal() {
        let session = prompt_history_session(
            "current",
            "/repo",
            crate::usecase::agent_session::session::SessionState::Idle,
            vec![agent_response("a1", "response", 10.0)],
        );

        let err = build_agent_copy_response_inner(&session, Some("0"), None).unwrap_err();

        assert_eq!(err, "Copy response index must be a positive number.");
    }

    #[test]
    fn export_transcript_builds_readable_session_content() {
        let session = prompt_history_session(
            "current",
            "/repo",
            crate::usecase::agent_session::session::SessionState::Idle,
            vec![
                human_prompt("h1", "Fix parser", 10.0),
                agent_response("a1", "Parser fixed", 20.0),
            ],
        );

        let result = build_agent_export_transcript_inner(&session, None).unwrap();

        assert_eq!(result.title, "Transcript ready");
        assert_eq!(result.path, None);
        assert_eq!(
            result.suggested_path.as_deref(),
            Some("transcripts/releash-agent-current.md")
        );
        assert_eq!(result.message_count, 2);
        assert!(result.content.contains("# Releash Agent Transcript"));
        assert!(result.content.contains("Session: current"));
        assert!(result.content.contains("## Human - h1 - 10"));
        assert!(result.content.contains("Fix parser"));
        assert!(result.content.contains("## Agent - a1 - 20"));
        assert!(result.content.contains("Parser fixed"));
    }

    #[test]
    fn export_transcript_writes_relative_file_inside_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let session = prompt_history_session(
            "current",
            &tmp.path().to_string_lossy(),
            crate::usecase::agent_session::session::SessionState::Idle,
            vec![agent_response("a1", "Export me", 20.0)],
        );

        let result =
            build_agent_export_transcript_inner(&session, Some("transcripts/session.md")).unwrap();

        let path = tmp
            .path()
            .canonicalize()
            .unwrap()
            .join("transcripts/session.md");
        assert_eq!(
            result.path.as_deref(),
            Some(path.to_string_lossy().as_ref())
        );
        assert_eq!(
            result.suggested_path.as_deref(),
            Some("transcripts/session.md")
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), result.content);
        assert!(result.content.contains("Export me"));
    }

    #[test]
    fn export_transcript_suggests_sanitized_relative_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let session = prompt_history_session(
            "Session 42:/Feature",
            &tmp.path().to_string_lossy(),
            crate::usecase::agent_session::session::SessionState::Idle,
            vec![agent_response("a1", "Export me", 20.0)],
        );

        let result = build_agent_export_transcript_inner(&session, None).unwrap();

        assert_eq!(
            result.suggested_path.as_deref(),
            Some("transcripts/releash-agent-session-42-feature.md")
        );
    }

    #[test]
    fn export_transcript_rejects_existing_or_outside_target() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("existing.md"), "keep").unwrap();
        let session = prompt_history_session(
            "current",
            &tmp.path().to_string_lossy(),
            crate::usecase::agent_session::session::SessionState::Idle,
            vec![agent_response("a1", "Export me", 20.0)],
        );

        let existing =
            build_agent_export_transcript_inner(&session, Some("existing.md")).unwrap_err();
        let outside = build_agent_export_transcript_inner(&session, Some("../out.md")).unwrap_err();

        assert!(existing.contains("already exists"));
        assert!(outside.contains("inside the worktree"));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("existing.md")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn prompt_history_search_respects_scope_and_deduplicates() {
        let sessions = vec![
            prompt_history_session(
                "current",
                "/repo",
                crate::usecase::agent_session::session::SessionState::Active,
                vec![human_prompt("m1", "Add tests", 10.0)],
            ),
            prompt_history_session(
                "project-old",
                "/repo",
                crate::usecase::agent_session::session::SessionState::Closed,
                vec![human_prompt("m2", "Review tests", 20.0)],
            ),
            prompt_history_session(
                "other",
                "/other",
                crate::usecase::agent_session::session::SessionState::Closed,
                vec![human_prompt("m3", "Other repo tests", 30.0)],
            ),
            prompt_history_session(
                "archived",
                "/repo",
                crate::usecase::agent_session::session::SessionState::Archived,
                vec![human_prompt("m4", "Archived tests", 40.0)],
            ),
        ];

        let session = search_prompt_history_entries(
            sessions.clone(),
            "current",
            "/repo",
            "tests",
            "session",
            vec!["Draft tests".to_string(), "Add tests".to_string()],
            10,
        );
        assert_eq!(
            session
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Add tests", "Draft tests"]
        );

        let project = search_prompt_history_entries(
            sessions.clone(),
            "current",
            "/repo",
            "tests",
            "project",
            Vec::new(),
            10,
        );
        assert_eq!(
            project
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Review tests", "Add tests"]
        );

        let all = search_prompt_history_entries(
            sessions,
            "current",
            "/repo",
            "tests",
            "all",
            Vec::new(),
            10,
        );
        assert_eq!(
            all.iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Other repo tests", "Review tests", "Add tests"]
        );
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
            permission_mode: "edit".to_string(),
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
            permission_mode: "edit".to_string(),
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
            permission_mode: "edit".to_string(),
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
    engine: tauri::State<'_, Arc<WorkflowEngine>>,
    open_tabs: tauri::State<'_, Arc<crate::usecase::agent_session::session::OpenTabRegistry>>,
    chat_session_id: Option<String>,
    worktree_path: String,
    content: String,
    permission_mode: Option<String>,
    backend_id: Option<String>,
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
            backend_id,
            images,
            mentions,
            editor_context,
        },
    )
    .await?;
    crate::workflow_state_events::emit_after_workflow_step_message(
        &app,
        engine.inner(),
        &response.session,
        handles.inner(),
        open_tabs.inner(),
    )
    .await;
    Ok(response)
}
