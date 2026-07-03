//! Codex app-server wire contract verified against `codex-cli 0.139.0`
//! on 2026-07-02.
//!
//! Verification notes:
//! - `codex --version` reported `codex-cli 0.139.0`.
//! - `initialize` with `clientInfo { name: "releash", title: "Releash", version }`
//!   and `capabilities { experimentalApi: true, requestAttestation: false }`
//!   returned `userAgent: "Codex Desktop/0.139.0 ..."` and a temp `codexHome`
//!   when HOME/CODEX_HOME were pointed at `/tmp`.
//! - `thread/start` accepted plan mode as `collaborationMode: "plan"`.
//!   The struct form `{ "mode": "plan", "settings": {} }` was also accepted
//!   by 0.139.0. Releash keeps the string form for compatibility with the
//!   existing shipped integration and because it is sufficient for this target.
//! - A network-failed `turn/start` emitted `error` notifications plus
//!   `turn/started`/`item/started` events; it did not emit an `item/completed`
//!   `error` item. Binary protocol strings for `TurnItem` variants list
//!   UserMessage/HookPrompt/AgentMessage/Reasoning/WebSearch/ImageView/
//!   ImageGeneration/FileChange/McpToolCall/ContextCompaction, so legacy
//!   `todo_list` and `error` item conversion is not part of the 0.139.0
//!   app-server contract.
//! - User input server request method is `item/tool/requestUserInput`; the
//!   dynamic tool name remains `request_user_input`.
//! - Compaction (`thread/compact/start`) emitted `item/started` /
//!   `item/completed` with `{ "type": "contextCompaction" }` items inside a
//!   dedicated turn; the deprecated `thread/compacted` notification was not
//!   emitted by 0.139.0. A failed compaction surfaces as an `error`
//!   notification (`params.error.message`) followed by `turn/completed` with
//!   `turn.status: "failed"` carrying the same `turn.error.message`.
//! - `thread/settings/update` accepted
//!   `{ threadId, permissions: null, approvalPolicy, sandboxPolicy }`.

use serde_json::{json, Value};

pub(crate) const METHOD_INITIALIZE: &str = "initialize";
pub(crate) const METHOD_INITIALIZED: &str = "initialized";
pub(crate) const METHOD_THREAD_START: &str = "thread/start";
pub(crate) const METHOD_THREAD_RESUME: &str = "thread/resume";
pub(crate) const METHOD_THREAD_FORK: &str = "thread/fork";
pub(crate) const METHOD_THREAD_ARCHIVE: &str = "thread/archive";
pub(crate) const METHOD_THREAD_UNARCHIVE: &str = "thread/unarchive";
pub(crate) const METHOD_THREAD_NAME_SET: &str = "thread/name/set";
pub(crate) const METHOD_THREAD_SETTINGS_UPDATE: &str = "thread/settings/update";
pub(crate) const METHOD_TURN_START: &str = "turn/start";
#[allow(dead_code)] // issues-1301 D16/F-2: steering remains capability-gated and unused for current Codex backend behavior.
pub(crate) const METHOD_TURN_STEER: &str = "turn/steer";
pub(crate) const METHOD_TURN_INTERRUPT: &str = "turn/interrupt";
pub(crate) const METHOD_SKILLS_LIST: &str = "skills/list";
pub(crate) const METHOD_FUZZY_FILE_SEARCH: &str = "fuzzyFileSearch";

pub(crate) const NOTIFY_THREAD_STARTED: &str = "thread/started";
pub(crate) const NOTIFY_THREAD_COMPACTED: &str = "thread/compacted";
pub(crate) const NOTIFY_THREAD_TOKEN_USAGE_UPDATED: &str = "thread/tokenUsage/updated";
pub(crate) const NOTIFY_TURN_STARTED: &str = "turn/started";
pub(crate) const NOTIFY_TURN_COMPLETED: &str = "turn/completed";
pub(crate) const NOTIFY_ITEM_STARTED: &str = "item/started";
pub(crate) const NOTIFY_ITEM_COMPLETED: &str = "item/completed";
pub(crate) const NOTIFY_AGENT_MESSAGE_DELTA: &str = "item/agentMessage/delta";
pub(crate) const NOTIFY_COMMAND_OUTPUT_DELTA: &str = "item/commandExecution/outputDelta";
pub(crate) const NOTIFY_FILE_CHANGE_OUTPUT_DELTA: &str = "item/fileChange/outputDelta";
pub(crate) const NOTIFY_FILE_CHANGE_PATCH_UPDATED: &str = "item/fileChange/patchUpdated";
pub(crate) const NOTIFY_ERROR: &str = "error";

pub(crate) const REQUEST_COMMAND_APPROVAL: &str = "item/commandExecution/requestApproval";
pub(crate) const REQUEST_FILE_CHANGE_APPROVAL: &str = "item/fileChange/requestApproval";
pub(crate) const REQUEST_TOOL_USER_INPUT: &str = "item/tool/requestUserInput";
pub(crate) const REQUEST_PERMISSIONS_APPROVAL: &str = "item/permissions/requestApproval";

// Codex app-server treats any JSON-RPC error response to permission requests as
// a denial. The protocol does not reserve a dedicated decline code, so Releash
// standardizes on -32001 for denied requests while keeping the semantic meaning
// local to this backend contract.
pub(crate) const JSONRPC_ERROR_REQUEST_DENIED: i64 = -32001;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppServerMessageKind {
    Response { id: u64 },
    Request { id: u64, method: String },
    Notification { method: String },
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

pub(crate) fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "id": id, "method": method, "params": params })
}

pub(crate) fn notification(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": params })
}

pub(crate) fn initialize_request(id: u64, version: &str) -> Value {
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

pub(crate) fn initialized_notification() -> Value {
    notification(METHOD_INITIALIZED, json!({}))
}

pub(crate) fn is_user_input_request_method(method: &str) -> bool {
    method == REQUEST_TOOL_USER_INPUT
}
