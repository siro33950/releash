#![allow(dead_code)]

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{timeout, Duration};

use crate::infrastructure::agent_session::runtime::permission_flags::{
    codex_approval_policy_from_mode, codex_sandbox_mode_from_mode,
};
use crate::infrastructure::agent_session::runtime::{AgentEditorContext, ImageAttachment};
use crate::permission::PermissionMode;

pub(crate) const METHOD_INITIALIZE: &str = "initialize";
pub(crate) const METHOD_INITIALIZED: &str = "initialized";
pub(crate) const METHOD_THREAD_START: &str = "thread/start";
pub(crate) const METHOD_THREAD_FORK: &str = "thread/fork";
pub(crate) const METHOD_THREAD_ARCHIVE: &str = "thread/archive";
pub(crate) const METHOD_THREAD_UNARCHIVE: &str = "thread/unarchive";
pub(crate) const METHOD_THREAD_BACKGROUND_TERMINALS_CLEAN: &str =
    "thread/backgroundTerminals/clean";
pub(crate) const METHOD_THREAD_COMPACT_START: &str = "thread/compact/start";
pub(crate) const METHOD_THREAD_GOAL_CLEAR: &str = "thread/goal/clear";
pub(crate) const METHOD_THREAD_GOAL_GET: &str = "thread/goal/get";
pub(crate) const METHOD_THREAD_GOAL_SET: &str = "thread/goal/set";
pub(crate) const METHOD_THREAD_LIST: &str = "thread/list";
pub(crate) const METHOD_THREAD_READ: &str = "thread/read";
pub(crate) const METHOD_THREAD_RESUME: &str = "thread/resume";
pub(crate) const METHOD_THREAD_SEARCH: &str = "thread/search";
pub(crate) const METHOD_THREAD_NAME_SET: &str = "thread/name/set";
pub(crate) const METHOD_THREAD_SETTINGS_UPDATE: &str = "thread/settings/update";
pub(crate) const METHOD_THREAD_SHELL_COMMAND: &str = "thread/shellCommand";
pub(crate) const METHOD_THREAD_TURNS_ITEMS_LIST: &str = "thread/turns/items/list";
pub(crate) const METHOD_THREAD_TURNS_LIST: &str = "thread/turns/list";
pub(crate) const METHOD_TURN_START: &str = "turn/start";
pub(crate) const METHOD_TURN_STEER: &str = "turn/steer";
pub(crate) const METHOD_TURN_INTERRUPT: &str = "turn/interrupt";
pub(crate) const METHOD_MODEL_LIST: &str = "model/list";
pub(crate) const METHOD_SKILLS_LIST: &str = "skills/list";
pub(crate) const METHOD_HOOKS_LIST: &str = "hooks/list";
pub(crate) const METHOD_MCP_SERVER_STATUS_LIST: &str = "mcpServerStatus/list";
pub(crate) const METHOD_ACCOUNT_READ: &str = "account/read";
pub(crate) const METHOD_ACCOUNT_USAGE_READ: &str = "account/usage/read";
pub(crate) const METHOD_ACCOUNT_RATE_LIMITS_READ: &str = "account/rateLimits/read";
pub(crate) const METHOD_CONFIG_READ: &str = "config/read";
pub(crate) const METHOD_CONFIG_REQUIREMENTS_READ: &str = "configRequirements/read";
pub(crate) const METHOD_FUZZY_FILE_SEARCH: &str = "fuzzyFileSearch";
pub(crate) const METHOD_APP_LIST: &str = "app/list";
pub(crate) const METHOD_COLLABORATION_MODE_LIST: &str = "collaborationMode/list";
pub(crate) const METHOD_MODEL_PROVIDER_CAPABILITIES_READ: &str = "modelProvider/capabilities/read";
pub(crate) const METHOD_PLUGIN_LIST: &str = "plugin/list";
pub(crate) const METHOD_REVIEW_START: &str = "review/start";
pub(crate) const METHOD_PERMISSION_PROFILE_LIST: &str = "permissionProfile/list";
pub(crate) const METHOD_THREAD_REALTIME_APPEND_AUDIO: &str = "thread/realtime/appendAudio";
pub(crate) const METHOD_THREAD_REALTIME_APPEND_TEXT: &str = "thread/realtime/appendText";
pub(crate) const METHOD_THREAD_REALTIME_LIST_VOICES: &str = "thread/realtime/listVoices";
pub(crate) const METHOD_THREAD_REALTIME_START: &str = "thread/realtime/start";
pub(crate) const METHOD_THREAD_REALTIME_STOP: &str = "thread/realtime/stop";
pub(crate) const NOTIFY_ACCOUNT_UPDATED: &str = "account/updated";
pub(crate) const NOTIFY_ACCOUNT_RATE_LIMITS_UPDATED: &str = "account/rateLimits/updated";
pub(crate) const NOTIFY_THREAD_STARTED: &str = "thread/started";
pub(crate) const NOTIFY_THREAD_COMPACTED: &str = "thread/compacted";
pub(crate) const NOTIFY_THREAD_GOAL_CLEARED: &str = "thread/goal/cleared";
pub(crate) const NOTIFY_THREAD_GOAL_UPDATED: &str = "thread/goal/updated";
pub(crate) const NOTIFY_THREAD_REALTIME_CLOSED: &str = "thread/realtime/closed";
pub(crate) const NOTIFY_THREAD_REALTIME_ERROR: &str = "thread/realtime/error";
pub(crate) const NOTIFY_THREAD_REALTIME_ITEM_ADDED: &str = "thread/realtime/itemAdded";
pub(crate) const NOTIFY_THREAD_REALTIME_OUTPUT_AUDIO_DELTA: &str =
    "thread/realtime/outputAudio/delta";
pub(crate) const NOTIFY_THREAD_REALTIME_SDP: &str = "thread/realtime/sdp";
pub(crate) const NOTIFY_THREAD_REALTIME_STARTED: &str = "thread/realtime/started";
pub(crate) const NOTIFY_THREAD_REALTIME_TRANSCRIPT_DELTA: &str = "thread/realtime/transcript/delta";
pub(crate) const NOTIFY_THREAD_REALTIME_TRANSCRIPT_DONE: &str = "thread/realtime/transcript/done";
pub(crate) const NOTIFY_THREAD_TOKEN_USAGE_UPDATED: &str = "thread/tokenUsage/updated";
pub(crate) const NOTIFY_TURN_STARTED: &str = "turn/started";
pub(crate) const NOTIFY_TURN_COMPLETED: &str = "turn/completed";
pub(crate) const NOTIFY_ITEM_STARTED: &str = "item/started";
pub(crate) const NOTIFY_ITEM_COMPLETED: &str = "item/completed";
pub(crate) const NOTIFY_AGENT_MESSAGE_DELTA: &str = "item/agentMessage/delta";
pub(crate) const NOTIFY_COMMAND_OUTPUT_DELTA: &str = "item/commandExecution/outputDelta";
pub(crate) const NOTIFY_FILE_CHANGE_OUTPUT_DELTA: &str = "item/fileChange/outputDelta";
pub(crate) const NOTIFY_FILE_CHANGE_PATCH_UPDATED: &str = "item/fileChange/patchUpdated";
pub(crate) const REQUEST_COMMAND_APPROVAL: &str = "item/commandExecution/requestApproval";
pub(crate) const REQUEST_FILE_CHANGE_APPROVAL: &str = "item/fileChange/requestApproval";
pub(crate) const REQUEST_PERMISSIONS_APPROVAL: &str = "item/permissions/requestApproval";
pub(crate) const REQUEST_USER_INPUT: &str = "request_user_input";
const JSONRPC_ERROR_REQUEST_DENIED: i64 = -32001;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppServerMessageKind {
    Response { id: u64 },
    Request { id: u64, method: String },
    Notification { method: String },
}

pub(crate) struct CodexAppServerProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

pub(crate) struct CodexAppServerProcessParts {
    pub child: Child,
    pub stdin: ChildStdin,
    pub stdout: Lines<BufReader<ChildStdout>>,
    #[cfg(unix)]
    pub pgid: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AppServerBridgeState {
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub latest_usage: Option<AppServerTokenUsage>,
    pending_approval_methods: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppServerTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: Option<u64>,
    pub context_window_tokens: Option<u64>,
}

fn app_server_args() -> [&'static str; 3] {
    ["app-server", "--listen", "stdio://"]
}

pub(crate) fn spawn_app_server_process_parts(
    cli_path: &str,
) -> Result<CodexAppServerProcessParts, String> {
    let mut command = Command::new(cli_path);
    command
        .args(app_server_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    // SAFETY: setsid() is async-signal-safe per POSIX.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("failed to spawn codex app-server: {e}"))?;
    #[cfg(unix)]
    let pgid = child.id();
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "codex app-server stdin is unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "codex app-server stdout is unavailable".to_string())?;
    Ok(CodexAppServerProcessParts {
        child,
        stdin,
        stdout: BufReader::new(stdout).lines(),
        #[cfg(unix)]
        pgid,
    })
}

pub(crate) fn encode_jsonl(message: &Value) -> Result<Vec<u8>, String> {
    let mut line = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    line.push(b'\n');
    Ok(line)
}

pub(crate) fn decode_jsonrpc_line(line: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(line).map_err(|e| format!("invalid app-server JSON-RPC: {e}"))
}

pub(crate) fn message_kind(message: &Value) -> Option<AppServerMessageKind> {
    let obj = message.as_object()?;
    let id = obj.get("id").and_then(Value::as_u64);
    let method = obj.get("method").and_then(Value::as_str);
    match (id, method) {
        (Some(id), None) => Some(AppServerMessageKind::Response { id }),
        (Some(id), Some(method)) => Some(AppServerMessageKind::Request {
            id,
            method: method.to_string(),
        }),
        (None, Some(method)) => Some(AppServerMessageKind::Notification {
            method: method.to_string(),
        }),
        (None, None) => None,
    }
}

fn get_string<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str()
}

fn get_u64(value: &Value, path: &[&str]) -> Option<u64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_u64()
}

fn text_delta_message(delta: &str) -> Value {
    json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "delta": {
                "type": "text_delta",
                "text": delta,
            },
        },
    })
}

fn thinking_delta_message(delta: &str) -> Value {
    json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "delta": {
                "type": "thinking_delta",
                "thinking": delta,
            },
        },
    })
}

fn tool_use_message(tool: &str, input: Value, id: &str) -> Value {
    json!({
        "type": "assistant",
        "message": {
            "content": [{
                "type": "tool_use",
                "name": tool,
                "input": input,
                "id": id,
            }],
        },
    })
}

fn todo_list_snapshot_message(items: Value) -> Value {
    json!({
        "type": "todo_list_snapshot",
        "items": items,
    })
}

fn tool_result_message(tool_use_id: &str, content: String, is_error: bool) -> Value {
    json!({
        "type": "user",
        "message": {
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error,
            }],
        },
    })
}

fn item_type_name(item_type: &str) -> &str {
    match item_type {
        "command_execution" => "commandExecution",
        "file_change" => "fileChange",
        "mcp_tool_call" => "mcpToolCall",
        "web_search" => "webSearch",
        "todo_list" => "todoList",
        other => other,
    }
}

