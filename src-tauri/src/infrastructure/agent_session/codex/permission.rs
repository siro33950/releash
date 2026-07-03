use serde_json::{json, Value};

use crate::domain::agent_session::entities::{PermissionResponse, PermissionResponseDecision};
use crate::domain::agent_session::value_objects::{JsonPayload, PermissionMode};

use super::wire::JSONRPC_ERROR_REQUEST_DENIED;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CodexPermissionSettings {
    pub approval_policy: Option<&'static str>,
    pub sandbox_policy: Option<Value>,
    pub permissions: Option<Value>,
}

pub(crate) fn codex_approval_policy(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask | PermissionMode::Edit => "on-request",
        PermissionMode::Full => "never",
    }
}

pub(crate) fn codex_sandbox_policy(mode: PermissionMode, cwd: &str) -> Value {
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

pub(crate) fn codex_permission_settings(
    mode: PermissionMode,
    plan_mode: bool,
    permission_profile_id: Option<&str>,
    cwd: &str,
) -> CodexPermissionSettings {
    if let Some(profile_id) = permission_profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return CodexPermissionSettings {
            approval_policy: None,
            sandbox_policy: None,
            permissions: Some(Value::String(profile_id.to_string())),
        };
    }
    if plan_mode {
        return CodexPermissionSettings {
            approval_policy: Some("on-request"),
            sandbox_policy: Some(codex_sandbox_policy(PermissionMode::Ask, cwd)),
            permissions: None,
        };
    }
    CodexPermissionSettings {
        approval_policy: Some(codex_approval_policy(mode)),
        sandbox_policy: Some(codex_sandbox_policy(mode, cwd)),
        permissions: None,
    }
}

pub(crate) fn codex_permission_response(
    jsonrpc_id: u64,
    source_method: &str,
    response: PermissionResponse,
) -> Value {
    match response.decision {
        PermissionResponseDecision::Allow {
            updated_input,
            answers,
        } => allow_response(jsonrpc_id, source_method, updated_input, answers),
        PermissionResponseDecision::Deny { message } => {
            deny_response(jsonrpc_id, source_method, message)
        }
    }
}

fn allow_response(
    jsonrpc_id: u64,
    source_method: &str,
    updated_input: Option<JsonPayload>,
    answers: Option<JsonPayload>,
) -> Value {
    if source_method == super::wire::REQUEST_PERMISSIONS_APPROVAL {
        let permissions = updated_input
            .and_then(|payload| serde_json::from_str::<Value>(payload.as_str()).ok())
            .and_then(|value| value.get("permissions").cloned().or(Some(value)))
            .unwrap_or_else(|| json!({ "fileSystem": null, "network": null }));
        return json!({
            "id": jsonrpc_id,
            "result": {
                "permissions": permissions,
                "scope": "turn",
            },
        });
    }
    if super::wire::is_user_input_request_method(source_method) {
        let answers = answers
            .and_then(|payload| serde_json::from_str::<Value>(payload.as_str()).ok())
            .unwrap_or(Value::Null);
        return json!({
            "id": jsonrpc_id,
            "result": { "answers": answers },
        });
    }
    json!({
        "id": jsonrpc_id,
        "result": { "decision": "accept" },
    })
}

fn deny_response(jsonrpc_id: u64, source_method: &str, message: Option<String>) -> Value {
    if source_method == super::wire::REQUEST_COMMAND_APPROVAL
        || source_method == super::wire::REQUEST_FILE_CHANGE_APPROVAL
    {
        return json!({
            "id": jsonrpc_id,
            "result": { "decision": "decline" },
        });
    }
    json!({
        "id": jsonrpc_id,
        "error": {
            "code": JSONRPC_ERROR_REQUEST_DENIED,
            "message": message.unwrap_or_else(|| "User denied request".to_string()),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow(updated_input: Option<&str>, answers: Option<&str>) -> PermissionResponse {
        PermissionResponse {
            request_id: "req-1".to_string(),
            decision: PermissionResponseDecision::Allow {
                updated_input: updated_input
                    .map(|value| JsonPayload::new_unchecked(value.to_string())),
                answers: answers.map(|value| JsonPayload::new_unchecked(value.to_string())),
            },
        }
    }

    #[test]
    fn test_codex_permission_settings_plan_modeはread_only_on_request() {
        let settings = codex_permission_settings(PermissionMode::Edit, true, None, "/repo");

        assert_eq!(settings.approval_policy, Some("on-request"));
        assert_eq!(settings.sandbox_policy.unwrap()["type"], "readOnly");
        assert!(settings.permissions.is_none());
    }

    #[test]
    fn test_codex_permission_settings_profile指定時はmode変換しない() {
        let settings =
            codex_permission_settings(PermissionMode::Full, false, Some(":team"), "/repo");

        assert_eq!(
            settings.permissions,
            Some(Value::String(":team".to_string()))
        );
        assert!(settings.approval_policy.is_none());
        assert!(settings.sandbox_policy.is_none());
    }

    #[test]
    fn test_codex_permission_response_command_accept_decline() {
        let accepted = codex_permission_response(
            7,
            super::super::wire::REQUEST_COMMAND_APPROVAL,
            allow(None, None),
        );
        assert_eq!(
            accepted,
            json!({ "id": 7, "result": { "decision": "accept" } })
        );

        let denied = codex_permission_response(
            8,
            super::super::wire::REQUEST_COMMAND_APPROVAL,
            PermissionResponse {
                request_id: "req-1".to_string(),
                decision: PermissionResponseDecision::Deny {
                    message: Some("no".to_string()),
                },
            },
        );
        assert_eq!(
            denied,
            json!({ "id": 8, "result": { "decision": "decline" } })
        );
    }

    #[test]
    fn test_codex_permission_response_permissions_fallback() {
        let value = codex_permission_response(
            9,
            super::super::wire::REQUEST_PERMISSIONS_APPROVAL,
            allow(None, None),
        );

        assert_eq!(value["result"]["scope"], "turn");
        assert_eq!(value["result"]["permissions"]["fileSystem"], Value::Null);
        assert_eq!(value["result"]["permissions"]["network"], Value::Null);
    }

    #[test]
    fn test_codex_permission_response_user_input_answers() {
        let value = codex_permission_response(
            10,
            super::super::wire::REQUEST_TOOL_USER_INPUT,
            allow(None, Some(r#"{"q1":"yes"}"#)),
        );

        assert_eq!(value["result"]["answers"]["q1"], "yes");
    }
}