fn mcp_tool_name(item: &Value) -> String {
    let server = get_string(item, &["server"]).unwrap_or("server");
    let tool = get_string(item, &["tool"]).unwrap_or("tool");
    format!("mcp__{server}__{tool}")
}

fn item_tool_name(item_type: &str, item: &Value) -> Option<String> {
    match item_type_name(item_type) {
        "commandExecution" => Some("Bash".to_string()),
        "fileChange" => Some("Edit".to_string()),
        "mcpToolCall" => Some(mcp_tool_name(item)),
        "dynamicToolCall" => get_string(item, &["tool"])
            .map(ToString::to_string)
            .or_else(|| Some("CodexTool".to_string())),
        "webSearch" => Some("WebSearch".to_string()),
        _ => None,
    }
}

fn mcp_result_content(item: &Value) -> Option<String> {
    if let Some(message) = get_string(item, &["error", "message"]) {
        return Some(message.to_string());
    }
    let content = item.get("result")?.get("content")?.as_array()?;
    let parts = content
        .iter()
        .filter_map(|block| {
            block
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| block.get("content").and_then(Value::as_str))
                .or_else(|| block.as_str())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn codex_question_input(arguments: Value) -> Value {
    if arguments.get("questions").is_some() {
        return arguments;
    }
    let question = arguments
        .get("question")
        .or_else(|| arguments.get("prompt"))
        .and_then(Value::as_str)
        .unwrap_or("Please choose an option.");
    let header = arguments
        .get("header")
        .and_then(Value::as_str)
        .unwrap_or("Question");
    let options = arguments
        .get("options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|option| {
            if let Some(label) = option.as_str() {
                return Some(json!({ "label": label, "description": "" }));
            }
            let label = option.get("label").and_then(Value::as_str)?;
            Some(json!({
                "label": label,
                "description": option
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            }))
        })
        .collect::<Vec<_>>();
    json!({
        "questions": [{
            "question": question,
            "header": header,
            "options": options,
            "multiSelect": arguments
                .get("multiSelect")
                .or_else(|| arguments.get("multi_select"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }]
    })
}

fn codex_request_user_input_message(item_id: &str, arguments: Value) -> Value {
    json!({
        "type": "permission_request",
        "request_id": item_id,
        "tool_name": "AskUserQuestion",
        "display_name": "Question",
        "input": codex_question_input(arguments),
        "tool_use_id": item_id,
        "title": "Question",
        "description": "Codex requests user input",
    })
}

fn file_change_kind(change: &Value) -> String {
    change
        .get("kind")
        .and_then(|kind| {
            kind.as_str()
                .or_else(|| kind.get("type").and_then(Value::as_str))
        })
        .unwrap_or("update")
        .to_string()
}

fn file_change_tool_use_id(item_id: &str, change: &Value, index: usize) -> String {
    let suffix = get_string(change, &["path"])
        .filter(|path| !path.is_empty())
        .map(|path| {
            path.chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                        ch
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .unwrap_or_else(|| format!("file_{}", index + 1));
    format!("{item_id}:{suffix}")
}

fn file_change_tool_messages(item_id: &str, changes: &Value, is_error: bool) -> Vec<Value> {
    let Some(changes) = changes.as_array() else {
        return Vec::new();
    };

    changes
        .iter()
        .enumerate()
        .flat_map(|(index, change)| {
            let tool_use_id = file_change_tool_use_id(item_id, change, index);
            let path = get_string(change, &["path"]).unwrap_or("");
            let diff = get_string(change, &["diff"]).unwrap_or("");
            let input = json!({
                "file_path": path,
                "codex_file_change": true,
                "kind": file_change_kind(change),
                "diff": diff,
                "changes": [change.clone()],
            });
            let content = if diff.is_empty() {
                serde_json::to_string(change)
                    .unwrap_or_else(|_| "Codex file change completed.".to_string())
            } else {
                diff.to_string()
            };
            [
                tool_use_message("Edit", input, &tool_use_id),
                tool_result_message(&tool_use_id, content, is_error),
            ]
        })
        .collect()
}

fn item_started_message(item: &Value) -> Option<Value> {
    let item_id = get_string(item, &["id"])?;
    let item_type = item_type_name(get_string(item, &["type"])?);
    if item_type == "dynamicToolCall" && get_string(item, &["tool"]) == Some("request_user_input") {
        return Some(codex_request_user_input_message(
            item_id,
            item.get("arguments").cloned().unwrap_or_else(|| json!({})),
        ));
    }
    let tool = item_tool_name(item_type, item)?;
    let input = match item_type {
        "commandExecution" => json!({
            "command": item.get("command").cloned().unwrap_or(Value::Null),
            "cwd": item.get("cwd").cloned().unwrap_or(Value::Null),
            "source": item.get("source").cloned().unwrap_or(Value::Null),
            "status": item.get("status").cloned().unwrap_or(Value::Null),
        }),
        "fileChange" => return None,
        "mcpToolCall" => json!({
            "server": item.get("server").cloned().unwrap_or(Value::Null),
            "tool": item.get("tool").cloned().unwrap_or(Value::Null),
            "arguments": item.get("arguments").cloned().unwrap_or(Value::Null),
        }),
        "dynamicToolCall" => json!({
            "tool": item.get("tool").cloned().unwrap_or(Value::Null),
            "arguments": item.get("arguments").cloned().unwrap_or(Value::Null),
        }),
        "webSearch" => json!({
            "query": item.get("query").cloned().unwrap_or(Value::Null),
        }),
        _ => return None,
    };
    Some(tool_use_message(&tool, input, item_id))
}

fn item_started_messages(item: &Value) -> Vec<Value> {
    let item_id = get_string(item, &["id"]).unwrap_or("");
    let item_type = get_string(item, &["type"])
        .map(item_type_name)
        .unwrap_or("");
    if item_type == "fileChange" {
        return file_change_tool_messages(
            item_id,
            &item.get("changes").cloned().unwrap_or_else(|| json!([])),
            false,
        );
    }
    item_started_message(item).into_iter().collect()
}

fn item_completed_message(item: &Value) -> Option<Value> {
    let item_id = get_string(item, &["id"])?;
    let item_type = item_type_name(get_string(item, &["type"])?);
    match item_type {
        "reasoning" => get_string(item, &["text"])
            .filter(|text| !text.is_empty())
            .map(thinking_delta_message),
        "error" => get_string(item, &["message"]).map(error_message),
        "todoList" => Some(todo_list_snapshot_message(
            item.get("items").cloned().unwrap_or_else(|| json!([])),
        )),
        "webSearch" => {
            let query = get_string(item, &["query"]).unwrap_or("");
            Some(tool_result_message(
                item_id,
                if query.is_empty() {
                    "Web search completed.".to_string()
                } else {
                    format!("Searched for \"{query}\".")
                },
                false,
            ))
        }
        "commandExecution" => {
            let content = item
                .get("aggregatedOutput")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    let command = get_string(item, &["command"]).unwrap_or("command");
                    let status = get_string(item, &["status"]).unwrap_or("completed");
                    let exit_code = item.get("exitCode").and_then(Value::as_i64);
                    match exit_code {
                        Some(code) => {
                            format!("Codex command `{command}` finished with status {status} and exit code {code}.")
                        }
                        None => format!("Codex command `{command}` finished with status {status}."),
                    }
                });
            let is_error = matches!(
                get_string(item, &["status"]),
                Some("failed") | Some("declined")
            ) || item
                .get("exitCode")
                .and_then(Value::as_i64)
                .is_some_and(|code| code != 0);
            Some(tool_result_message(item_id, content, is_error))
        }
        "fileChange" => None,
        "mcpToolCall" => {
            let content = mcp_result_content(item)
                .unwrap_or_else(|| "Codex MCP tool call completed.".to_string());
            let is_error = get_string(item, &["status"])
                .is_some_and(|status| matches!(status, "failed" | "errored" | "declined"))
                || item.get("error").is_some();
            Some(tool_result_message(item_id, content, is_error))
        }
        "dynamicToolCall" => {
            let content = serde_json::to_string(item)
                .unwrap_or_else(|_| "Codex tool call completed.".to_string());
            let is_error = get_string(item, &["status"])
                .is_some_and(|status| matches!(status, "failed" | "errored" | "declined"));
            Some(tool_result_message(item_id, content, is_error))
        }
        _ => None,
    }
}

fn item_completed_messages(item: &Value) -> Vec<Value> {
    let item_id = get_string(item, &["id"]).unwrap_or("");
    let item_type = get_string(item, &["type"])
        .map(item_type_name)
        .unwrap_or("");
    if item_type == "fileChange" {
        let is_error = !matches!(get_string(item, &["status"]), Some("completed"));
        return file_change_tool_messages(
            item_id,
            &item.get("changes").cloned().unwrap_or_else(|| json!([])),
            is_error,
        );
    }
    item_completed_message(item).into_iter().collect()
}

fn command_output_delta_message(params: &Value) -> Option<Value> {
    let item_id = get_string(params, &["itemId"])?;
    let delta = get_string(params, &["delta"])?;
    Some(tool_result_message(item_id, delta.to_string(), false))
}

fn file_change_patch_messages(params: &Value) -> Vec<Value> {
    let Some(item_id) = get_string(params, &["itemId"]) else {
        return Vec::new();
    };
    let changes = params.get("changes").cloned().unwrap_or_else(|| json!([]));
    file_change_tool_messages(item_id, &changes, false)
}

fn usage_result_message(
    thread_id: Option<&str>,
    latest_usage: Option<AppServerTokenUsage>,
) -> Value {
    let usage = latest_usage.unwrap_or(AppServerTokenUsage {
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: None,
        context_window_tokens: None,
    });
    json!({
        "type": "result",
        "session_id": thread_id,
        "modelUsage": {
            "codex": {
                "inputTokens": usage.input_tokens,
                "outputTokens": usage.output_tokens,
                "totalTokens": usage.total_tokens,
                "contextWindowTokens": usage.context_window_tokens,
            },
        },
    })
}

fn turn_complete_message(thread_id: Option<&str>, exit_code: i32) -> Value {
    json!({
        "type": "turn_complete",
        "session_id": thread_id,
        "exit_code": exit_code,
    })
}

fn error_message(message: &str) -> Value {
    json!({
        "type": "error",
        "message": message,
    })
}

fn session_ready_message(thread_id: &str) -> Value {
    json!({
        "type": "session_ready",
        "session_id": thread_id,
    })
}

fn compaction_completed_message(params: &Value) -> Value {
    let trigger = get_string(params, &["trigger"]).unwrap_or("manual");
    json!({
        "type": "system",
        "subtype": "compact_boundary",
        "compact_metadata": {
            "trigger": trigger,
            "pre_summary_token_count": params
                .get("preSummaryTokenCount")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
    })
}

fn app_server_request_to_permission_request(id: u64, method: &str, params: &Value) -> Value {
    let item_id = get_string(params, &["itemId"])
        .or_else(|| get_string(params, &["item", "id"]))
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("app-server-request-{id}"));
    let tool_name = match method {
        REQUEST_COMMAND_APPROVAL => "CodexCommand",
        REQUEST_FILE_CHANGE_APPROVAL => "CodexFileChange",
        REQUEST_PERMISSIONS_APPROVAL => "CodexPermissions",
        _ if is_user_input_request_method(method) => "AskUserQuestion",
        _ => "CodexApproval",
    };
    let input = if is_user_input_request_method(method) {
        codex_question_input(params.clone())
    } else {
        params.clone()
    };
    json!({
        "type": "permission_request",
        "request_id": id.to_string(),
        "tool_name": tool_name,
        "display_name": tool_name,
        "input": input,
        "tool_use_id": item_id,
        "title": if is_user_input_request_method(method) { "Question" } else { "Codex approval requested" },
        "description": method,
    })
}

fn is_user_input_request_method(method: &str) -> bool {
    let lower = method.to_lowercase();
    lower.contains("request_user_input") || lower.contains("requestuserinput")
}

fn jsonrpc_response(id: u64, result: Value) -> Value {
    json!({
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: u64, code: i64, message: &str) -> Value {
    json!({
        "id": id,
        "error": {
            "code": code,
            "message": message,
        },
    })
}

fn permission_grant_from_updated_input(updated_input: Option<&str>) -> Option<Value> {
    let parsed = updated_input
        .and_then(|input| serde_json::from_str::<Value>(input).ok())
        .unwrap_or(Value::Null);

    parsed
        .get("permissions")
        .cloned()
        .or_else(|| parsed.get("grantedPermissions").cloned())
        .or_else(|| {
            if parsed.get("fileSystem").is_some() || parsed.get("network").is_some() {
                Some(parsed)
            } else {
                None
            }
        })
}

fn display_editor_path(cwd: &str, path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let path = Path::new(trimmed);
    let cwd = Path::new(cwd);
    path.strip_prefix(cwd)
        .ok()
        .and_then(|relative| relative.to_str())
        .map(|relative| {
            let relative = relative.trim();
            if relative.is_empty() {
                ".".to_string()
            } else {
                relative.to_string()
            }
        })
        .unwrap_or_else(|| trimmed.to_string())
}

fn resolve_editor_path(cwd: &str, path: &str) -> Option<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cwd_path = Path::new(cwd);
    let candidate = Path::new(trimmed);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd_path.join(candidate)
    };
    if absolute.strip_prefix(cwd_path).is_err() {
        return None;
    }
    Some(absolute)
}

fn selected_line_excerpt(content: &str, start_line: u32, end_line: u32) -> Option<String> {
    if start_line == 0 || end_line == 0 {
        return None;
    }
    let start = start_line.min(end_line) as usize;
    let end = start_line.max(end_line) as usize;
    let mut selected = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if line_no >= start && line_no <= end {
            selected.push(line);
        }
        if line_no > end || selected.len() >= 120 {
            break;
        }
    }
    if selected.is_empty() {
        return None;
    }
    let mut text = selected.join("\n");
    const MAX_SELECTION_CHARS: usize = 8_000;
    if text.chars().count() > MAX_SELECTION_CHARS {
        text = text.chars().take(MAX_SELECTION_CHARS).collect();
        text.push_str("\n...[truncated]");
    }
    Some(text)
}

fn editor_selection_context_lines(cwd: &str, context: &AgentEditorContext) -> Vec<String> {
    let Some(selection) = context.selection.as_ref() else {
        return Vec::new();
    };
    let file_path = selection.file_path.trim();
    if file_path.is_empty() || selection.start_line == 0 || selection.end_line == 0 {
        return Vec::new();
    }
    let display_path = display_editor_path(cwd, file_path);
    let start_line = selection.start_line.min(selection.end_line);
    let end_line = selection.start_line.max(selection.end_line);
    let mut lines = vec![format!(
        "Current selection: {display_path}:L{start_line}-L{end_line}"
    )];

    if let Some(path) = resolve_editor_path(cwd, file_path) {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some(excerpt) = selected_line_excerpt(&content, start_line, end_line) {
                lines.push("Selected text:".to_string());
                lines.push("```".to_string());
                lines.push(excerpt);
                lines.push("```".to_string());
            }
        }
    }

    lines
}

pub(crate) fn build_editor_additional_context(
    cwd: &str,
    editor_context: Option<&AgentEditorContext>,
) -> Option<Value> {
    let context = editor_context?;
    let active_path = context
        .active_editor_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|path| display_editor_path(cwd, path));
    let mut open_paths = Vec::new();
    for path in &context.open_editor_paths {
        let display = display_editor_path(cwd, path);
        if !display.is_empty() && !open_paths.contains(&display) {
            open_paths.push(display);
        }
    }

    let selection_lines = editor_selection_context_lines(cwd, context);

    if active_path.is_none() && open_paths.is_empty() && selection_lines.is_empty() {
        return None;
    }

    let mut lines = Vec::new();
    if let Some(active_path) = active_path {
        lines.push(format!("Active editor: {active_path}"));
    }
    if !open_paths.is_empty() {
        lines.push("Open editors:".to_string());
        for path in open_paths {
            lines.push(format!("- {path}"));
        }
    }
    lines.extend(selection_lines);

    Some(json!({
        "releash.ide": {
            "kind": "untrusted",
            "value": lines.join("\n"),
        },
    }))
}

pub(crate) fn build_app_server_permission_response(
    id: u64,
    method: &str,
    behavior: &str,
    updated_input: Option<&str>,
) -> Result<Value, String> {
    match (method, behavior) {
        (REQUEST_COMMAND_APPROVAL, "allow") => {
            Ok(jsonrpc_response(id, json!({ "decision": "accept" })))
        }
        (REQUEST_COMMAND_APPROVAL, "deny") => {
            Ok(jsonrpc_response(id, json!({ "decision": "decline" })))
        }
        (REQUEST_FILE_CHANGE_APPROVAL, "allow") => {
            Ok(jsonrpc_response(id, json!({ "decision": "accept" })))
        }
        (REQUEST_FILE_CHANGE_APPROVAL, "deny") => {
            Ok(jsonrpc_response(id, json!({ "decision": "decline" })))
        }
        (REQUEST_PERMISSIONS_APPROVAL, "allow") => {
            let permissions =
                permission_grant_from_updated_input(updated_input).unwrap_or_else(|| {
                    json!({
                        "fileSystem": null,
                        "network": null,
                    })
                });
            Ok(jsonrpc_response(
                id,
                json!({
                    "permissions": permissions,
                    "scope": "turn",
                }),
            ))
        }
        (REQUEST_PERMISSIONS_APPROVAL, "deny") => Ok(jsonrpc_error(
            id,
            JSONRPC_ERROR_REQUEST_DENIED,
            "User denied additional permissions",
        )),
        (method, "allow") if is_user_input_request_method(method) => {
            let answers = updated_input
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .and_then(|value| value.get("answers").cloned())
                .unwrap_or(Value::Null);
            Ok(jsonrpc_response(id, json!({ "answers": answers })))
        }
        (method, "deny") if is_user_input_request_method(method) => Ok(jsonrpc_error(
            id,
            JSONRPC_ERROR_REQUEST_DENIED,
            "User declined to answer",
        )),
        (_, "allow" | "deny") => Err(format!("Unsupported app-server approval method: {method}")),
        (_, _) => Err(format!("Invalid behavior: {behavior}")),
    }
}

pub(crate) fn build_app_server_permission_response_for_bridge_request(
    state: &mut AppServerBridgeState,
    request_id: &str,
    behavior: &str,
    updated_input: Option<&str>,
) -> Result<Value, String> {
    let id = request_id
        .parse::<u64>()
        .map_err(|_| format!("Invalid app-server request id: {request_id}"))?;
    let method = state
        .pending_approval_methods
        .remove(request_id)
        .ok_or_else(|| format!("Unknown app-server approval request: {request_id}"))?;
    build_app_server_permission_response(id, &method, behavior, updated_input)
}

pub(crate) fn app_server_message_to_bridge_messages(
    message: &Value,
    state: &mut AppServerBridgeState,
) -> Vec<Value> {
    match message_kind(message) {
        Some(AppServerMessageKind::Notification { method }) => {
            let params = message.get("params").unwrap_or(&Value::Null);
            match method.as_str() {
                NOTIFY_ACCOUNT_UPDATED | NOTIFY_ACCOUNT_RATE_LIMITS_UPDATED => Vec::new(),
                NOTIFY_THREAD_STARTED => {
                    if let Some(thread_id) = get_string(params, &["thread", "id"]) {
                        state.thread_id = Some(thread_id.to_string());
                        vec![session_ready_message(thread_id)]
                    } else {
                        Vec::new()
                    }
                }
                NOTIFY_THREAD_COMPACTED => vec![compaction_completed_message(params)],
                NOTIFY_THREAD_GOAL_UPDATED | NOTIFY_THREAD_GOAL_CLEARED => Vec::new(),
                NOTIFY_THREAD_REALTIME_STARTED
                | NOTIFY_THREAD_REALTIME_ITEM_ADDED
                | NOTIFY_THREAD_REALTIME_TRANSCRIPT_DELTA
                | NOTIFY_THREAD_REALTIME_TRANSCRIPT_DONE
                | NOTIFY_THREAD_REALTIME_OUTPUT_AUDIO_DELTA
                | NOTIFY_THREAD_REALTIME_SDP
                | NOTIFY_THREAD_REALTIME_ERROR
                | NOTIFY_THREAD_REALTIME_CLOSED => Vec::new(),
                NOTIFY_THREAD_TOKEN_USAGE_UPDATED => {
                    let input = get_u64(params, &["tokenUsage", "last", "inputTokens"]);
                    let output = get_u64(params, &["tokenUsage", "last", "outputTokens"]);
                    if let (Some(input), Some(output)) = (input, output) {
                        let usage = AppServerTokenUsage {
                            input_tokens: input,
                            output_tokens: output,
                            total_tokens: get_u64(params, &["tokenUsage", "total", "totalTokens"]),
                            context_window_tokens: get_u64(
                                params,
                                &["tokenUsage", "modelContextWindow"],
                            ),
                        };
                        state.latest_usage = Some(usage);
                        vec![usage_result_message(
                            state.thread_id.as_deref(),
                            Some(usage),
                        )]
                    } else {
                        Vec::new()
                    }
                }
                NOTIFY_TURN_STARTED => {
                    if let Some(turn_id) = get_string(params, &["turn", "id"]) {
                        state.turn_id = Some(turn_id.to_string());
                    }
                    Vec::new()
                }
                NOTIFY_ITEM_STARTED => params
                    .get("item")
                    .map(item_started_messages)
                    .unwrap_or_default(),
                NOTIFY_ITEM_COMPLETED => params
                    .get("item")
                    .map(item_completed_messages)
                    .unwrap_or_default(),
                NOTIFY_AGENT_MESSAGE_DELTA => get_string(params, &["delta"])
                    .map(text_delta_message)
                    .into_iter()
                    .collect(),
                NOTIFY_COMMAND_OUTPUT_DELTA | NOTIFY_FILE_CHANGE_OUTPUT_DELTA => {
                    command_output_delta_message(params).into_iter().collect()
                }
                NOTIFY_FILE_CHANGE_PATCH_UPDATED => file_change_patch_messages(params),
                NOTIFY_TURN_COMPLETED => {
                    let thread_id =
                        get_string(params, &["threadId"]).or(state.thread_id.as_deref());
                    let status = get_string(params, &["turn", "status"]).unwrap_or("completed");
                    let exit_code = if status == "failed" { 1 } else { 0 };
                    let mut messages = Vec::new();
                    if exit_code != 0 {
                        let error = get_string(params, &["turn", "error", "message"])
                            .unwrap_or("Codex turn failed");
                        messages.push(error_message(error));
                    }
                    messages.push(usage_result_message(thread_id, state.latest_usage));
                    messages.push(turn_complete_message(thread_id, exit_code));
                    state.turn_id = None;
                    messages
                }
                _ => Vec::new(),
            }
        }
        Some(AppServerMessageKind::Response { .. }) => {
            let Some(thread_id) = get_string(message, &["result", "thread", "id"]) else {
                return Vec::new();
            };
            if state.thread_id.is_some() {
                return Vec::new();
            }
            state.thread_id = Some(thread_id.to_string());
            vec![session_ready_message(thread_id)]
        }
        Some(AppServerMessageKind::Request { id, method }) => {
            let params = message.get("params").unwrap_or(&Value::Null);
            match method.as_str() {
                REQUEST_COMMAND_APPROVAL
                | REQUEST_FILE_CHANGE_APPROVAL
                | REQUEST_PERMISSIONS_APPROVAL => {
                    state
                        .pending_approval_methods
                        .insert(id.to_string(), method.clone());
                    vec![app_server_request_to_permission_request(
                        id, &method, params,
                    )]
                }
                _ if is_user_input_request_method(&method) => {
                    state
                        .pending_approval_methods
                        .insert(id.to_string(), method.clone());
                    vec![app_server_request_to_permission_request(
                        id, &method, params,
                    )]
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

impl CodexAppServerProcess {
    pub(crate) fn spawn(cli_path: &str) -> Result<Self, String> {
        let parts = spawn_app_server_process_parts(cli_path)?;
        Ok(Self {
            child: parts.child,
            stdin: parts.stdin,
            stdout: parts.stdout,
            next_id: 1,
        })
    }

    pub(crate) fn next_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(crate) async fn send(&mut self, message: &Value) -> Result<(), String> {
        let line = encode_jsonl(message)?;
        self.stdin
            .write_all(&line)
            .await
            .map_err(|e| format!("failed to write codex app-server message: {e}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("failed to flush codex app-server stdin: {e}"))
    }

    pub(crate) async fn read_message(&mut self) -> Result<Option<Value>, String> {
        loop {
            let Some(line) = self
                .stdout
                .next_line()
                .await
                .map_err(|e| format!("failed to read codex app-server stdout: {e}"))?
            else {
                return Ok(None);
            };
            if line.trim().is_empty() {
                continue;
            }
            return decode_jsonrpc_line(&line).map(Some);
        }
    }

    pub(crate) async fn initialize(&mut self, version: &str) -> Result<u64, String> {
        let id = self.next_request_id();
        let request = build_initialize_request(id, version);
        self.send(&request).await?;
        let initialized = build_initialized_notification();
        self.send(&initialized).await?;
        Ok(id)
    }

    pub(crate) async fn start_thread(
        &mut self,
        cwd: &str,
        model: Option<&str>,
        permission_mode: Option<&str>,
        plan_mode: bool,
        permission_profile_id: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Result<u64, String> {
        let id = self.next_request_id();
        let request = build_thread_start_request(
            id,
            cwd,
            model,
            permission_mode,
            plan_mode,
            permission_profile_id,
            system_prompt,
        )?;
        self.send(&request).await?;
        Ok(id)
    }

    pub(crate) async fn start_turn(
        &mut self,
        thread_id: &str,
        cwd: &str,
        content: &str,
        images: &[ImageAttachment],
        client_user_message_id: Option<&str>,
    ) -> Result<u64, String> {
        let id = self.next_request_id();
        let request = build_turn_start_request(
            id,
            thread_id,
            cwd,
            content,
            images,
            client_user_message_id,
            None,
        )?;
        self.send(&request).await?;
        Ok(id)
    }

    pub(crate) async fn steer_turn(
        &mut self,
        thread_id: &str,
        expected_turn_id: &str,
        cwd: &str,
        content: &str,
        images: &[ImageAttachment],
        client_user_message_id: Option<&str>,
    ) -> Result<u64, String> {
        let id = self.next_request_id();
        let request = build_turn_steer_request(
            id,
            thread_id,
            expected_turn_id,
            cwd,
            content,
            images,
            client_user_message_id,
            None,
        );
        self.send(&request).await?;
        Ok(id)
    }

    pub(crate) async fn interrupt_turn(
        &mut self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<u64, String> {
        let id = self.next_request_id();
        let request = build_turn_interrupt_request(id, thread_id, turn_id);
        self.send(&request).await?;
        Ok(id)
    }

    pub(crate) async fn read_response_result(&mut self, id: u64) -> Result<Value, String> {
        loop {
            let message = timeout(Duration::from_secs(10), self.read_message())
                .await
                .map_err(|_| format!("timed out waiting for codex app-server response {id}"))?
                .and_then(|message| {
                    message.ok_or_else(|| {
                        format!("codex app-server exited before response {id} was received")
                    })
                })?;

            if !matches!(message_kind(&message), Some(AppServerMessageKind::Response { id: response_id }) if response_id == id)
            {
                continue;
            }
            if let Some(error) = message.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("codex app-server request failed");
                return Err(message.to_string());
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    pub(crate) async fn shutdown(mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({
        "id": id,
        "method": method,
        "params": params,
    })
}

fn notification(method: &str, params: Value) -> Value {
    json!({
        "method": method,
        "params": params,
    })
}

fn parsed_mode(mode: Option<&str>) -> Result<PermissionMode, String> {
    PermissionMode::parse(mode.unwrap_or("edit")).map_err(|e| e.to_string())
}

fn approval_policy(mode: PermissionMode) -> &'static str {
    codex_approval_policy_from_mode(mode)
}

fn sandbox_mode(mode: PermissionMode) -> &'static str {
    codex_sandbox_mode_from_mode(mode)
}

fn normalized_permission_profile_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn sandbox_policy(mode: PermissionMode, cwd: &str) -> Value {
    match mode {
        PermissionMode::Ask => json!({
            "type": "readOnly",
            "networkAccess": false,
        }),
        PermissionMode::Edit => json!({
            "type": "workspaceWrite",
            "writableRoots": [cwd],
            "networkAccess": false,
            "excludeTmpdirEnvVar": false,
            "excludeSlashTmp": false,
        }),
        PermissionMode::Full => json!({
            "type": "dangerFullAccess",
        }),
    }
}

fn apply_plan_mode(params: &mut Value) {
    params["collaborationMode"] = Value::String("plan".to_string());
    params["approvalPolicy"] = Value::String("on-request".to_string());
    params["sandbox"] = Value::String("read-only".to_string());
    params["sandboxPolicy"] = json!({
        "type": "readOnly",
        "networkAccess": false,
    });
}

pub(crate) fn build_initialize_request(id: u64, version: &str) -> Value {
    request(
        id,
        METHOD_INITIALIZE,
        json!({
            "clientInfo": {
                "name": "releash",
                "title": "Releash",
                "version": version,
            },
            "capabilities": {
                "experimentalApi": true,
                "requestAttestation": false,
            },
        }),
    )
}

pub(crate) fn build_initialized_notification() -> Value {
    notification(METHOD_INITIALIZED, json!({}))
}

pub(crate) fn build_thread_start_request(
    id: u64,
    cwd: &str,
    model: Option<&str>,
    permission_mode: Option<&str>,
    plan_mode: bool,
    permission_profile_id: Option<&str>,
    system_prompt: Option<&str>,
) -> Result<Value, String> {
    let mut params = json!({
        "cwd": cwd,
        "runtimeWorkspaceRoots": [cwd],
        "threadSource": "user",
    });
    if let Some(profile_id) = normalized_permission_profile_id(permission_profile_id) {
        params["permissions"] = Value::String(profile_id);
    } else {
        let mode = parsed_mode(permission_mode)?;
        params["approvalPolicy"] = Value::String(approval_policy(mode).to_string());
        params["sandbox"] = Value::String(sandbox_mode(mode).to_string());
    }
    if plan_mode {
        apply_plan_mode(&mut params);
    }
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        params["model"] = Value::String(model.to_string());
    }
    if let Some(system_prompt) = system_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params["developerInstructions"] = Value::String(system_prompt.to_string());
    }
    Ok(request(id, METHOD_THREAD_START, params))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_thread_resume_request(
    id: u64,
    thread_id: &str,
    cwd: &str,
    model: Option<&str>,
    permission_mode: Option<&str>,
    plan_mode: bool,
    permission_profile_id: Option<&str>,
    system_prompt: Option<&str>,
) -> Result<Value, String> {
    let mut params = json!({
        "threadId": thread_id,
        "cwd": cwd,
        "runtimeWorkspaceRoots": [cwd],
    });
    if let Some(profile_id) = normalized_permission_profile_id(permission_profile_id) {
        params["permissions"] = Value::String(profile_id);
    } else {
        let mode = parsed_mode(permission_mode)?;
        params["approvalPolicy"] = Value::String(approval_policy(mode).to_string());
        params["sandbox"] = Value::String(sandbox_mode(mode).to_string());
    }
    if plan_mode {
        apply_plan_mode(&mut params);
    }
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        params["model"] = Value::String(model.to_string());
    }
    if let Some(system_prompt) = system_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params["developerInstructions"] = Value::String(system_prompt.to_string());
    }
    Ok(request(id, METHOD_THREAD_RESUME, params))
}

pub(crate) fn build_thread_fork_request(
    id: u64,
    thread_id: &str,
    cwd: &str,
    model: Option<&str>,
    permission_mode: Option<&str>,
    plan_mode: bool,
    permission_profile_id: Option<&str>,
) -> Result<Value, String> {
    let mut params = json!({
        "threadId": thread_id,
        "cwd": cwd,
        "runtimeWorkspaceRoots": [cwd],
        "threadSource": "user",
        "excludeTurns": true,
    });
    if let Some(profile_id) = normalized_permission_profile_id(permission_profile_id) {
        params["permissions"] = Value::String(profile_id);
    } else {
        let mode = parsed_mode(permission_mode)?;
        params["approvalPolicy"] = Value::String(approval_policy(mode).to_string());
        params["sandbox"] = Value::String(sandbox_mode(mode).to_string());
    }
    if plan_mode {
        apply_plan_mode(&mut params);
    }
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        params["model"] = Value::String(model.to_string());
    }
    Ok(request(id, METHOD_THREAD_FORK, params))
}

pub(crate) fn build_thread_archive_request(id: u64, thread_id: &str) -> Value {
    request(
        id,
        METHOD_THREAD_ARCHIVE,
        json!({
            "threadId": thread_id,
        }),
    )
}

pub(crate) fn build_thread_unarchive_request(id: u64, thread_id: &str) -> Value {
    request(
        id,
        METHOD_THREAD_UNARCHIVE,
        json!({
            "threadId": thread_id,
        }),
    )
}

pub(crate) fn build_thread_list_request(id: u64, cwd: &str, cursor: Option<&str>) -> Value {
    let mut params = json!({
        "archived": false,
        "cwd": cwd,
        "limit": 20,
        "sortDirection": "desc",
        "sortKey": "updated_at",
    });
    if let Some(cursor) = cursor.map(str::trim).filter(|value| !value.is_empty()) {
        params["cursor"] = Value::String(cursor.to_string());
    }
    request(id, METHOD_THREAD_LIST, params)
}

pub(crate) fn build_thread_search_request(
    id: u64,
    search_term: &str,
    cursor: Option<&str>,
) -> Value {
    let mut params = json!({
        "archived": false,
        "limit": 20,
        "searchTerm": search_term,
        "sortDirection": "desc",
        "sortKey": "updated_at",
    });
    if let Some(cursor) = cursor.map(str::trim).filter(|value| !value.is_empty()) {
        params["cursor"] = Value::String(cursor.to_string());
    }
    request(id, METHOD_THREAD_SEARCH, params)
}

pub(crate) fn build_thread_read_request(id: u64, thread_id: &str, include_turns: bool) -> Value {
    request(
        id,
        METHOD_THREAD_READ,
        json!({
            "threadId": thread_id,
            "includeTurns": include_turns,
        }),
    )
}

pub(crate) fn build_thread_turns_list_request(
    id: u64,
    thread_id: &str,
    cursor: Option<&str>,
) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "itemsView": "full",
        "limit": 20,
        "sortDirection": "asc",
    });
    if let Some(cursor) = cursor.map(str::trim).filter(|value| !value.is_empty()) {
        params["cursor"] = Value::String(cursor.to_string());
    }
    request(id, METHOD_THREAD_TURNS_LIST, params)
}

pub(crate) fn build_thread_turn_items_list_request(
    id: u64,
    thread_id: &str,
    turn_id: &str,
    cursor: Option<&str>,
) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "turnId": turn_id,
        "limit": 100,
        "sortDirection": "asc",
    });
    if let Some(cursor) = cursor.map(str::trim).filter(|value| !value.is_empty()) {
        params["cursor"] = Value::String(cursor.to_string());
    }
    request(id, METHOD_THREAD_TURNS_ITEMS_LIST, params)
}

fn build_user_input(content: &str, images: &[ImageAttachment]) -> Vec<Value> {
    let mut input = Vec::new();
    if !content.is_empty() || images.is_empty() {
        input.push(json!({
            "type": "text",
            "text": content,
        }));
    }
    input.extend(images.iter().map(|image| {
        json!({
            "type": "image",
            "url": format!("data:{};base64,{}", image.media_type, image.data),
        })
    }));
    input
}

pub(crate) fn build_turn_start_request(
    id: u64,
    thread_id: &str,
    cwd: &str,
    content: &str,
    images: &[ImageAttachment],
    client_user_message_id: Option<&str>,
    editor_context: Option<&AgentEditorContext>,
) -> Result<Value, String> {
    let input = build_user_input(content, images);

    let mut params = json!({
        "threadId": thread_id,
        "cwd": cwd,
        "input": input,
        "runtimeWorkspaceRoots": [cwd],
    });
    if let Some(message_id) = client_user_message_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params["clientUserMessageId"] = Value::String(message_id.to_string());
    }
    if let Some(additional_context) = build_editor_additional_context(cwd, editor_context) {
        params["additionalContext"] = additional_context;
    }
    Ok(request(id, METHOD_TURN_START, params))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_turn_start_request_with_permission(
    id: u64,
    thread_id: &str,
    cwd: &str,
    content: &str,
    images: &[ImageAttachment],
    client_user_message_id: Option<&str>,
    editor_context: Option<&AgentEditorContext>,
    permission_mode: Option<&str>,
    plan_mode: bool,
    permission_profile_id: Option<&str>,
) -> Result<Value, String> {
    let mut value = build_turn_start_request(
        id,
        thread_id,
        cwd,
        content,
        images,
        client_user_message_id,
        editor_context,
    )?;
    if let Some(profile_id) = normalized_permission_profile_id(permission_profile_id) {
        value["params"]["permissions"] = Value::String(profile_id);
    } else if let Some(permission_mode) = permission_mode {
        let mode = parsed_mode(Some(permission_mode))?;
        value["params"]["approvalPolicy"] = Value::String(approval_policy(mode).to_string());
        value["params"]["sandboxPolicy"] = sandbox_policy(mode, cwd);
    }
    if plan_mode {
        apply_plan_mode(&mut value["params"]);
    }
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_turn_steer_request(
    id: u64,
    thread_id: &str,
    expected_turn_id: &str,
    cwd: &str,
    content: &str,
    images: &[ImageAttachment],
    client_user_message_id: Option<&str>,
    editor_context: Option<&AgentEditorContext>,
) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "expectedTurnId": expected_turn_id,
        "input": build_user_input(content, images),
    });
    if let Some(message_id) = client_user_message_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params["clientUserMessageId"] = Value::String(message_id.to_string());
    }
    if let Some(additional_context) = build_editor_additional_context(cwd, editor_context) {
        params["additionalContext"] = additional_context;
    }
    request(id, METHOD_TURN_STEER, params)
}

pub(crate) fn build_turn_interrupt_request(id: u64, thread_id: &str, turn_id: &str) -> Value {
    request(
        id,
        METHOD_TURN_INTERRUPT,
        json!({
            "threadId": thread_id,
            "turnId": turn_id,
        }),
    )
}

pub(crate) fn build_thread_name_set_request(id: u64, thread_id: &str, name: &str) -> Value {
    request(
        id,
        METHOD_THREAD_NAME_SET,
        json!({
            "threadId": thread_id,
            "name": name,
        }),
    )
}

pub(crate) fn build_thread_settings_update_permission_request(
    id: u64,
    thread_id: &str,
    cwd: &str,
    permission_mode: &str,
    permission_profile_id: Option<&str>,
) -> Result<Value, String> {
    let params = if let Some(profile_id) = normalized_permission_profile_id(permission_profile_id) {
        json!({
            "threadId": thread_id,
            "permissions": profile_id,
        })
    } else {
        let mode = parsed_mode(Some(permission_mode))?;
        json!({
            "threadId": thread_id,
            "permissions": Value::Null,
            "approvalPolicy": approval_policy(mode),
            "sandboxPolicy": sandbox_policy(mode, cwd),
        })
    };
    Ok(request(id, METHOD_THREAD_SETTINGS_UPDATE, params))
}

pub(crate) fn build_config_read_request(id: u64, cwd: &str, include_layers: bool) -> Value {
    request(
        id,
        METHOD_CONFIG_READ,
        json!({
            "cwd": cwd,
            "includeLayers": include_layers,
        }),
    )
}

pub(crate) fn build_config_requirements_read_request(id: u64) -> Value {
    request(id, METHOD_CONFIG_REQUIREMENTS_READ, Value::Null)
}

pub(crate) fn build_fuzzy_file_search_request(id: u64, root: &str, query: &str) -> Value {
    request(
        id,
        METHOD_FUZZY_FILE_SEARCH,
        json!({
            "query": query,
            "roots": [root],
            "cancellationToken": Value::Null,
        }),
    )
}

pub(crate) fn build_model_provider_capabilities_read_request(id: u64) -> Value {
    request(id, METHOD_MODEL_PROVIDER_CAPABILITIES_READ, json!({}))
}

pub(crate) fn build_collaboration_mode_list_request(id: u64) -> Value {
    request(id, METHOD_COLLABORATION_MODE_LIST, json!({}))
}

pub(crate) fn build_app_list_request(
    id: u64,
    thread_id: Option<&str>,
    cursor: Option<&str>,
) -> Value {
    let mut params = json!({
        "limit": 100,
        "forceRefetch": false,
    });
    if let Some(thread_id) = thread_id.filter(|value| !value.trim().is_empty()) {
        params["threadId"] = json!(thread_id);
    }
    if let Some(cursor) = cursor.filter(|value| !value.trim().is_empty()) {
        params["cursor"] = json!(cursor);
    }
    request(id, METHOD_APP_LIST, params)
}

pub(crate) fn build_plugin_list_request(id: u64, cwd: &str) -> Value {
    request(
        id,
        METHOD_PLUGIN_LIST,
        json!({
            "cwds": [cwd],
            "marketplaceKinds": ["local", "workspace-directory"],
        }),
    )
}

pub(crate) fn build_model_list_request(id: u64, cursor: Option<&str>) -> Value {
    let mut params = json!({
        "includeHidden": false,
        "limit": 100,
    });
    if let Some(cursor) = cursor.map(str::trim).filter(|value| !value.is_empty()) {
        params["cursor"] = Value::String(cursor.to_string());
    }
    request(id, METHOD_MODEL_LIST, params)
}

pub(crate) fn build_skills_list_request(id: u64, cwd: &str, force_reload: bool) -> Value {
    request(
        id,
        METHOD_SKILLS_LIST,
        json!({
            "cwds": [cwd],
            "forceReload": force_reload,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_server_args_use_stdio_transport() {
        assert_eq!(app_server_args(), ["app-server", "--listen", "stdio://"]);
    }

    #[test]
    fn jsonrpc_framing_encodes_one_message_per_line() {
        let value = json!({
            "id": 7,
            "method": "thread/start",
            "params": { "cwd": "/repo" },
        });

        let encoded = encode_jsonl(&value).expect("jsonl");
        assert_eq!(encoded.last(), Some(&b'\n'));
        let decoded = decode_jsonrpc_line(std::str::from_utf8(&encoded).unwrap()).expect("json");
        assert_eq!(decoded, value);
    }

    #[test]
    fn message_kind_classifies_jsonrpc_messages() {
        assert_eq!(
            message_kind(&json!({ "id": 1, "result": {} })),
            Some(AppServerMessageKind::Response { id: 1 })
        );
        assert_eq!(
            message_kind(
                &json!({ "id": 2, "method": "item/permissions/requestApproval", "params": {} })
            ),
            Some(AppServerMessageKind::Request {
                id: 2,
                method: "item/permissions/requestApproval".to_string(),
            })
        );
        assert_eq!(
            message_kind(&json!({ "method": "turn/started", "params": {} })),
            Some(AppServerMessageKind::Notification {
                method: "turn/started".to_string(),
            })
        );
        assert_eq!(message_kind(&json!({ "result": {} })), None);
    }

    #[test]
    fn app_server_thread_started_becomes_session_ready() {
        let mut state = AppServerBridgeState::default();
        let messages = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/started",
                "params": {
                    "thread": { "id": "thr_123" }
                }
            }),
            &mut state,
        );

        assert_eq!(state.thread_id.as_deref(), Some("thr_123"));
        assert_eq!(
            messages,
            vec![json!({ "type": "session_ready", "session_id": "thr_123" })]
        );
    }

    #[test]
    fn app_server_thread_response_becomes_session_ready_once() {
        let mut state = AppServerBridgeState::default();
        let messages = app_server_message_to_bridge_messages(
            &json!({
                "id": 2,
                "result": {
                    "thread": { "id": "thr_123" }
                }
            }),
            &mut state,
        );
        let duplicate = app_server_message_to_bridge_messages(
            &json!({
                "id": 3,
                "result": {
                    "thread": { "id": "thr_123" }
                }
            }),
            &mut state,
        );

        assert_eq!(
            messages,
            vec![json!({ "type": "session_ready", "session_id": "thr_123" })]
        );
        assert!(duplicate.is_empty());
        assert_eq!(state.thread_id.as_deref(), Some("thr_123"));
    }

    #[test]
    fn app_server_agent_delta_becomes_text_delta_stream_event() {
        let mut state = AppServerBridgeState::default();
        let messages = app_server_message_to_bridge_messages(
            &json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "itemId": "item_1",
                    "delta": "hello",
                }
            }),
            &mut state,
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["type"], "stream_event");
        assert_eq!(messages[0]["event"]["delta"]["type"], "text_delta");
        assert_eq!(messages[0]["event"]["delta"]["text"], "hello");
    }

    #[test]
    fn app_server_command_item_lifecycle_becomes_tool_use_and_result() {
        let mut state = AppServerBridgeState::default();
        let started = app_server_message_to_bridge_messages(
            &json!({
                "method": "item/started",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "startedAtMs": 1,
                    "item": {
                        "id": "cmd_1",
                        "type": "commandExecution",
                        "command": "pnpm test",
                        "cwd": "/repo",
                        "commandActions": [],
                        "status": "inProgress"
                    }
                }
            }),
            &mut state,
        );
        let completed = app_server_message_to_bridge_messages(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "completedAtMs": 2,
                    "item": {
                        "id": "cmd_1",
                        "type": "commandExecution",
                        "command": "pnpm test",
                        "cwd": "/repo",
                        "commandActions": [],
                        "status": "completed",
                        "exitCode": 0,
                        "aggregatedOutput": "ok"
                    }
                }
            }),
            &mut state,
        );

        assert_eq!(started.len(), 1);
        assert_eq!(started[0]["type"], "assistant");
        assert_eq!(started[0]["message"]["content"][0]["type"], "tool_use");
        assert_eq!(started[0]["message"]["content"][0]["name"], "Bash");
        assert_eq!(
            started[0]["message"]["content"][0]["input"]["command"],
            "pnpm test"
        );
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0]["type"], "user");
        assert_eq!(completed[0]["message"]["content"][0]["type"], "tool_result");
        assert_eq!(
            completed[0]["message"]["content"][0]["tool_use_id"],
            "cmd_1"
        );
        assert_eq!(completed[0]["message"]["content"][0]["content"], "ok");
        assert_eq!(completed[0]["message"]["content"][0]["is_error"], false);
    }

    #[test]
    fn app_server_file_change_item_and_patch_update_become_tool_events() {
        let mut state = AppServerBridgeState::default();
        let started = app_server_message_to_bridge_messages(
            &json!({
                "method": "item/started",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "startedAtMs": 1,
                    "item": {
                        "id": "patch_1",
                        "type": "fileChange",
                        "status": "inProgress",
                        "changes": []
                    }
                }
            }),
            &mut state,
        );
        let patch = app_server_message_to_bridge_messages(
            &json!({
                "method": "item/fileChange/patchUpdated",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "itemId": "patch_1",
                    "changes": [{
                        "path": "src/lib.rs",
                        "kind": { "type": "update" },
                        "diff": "@@ -1 +1 @@\n-old\n+new\n"
                    }]
                }
            }),
            &mut state,
        );

        assert_eq!(started.len(), 0);
        assert_eq!(patch.len(), 2);
        assert_eq!(patch[0]["message"]["content"][0]["name"], "Edit");
        assert_eq!(
            patch[0]["message"]["content"][0]["input"]["file_path"],
            "src/lib.rs"
        );
        assert_eq!(
            patch[0]["message"]["content"][0]["input"]["codex_file_change"],
            true
        );
        assert_eq!(
            patch[1]["message"]["content"][0]["tool_use_id"],
            "patch_1:src_lib.rs"
        );
        assert!(patch[1]["message"]["content"][0]["content"]
            .as_str()
            .unwrap()
            .contains("-old"));
    }

    #[test]
    fn app_server_command_output_delta_becomes_tool_result_delta() {
        let mut state = AppServerBridgeState::default();
        let messages = app_server_message_to_bridge_messages(
            &json!({
                "method": "item/commandExecution/outputDelta",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "itemId": "cmd_1",
                    "delta": "running\n"
                }
            }),
            &mut state,
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["type"], "user");
        assert_eq!(messages[0]["message"]["content"][0]["tool_use_id"], "cmd_1");
        assert_eq!(messages[0]["message"]["content"][0]["content"], "running\n");
    }

    #[test]
    fn app_server_turn_completed_emits_usage_result_and_turn_complete() {
        let mut state = AppServerBridgeState {
            thread_id: Some("thr_123".to_string()),
            turn_id: Some("turn_456".to_string()),
            latest_usage: None,
            ..Default::default()
        };
        let usage_messages = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/tokenUsage/updated",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "tokenUsage": {
                        "last": {
                            "inputTokens": 12,
                            "outputTokens": 34,
                            "cachedInputTokens": 0,
                            "reasoningOutputTokens": 0,
                            "totalTokens": 46
                        },
                        "total": {
                            "inputTokens": 12,
                            "outputTokens": 34,
                            "cachedInputTokens": 0,
                            "reasoningOutputTokens": 0,
                            "totalTokens": 46
                        },
                        "modelContextWindow": 200000
                    }
                }
            }),
            &mut state,
        );
        assert_eq!(usage_messages.len(), 1);
        assert_eq!(usage_messages[0]["type"], "result");
        assert_eq!(usage_messages[0]["modelUsage"]["codex"]["inputTokens"], 12);
        assert_eq!(usage_messages[0]["modelUsage"]["codex"]["outputTokens"], 34);
        assert_eq!(usage_messages[0]["modelUsage"]["codex"]["totalTokens"], 46);
        assert_eq!(
            usage_messages[0]["modelUsage"]["codex"]["contextWindowTokens"],
            200000
        );

        let messages = app_server_message_to_bridge_messages(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thr_123",
                    "turn": {
                        "id": "turn_456",
                        "items": [],
                        "status": "completed"
                    }
                }
            }),
            &mut state,
        );

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["type"], "result");
        assert_eq!(messages[0]["modelUsage"]["codex"]["inputTokens"], 12);
        assert_eq!(messages[0]["modelUsage"]["codex"]["outputTokens"], 34);
        assert_eq!(messages[0]["modelUsage"]["codex"]["totalTokens"], 46);
        assert_eq!(
            messages[0]["modelUsage"]["codex"]["contextWindowTokens"],
            200000
        );
        assert_eq!(messages[1]["type"], "turn_complete");
        assert_eq!(messages[1]["exit_code"], 0);
        assert!(state.turn_id.is_none());
    }

    #[test]
    fn app_server_account_notifications_are_ignored() {
        let mut state = AppServerBridgeState::default();
        let account = app_server_message_to_bridge_messages(
            &json!({
                "method": "account/updated",
                "params": {
                    "authMode": "chatgpt",
                    "planType": "pro"
                }
            }),
            &mut state,
        );
        assert!(account.is_empty());

        let limits = app_server_message_to_bridge_messages(
            &json!({
                "method": "account/rateLimits/updated",
                "params": {
                    "rateLimits": {
                        "limitName": "codex",
                        "primary": {
                            "usedPercent": 50
                        },
                        "secondary": {
                            "usedPercent": 25
                        }
                    }
                }
            }),
            &mut state,
        );
        assert!(limits.is_empty());
    }

    #[test]
    fn app_server_failed_turn_emits_error_and_nonzero_turn_complete() {
        let mut state = AppServerBridgeState::default();
        let messages = app_server_message_to_bridge_messages(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thr_123",
                    "turn": {
                        "id": "turn_456",
                        "items": [],
                        "status": "failed",
                        "error": { "message": "boom" }
                    }
                }
            }),
            &mut state,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0], json!({ "type": "error", "message": "boom" }));
        assert_eq!(messages[2]["type"], "turn_complete");
        assert_eq!(messages[2]["exit_code"], 1);
    }

    #[test]
    fn app_server_approval_request_becomes_permission_request() {
        let mut state = AppServerBridgeState::default();
        let messages = app_server_message_to_bridge_messages(
            &json!({
                "id": 99,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "itemId": "item_cmd",
                    "command": "pnpm test"
                }
            }),
            &mut state,
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["type"], "permission_request");
        assert_eq!(messages[0]["request_id"], "99");
        assert_eq!(messages[0]["tool_name"], "CodexCommand");
        assert_eq!(messages[0]["tool_use_id"], "item_cmd");
        assert_eq!(messages[0]["input"]["command"], "pnpm test");
    }

    #[test]
    fn app_server_approval_response_uses_stored_request_method() {
        let mut state = AppServerBridgeState::default();
        let messages = app_server_message_to_bridge_messages(
            &json!({
                "id": 99,
                "method": "item/fileChange/requestApproval",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "itemId": "patch_1"
                }
            }),
            &mut state,
        );
        assert_eq!(messages[0]["request_id"], "99");

        let response =
            build_app_server_permission_response_for_bridge_request(&mut state, "99", "deny", None)
                .expect("response");

        assert_eq!(
            response,
            json!({ "id": 99, "result": { "decision": "decline" } })
        );
        assert!(build_app_server_permission_response_for_bridge_request(
            &mut state, "99", "deny", None
        )
        .is_err());
    }

    #[test]
    fn command_approval_response_maps_allow_and_deny_to_app_server_decisions() {
        let allow =
            build_app_server_permission_response(10, REQUEST_COMMAND_APPROVAL, "allow", None)
                .expect("allow response");
        let deny = build_app_server_permission_response(11, REQUEST_COMMAND_APPROVAL, "deny", None)
            .expect("deny response");

        assert_eq!(
            allow,
            json!({ "id": 10, "result": { "decision": "accept" } })
        );
        assert_eq!(
            deny,
            json!({ "id": 11, "result": { "decision": "decline" } })
        );
    }

    #[test]
    fn file_change_approval_response_maps_allow_and_deny_to_app_server_decisions() {
        let allow =
            build_app_server_permission_response(12, REQUEST_FILE_CHANGE_APPROVAL, "allow", None)
                .expect("allow response");
        let deny =
            build_app_server_permission_response(13, REQUEST_FILE_CHANGE_APPROVAL, "deny", None)
                .expect("deny response");

        assert_eq!(
            allow,
            json!({ "id": 12, "result": { "decision": "accept" } })
        );
        assert_eq!(
            deny,
            json!({ "id": 13, "result": { "decision": "decline" } })
        );
    }

    #[test]
    fn permissions_approval_response_uses_permission_grant_or_denial_error() {
        let allow = build_app_server_permission_response(
            14,
            REQUEST_PERMISSIONS_APPROVAL,
            "allow",
            Some(r#"{"permissions":{"network":{"enabled":true},"fileSystem":null}}"#),
        )
        .expect("allow response");
        let deny =
            build_app_server_permission_response(15, REQUEST_PERMISSIONS_APPROVAL, "deny", None)
                .expect("deny response");

        assert_eq!(
            allow,
            json!({
                "id": 14,
                "result": {
                    "permissions": {
                        "network": { "enabled": true },
                        "fileSystem": null,
                    },
                    "scope": "turn",
                },
            })
        );
        assert_eq!(deny["id"], 15);
        assert_eq!(deny["error"]["code"], JSONRPC_ERROR_REQUEST_DENIED);
    }

    #[test]
    fn initialize_request_matches_app_server_handshake() {
        let value = build_initialize_request(1, "0.3.45");

        assert_eq!(value["id"], 1);
        assert_eq!(value["method"], METHOD_INITIALIZE);
        assert_eq!(value["params"]["clientInfo"]["name"], "releash");
        assert_eq!(value["params"]["clientInfo"]["title"], "Releash");
        assert_eq!(value["params"]["clientInfo"]["version"], "0.3.45");
        assert_eq!(value["params"]["capabilities"]["experimentalApi"], true);
        assert_eq!(value["params"]["capabilities"]["requestAttestation"], false);

        let initialized = build_initialized_notification();
        assert!(initialized.get("id").is_none());
        assert_eq!(initialized["method"], METHOD_INITIALIZED);
        assert_eq!(initialized["params"], json!({}));
    }

    #[test]
    fn thread_start_uses_codex_permission_and_workspace_fields() {
        let value = build_thread_start_request(
            2,
            "/repo",
            Some("gpt-5.3-codex"),
            Some("full"),
            false,
            None,
            Some("Follow repo rules."),
        )
        .expect("request");

        assert_eq!(value["method"], METHOD_THREAD_START);
        assert_eq!(value["params"]["cwd"], "/repo");
        assert_eq!(value["params"]["model"], "gpt-5.3-codex");
        assert_eq!(value["params"]["approvalPolicy"], "never");
        assert_eq!(value["params"]["sandbox"], "danger-full-access");
        assert_eq!(value["params"]["runtimeWorkspaceRoots"], json!(["/repo"]));
        assert_eq!(
            value["params"]["developerInstructions"],
            "Follow repo rules."
        );
        assert_eq!(value["params"]["threadSource"], "user");
    }

    #[test]
    fn thread_start_rejects_non_abstract_permission_modes() {
        let err = build_thread_start_request(
            2,
            "/repo",
            None,
            Some("bypassPermissions"),
            false,
            None,
            None,
        )
        .unwrap_err();
        assert!(err.contains("bypassPermissions"));
    }

    #[test]
    fn thread_start_can_use_named_permission_profile() {
        let value = build_thread_start_request(
            2,
            "/repo",
            None,
            Some("full"),
            false,
            Some(":read-only"),
            None,
        )
        .expect("request");

        assert_eq!(value["params"]["permissions"], ":read-only");
        assert!(value["params"].get("approvalPolicy").is_none());
        assert!(value["params"].get("sandbox").is_none());
    }

    #[test]
    fn thread_resume_uses_saved_thread_and_codex_runtime_fields() {
        let value = build_thread_resume_request(
            3,
            "thr_saved",
            "/repo",
            Some("gpt-5.3-codex"),
            Some("ask"),
            false,
            None,
            Some("Follow repo rules."),
        )
        .expect("request");

        assert_eq!(value["method"], METHOD_THREAD_RESUME);
        assert_eq!(value["params"]["threadId"], "thr_saved");
        assert_eq!(value["params"]["cwd"], "/repo");
        assert_eq!(value["params"]["model"], "gpt-5.3-codex");
        assert_eq!(value["params"]["approvalPolicy"], "on-request");
        assert_eq!(value["params"]["sandbox"], "read-only");
        assert_eq!(value["params"]["runtimeWorkspaceRoots"], json!(["/repo"]));
        assert_eq!(
            value["params"]["developerInstructions"],
            "Follow repo rules."
        );
    }

    #[test]
    fn thread_fork_uses_saved_thread_and_runtime_fields() {
        let value = build_thread_fork_request(
            3,
            "thr_saved",
            "/repo",
            Some("gpt-5.3-codex"),
            Some("edit"),
            false,
            None,
        )
        .expect("request");

        assert_eq!(value["method"], METHOD_THREAD_FORK);
        assert_eq!(value["params"]["threadId"], "thr_saved");
        assert_eq!(value["params"]["cwd"], "/repo");
        assert_eq!(value["params"]["model"], "gpt-5.3-codex");
        assert_eq!(value["params"]["approvalPolicy"], "on-request");
        assert_eq!(value["params"]["sandbox"], "workspace-write");
        assert_eq!(value["params"]["runtimeWorkspaceRoots"], json!(["/repo"]));
        assert_eq!(value["params"]["threadSource"], "user");
        assert_eq!(value["params"]["excludeTurns"], true);
    }

    #[test]
    fn thread_fork_can_use_named_permission_profile() {
        let value = build_thread_fork_request(
            3,
            "thr_saved",
            "/repo",
            None,
            Some("full"),
            false,
            Some(":team"),
        )
        .expect("request");

        assert_eq!(value["params"]["permissions"], ":team");
        assert!(value["params"].get("approvalPolicy").is_none());
        assert!(value["params"].get("sandbox").is_none());
    }

    #[test]
    fn thread_archive_requests_use_thread_id() {
        let archive = build_thread_archive_request(4, "thr_saved");
        let unarchive = build_thread_unarchive_request(5, "thr_saved");

        assert_eq!(archive["method"], METHOD_THREAD_ARCHIVE);
        assert_eq!(archive["params"]["threadId"], "thr_saved");
        assert_eq!(unarchive["method"], METHOD_THREAD_UNARCHIVE);
        assert_eq!(unarchive["params"]["threadId"], "thr_saved");
    }

    #[test]
    fn turn_start_uses_text_input_and_client_message_id() {
        let value = build_turn_start_request(
            3,
            "thr_123",
            "/repo",
            "Summarize this repo.",
            &[],
            Some("m1"),
            None,
        )
        .expect("request");

        assert_eq!(value["method"], METHOD_TURN_START);
        assert_eq!(value["params"]["threadId"], "thr_123");
        assert_eq!(value["params"]["cwd"], "/repo");
        assert_eq!(value["params"]["input"][0]["type"], "text");
        assert_eq!(value["params"]["input"][0]["text"], "Summarize this repo.");
        assert_eq!(value["params"]["clientUserMessageId"], "m1");
        assert_eq!(value["params"]["runtimeWorkspaceRoots"], json!(["/repo"]));
    }

    #[test]
    fn turn_start_uses_app_server_image_inputs() {
        let images = vec![ImageAttachment {
            data: "aGVsbG8=".to_string(),
            media_type: "image/png".to_string(),
        }];
        let value =
            build_turn_start_request(3, "thr_123", "/repo", "Check this", &images, None, None)
                .expect("request");

        assert_eq!(value["params"]["input"][0]["type"], "text");
        assert_eq!(value["params"]["input"][0]["text"], "Check this");
        assert_eq!(value["params"]["input"][1]["type"], "image");
        assert_eq!(
            value["params"]["input"][1]["url"],
            "data:image/png;base64,aGVsbG8="
        );
    }

    #[test]
    fn turn_start_allows_image_only_input() {
        let images = vec![ImageAttachment {
            data: "aGVsbG8=".to_string(),
            media_type: "image/png".to_string(),
        }];
        let value = build_turn_start_request(3, "thr_123", "/repo", "", &images, None, None)
            .expect("request");

        let input = value["params"]["input"].as_array().expect("input array");
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "image");
        assert_eq!(input[0]["url"], "data:image/png;base64,aGVsbG8=");
    }

    #[test]
    fn turn_start_can_update_permission_settings_for_turn() {
        let value = build_turn_start_request_with_permission(
            3,
            "thr_123",
            "/repo",
            "Use full access.",
            &[],
            None,
            None,
            Some("edit"),
            false,
            None,
        )
        .expect("request");

        assert_eq!(value["params"]["approvalPolicy"], "on-request");
        assert_eq!(value["params"]["sandboxPolicy"]["type"], "workspaceWrite");
        assert_eq!(
            value["params"]["sandboxPolicy"]["writableRoots"],
            json!(["/repo"])
        );
        assert_eq!(value["params"]["sandboxPolicy"]["networkAccess"], false);
    }

    #[test]
    fn turn_start_can_use_named_permission_profile() {
        let value = build_turn_start_request_with_permission(
            3,
            "thr_123",
            "/repo",
            "Use profile.",
            &[],
            None,
            None,
            Some("full"),
            false,
            Some(":read-only"),
        )
        .expect("request");

        assert_eq!(value["params"]["permissions"], ":read-only");
        assert!(value["params"].get("approvalPolicy").is_none());
        assert!(value["params"].get("sandboxPolicy").is_none());
    }

    #[test]
    fn thread_settings_update_uses_permission_schema() {
        let value =
            build_thread_settings_update_permission_request(8, "thr_123", "/repo", "ask", None)
                .expect("request");

        assert_eq!(value["method"], METHOD_THREAD_SETTINGS_UPDATE);
        assert_eq!(value["params"]["threadId"], "thr_123");
        assert_eq!(value["params"]["permissions"], Value::Null);
        assert_eq!(value["params"]["approvalPolicy"], "on-request");
        assert_eq!(value["params"]["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(value["params"]["sandboxPolicy"]["networkAccess"], false);
    }

    #[test]
    fn thread_settings_update_can_use_named_permission_profile() {
        let value = build_thread_settings_update_permission_request(
            8,
            "thr_123",
            "/repo",
            "ask",
            Some(":workspace"),
        )
        .expect("request");

        assert_eq!(value["method"], METHOD_THREAD_SETTINGS_UPDATE);
        assert_eq!(value["params"]["threadId"], "thr_123");
        assert_eq!(value["params"]["permissions"], ":workspace");
        assert!(value["params"].get("approvalPolicy").is_none());
        assert!(value["params"].get("sandboxPolicy").is_none());
    }

    #[test]
    fn turn_steer_uses_expected_turn_and_active_turn_input() {
        let value = build_turn_steer_request(
            4,
            "thr_123",
            "turn_456",
            "/repo",
            "/status",
            &[],
            Some("human_1"),
            None,
        );

        assert_eq!(value["method"], METHOD_TURN_STEER);
        assert_eq!(value["params"]["threadId"], "thr_123");
        assert_eq!(value["params"]["expectedTurnId"], "turn_456");
        assert_eq!(value["params"]["input"][0]["type"], "text");
        assert_eq!(value["params"]["input"][0]["text"], "/status");
        assert_eq!(value["params"]["clientUserMessageId"], "human_1");
        assert!(value["params"].get("cwd").is_none());
        assert!(value["params"].get("runtimeWorkspaceRoots").is_none());
    }

    #[test]
    fn model_list_request_uses_visible_picker_catalog() {
        let first = build_model_list_request(5, None);
        assert_eq!(first["method"], METHOD_MODEL_LIST);
        assert_eq!(first["params"]["includeHidden"], false);
        assert_eq!(first["params"]["limit"], 100);
        assert!(first["params"].get("cursor").is_none());

        let next = build_model_list_request(6, Some("cursor-1"));
        assert_eq!(next["params"]["cursor"], "cursor-1");
    }

    #[test]
    fn skills_list_request_uses_worktree_scope() {
        let value = build_skills_list_request(7, "/repo", false);

        assert_eq!(value["method"], METHOD_SKILLS_LIST);
        assert_eq!(value["params"]["cwds"], json!(["/repo"]));
        assert_eq!(value["params"]["forceReload"], false);
    }

    #[test]
    fn thread_history_requests_use_runtime_thread_pagination() {
        let list = build_thread_list_request(13, "/repo", None);
        assert_eq!(list["method"], METHOD_THREAD_LIST);
        assert_eq!(list["params"]["cwd"], "/repo");
        assert_eq!(list["params"]["archived"], false);
        assert_eq!(list["params"]["limit"], 20);
        assert_eq!(list["params"]["sortKey"], "updated_at");
        assert!(list["params"].get("cursor").is_none());

        let search = build_thread_search_request(14, "parser", Some("cursor-1"));
        assert_eq!(search["method"], METHOD_THREAD_SEARCH);
        assert_eq!(search["params"]["searchTerm"], "parser");
        assert_eq!(search["params"]["cursor"], "cursor-1");
    }

    #[test]
    fn thread_read_request_can_include_turn_history() {
        let value = build_thread_read_request(15, "thr_123", true);
        assert_eq!(value["method"], METHOD_THREAD_READ);
        assert_eq!(value["params"]["threadId"], "thr_123");
        assert_eq!(value["params"]["includeTurns"], true);
    }

    #[test]
    fn thread_turn_pagination_requests_use_full_items_view() {
        let turns = build_thread_turns_list_request(16, "thr_123", Some("turn-cursor"));
        assert_eq!(turns["method"], METHOD_THREAD_TURNS_LIST);
        assert_eq!(turns["params"]["threadId"], "thr_123");
        assert_eq!(turns["params"]["itemsView"], "full");
        assert_eq!(turns["params"]["limit"], 20);
        assert_eq!(turns["params"]["sortDirection"], "asc");
        assert_eq!(turns["params"]["cursor"], "turn-cursor");

        let items = build_thread_turn_items_list_request(17, "thr_123", "turn_1", None);
        assert_eq!(items["method"], METHOD_THREAD_TURNS_ITEMS_LIST);
        assert_eq!(items["params"]["threadId"], "thr_123");
        assert_eq!(items["params"]["turnId"], "turn_1");
        assert_eq!(items["params"]["limit"], 100);
        assert_eq!(items["params"]["sortDirection"], "asc");
        assert!(items["params"].get("cursor").is_none());
    }

    #[test]
    fn turn_start_includes_editor_additional_context() {
        let editor_context = AgentEditorContext {
            active_editor_path: Some("/repo/src/main.rs".to_string()),
            open_editor_paths: vec![
                "/repo/src/main.rs".to_string(),
                "/repo/src/lib.rs".to_string(),
                "/other/README.md".to_string(),
            ],
            selection: None,
        };
        let value = build_turn_start_request(
            3,
            "thr_123",
            "/repo",
            "Explain the active file.",
            &[],
            None,
            Some(&editor_context),
        )
        .expect("request");

        let context = &value["params"]["additionalContext"]["releash.ide"];
        assert_eq!(context["kind"], "untrusted");
        let text = context["value"].as_str().expect("context text");
        assert!(text.contains("Active editor: src/main.rs"));
        assert!(text.contains("- src/lib.rs"));
        assert!(text.contains("- /other/README.md"));
    }

    #[test]
    fn turn_start_includes_editor_selection_text_from_worktree_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).expect("src dir");
        let file_path = src_dir.join("main.rs");
        std::fs::write(
            &file_path,
            "fn main() {\n    println!(\"hello\");\n    println!(\"world\");\n}\n",
        )
        .expect("write file");
        let editor_context = AgentEditorContext {
            active_editor_path: Some(file_path.to_string_lossy().to_string()),
            open_editor_paths: vec![file_path.to_string_lossy().to_string()],
            selection: Some(
                crate::infrastructure::agent_session::runtime::AgentEditorSelection {
                    file_path: file_path.to_string_lossy().to_string(),
                    start_line: 2,
                    end_line: 3,
                },
            ),
        };
        let value = build_turn_start_request(
            3,
            "thr_123",
            &tmp.path().to_string_lossy(),
            "Explain the selected code.",
            &[],
            None,
            Some(&editor_context),
        )
        .expect("request");

        let text = value["params"]["additionalContext"]["releash.ide"]["value"]
            .as_str()
            .expect("context text");
        assert!(text.contains("Current selection: src/main.rs:L2-L3"));
        assert!(text.contains("Selected text:"));
        assert!(text.contains("println!(\"hello\")"));
        assert!(text.contains("println!(\"world\")"));
    }

    #[test]
    fn turn_interrupt_targets_thread_and_turn() {
        let value = build_turn_interrupt_request(4, "thr_123", "turn_456");

        assert_eq!(value["method"], METHOD_TURN_INTERRUPT);
        assert_eq!(value["params"]["threadId"], "thr_123");
        assert_eq!(value["params"]["turnId"], "turn_456");
    }

    #[test]
    fn thread_name_set_targets_thread() {
        let value = build_thread_name_set_request(6, "thr_123", "Review parser");

        assert_eq!(value["method"], METHOD_THREAD_NAME_SET);
        assert_eq!(value["params"]["threadId"], "thr_123");
        assert_eq!(value["params"]["name"], "Review parser");
    }

    #[test]
    fn app_server_realtime_transcript_notifications_are_ignored() {
        let mut state = AppServerBridgeState::default();
        let assistant_delta = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/realtime/transcript/delta",
                "params": {
                    "threadId": "thr_123",
                    "role": "assistant",
                    "delta": "Hello"
                }
            }),
            &mut state,
        );
        assert!(assistant_delta.is_empty());

        let user_delta = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/realtime/transcript/delta",
                "params": {
                    "threadId": "thr_123",
                    "role": "user",
                    "delta": "please review"
                }
            }),
            &mut state,
        );
        assert!(user_delta.is_empty());
    }

    #[test]
    fn app_server_realtime_lifecycle_notifications_are_ignored() {
        let mut state = AppServerBridgeState::default();
        let started = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/realtime/started",
                "params": {
                    "threadId": "thr_123",
                    "version": "v2",
                    "realtimeSessionId": "rt_456"
                }
            }),
            &mut state,
        );
        assert!(started.is_empty());

        let audio = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/realtime/outputAudio/delta",
                "params": {
                    "threadId": "thr_123",
                    "audio": {
                        "data": "AAEC",
                        "sampleRate": 24000,
                        "numChannels": 1,
                        "samplesPerChannel": 3
                    }
                }
            }),
            &mut state,
        );
        assert!(audio.is_empty());

        let error = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/realtime/error",
                "params": {
                    "threadId": "thr_123",
                    "message": "microphone failed"
                }
            }),
            &mut state,
        );
        assert!(error.is_empty());

        let closed = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/realtime/closed",
                "params": {
                    "threadId": "thr_123",
                    "reason": "completed"
                }
            }),
            &mut state,
        );
        assert!(closed.is_empty());
    }

    #[test]
    fn config_diagnostics_requests_match_schema() {
        let config = build_config_read_request(11, "/repo", true);
        assert_eq!(config["method"], METHOD_CONFIG_READ);
        assert_eq!(config["params"]["cwd"], "/repo");
        assert_eq!(config["params"]["includeLayers"], true);

        let requirements = build_config_requirements_read_request(12);
        assert_eq!(requirements["method"], METHOD_CONFIG_REQUIREMENTS_READ);
        assert!(requirements["params"].is_null());
    }

    #[test]
    fn fuzzy_file_search_request_matches_schema() {
        let value = build_fuzzy_file_search_request(13, "/repo", "main");

        assert_eq!(value["method"], METHOD_FUZZY_FILE_SEARCH);
        assert_eq!(value["params"]["query"], "main");
        assert_eq!(value["params"]["roots"], json!(["/repo"]));
        assert!(value["params"]["cancellationToken"].is_null());
    }

    #[test]
    fn runtime_inventory_requests_match_schema() {
        let capabilities = build_model_provider_capabilities_read_request(14);
        assert_eq!(
            capabilities["method"],
            METHOD_MODEL_PROVIDER_CAPABILITIES_READ
        );
        assert_eq!(capabilities["params"], json!({}));

        let collaboration_modes = build_collaboration_mode_list_request(15);
        assert_eq!(
            collaboration_modes["method"],
            METHOD_COLLABORATION_MODE_LIST
        );
        assert_eq!(collaboration_modes["params"], json!({}));

        let app_list = build_app_list_request(16, Some("thr_123"), Some("cursor-1"));
        assert_eq!(app_list["method"], METHOD_APP_LIST);
        assert_eq!(app_list["params"]["threadId"], "thr_123");
        assert_eq!(app_list["params"]["cursor"], "cursor-1");
        assert_eq!(app_list["params"]["limit"], 100);
        assert_eq!(app_list["params"]["forceRefetch"], false);

        let plugins = build_plugin_list_request(17, "/repo");
        assert_eq!(plugins["method"], METHOD_PLUGIN_LIST);
        assert_eq!(plugins["params"]["cwds"], json!(["/repo"]));
        assert_eq!(
            plugins["params"]["marketplaceKinds"],
            json!(["local", "workspace-directory"])
        );
    }

    #[test]
    fn app_server_thread_compacted_becomes_compaction_system_message() {
        let mut state = AppServerBridgeState::default();
        let messages = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/compacted",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_1"
                }
            }),
            &mut state,
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["type"], "system");
        assert_eq!(messages[0]["subtype"], "compact_boundary");
        assert_eq!(messages[0]["compact_metadata"]["trigger"], "manual");
    }

    #[test]
    fn app_server_thread_goal_notifications_are_ignored() {
        let mut state = AppServerBridgeState::default();
        let updated = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/goal/updated",
                "params": {
                    "threadId": "thr_123",
                    "goal": {
                        "objective": "Ship",
                        "status": "active",
                        "tokenBudget": 1000,
                        "tokensUsed": 10,
                        "timeUsedSeconds": 2
                    }
                }
            }),
            &mut state,
        );
        assert!(updated.is_empty());

        let cleared = app_server_message_to_bridge_messages(
            &json!({
                "method": "thread/goal/cleared",
                "params": {
                    "threadId": "thr_123"
                }
            }),
            &mut state,
        );
        assert!(cleared.is_empty());
    }
}
