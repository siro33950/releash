use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::backends::bridge_common::{
    start_agent_session_internal, start_agent_turn, write_bridge_command, AgentProcessMap,
    CODEX_BACKEND_ID,
};
use crate::backends::process_io::run_command_with_output_limit;
use crate::backends::{
    AgentBackend, AgentMessage, BackendRuntimeConfig, PermissionResponse, SessionConfig,
    SessionHandle,
};
use crate::session::{resolve_data_dir, SessionStore};

/// `codex debug models` の取得タイムアウト。長くなる場合は CLI 側の問題として扱う。
const CODEX_FETCH_MODELS_TIMEOUT: Duration = Duration::from_secs(10);

/// `codex debug models` の stdout/stderr の取り込み上限（バイト）。
/// CLI が暴走したり巨大な出力を返した場合のメモリ・ログ肥大化を防ぐ。
const CODEX_OUTPUT_MAX_BYTES: usize = 1024 * 1024;

/// stderr ログに残す要約バイト数（末尾）。機密情報の偶発的な漏えいを抑制する。
const CODEX_STDERR_LOG_SUMMARY_BYTES: usize = 512;

/// Codex SDK Bridge バックエンド。
/// 実際のプロセス制御は AgentProcess bridge runtime に委譲する。
pub struct CodexBackend {
    #[allow(dead_code)]
    runtime: Option<Arc<dyn CodexBackendRuntime>>,
    cli_path: Option<String>,
}

/// `codex debug models` の標準出力（JSON カタログ）をパースし、`models[].slug` を抽出する。
/// 出力が JSON として不正、または `models` 配列が欠落している場合は Err を返す。
/// 値は加工せずそのまま返し、検証は呼び出し側に委ねる（all-or-nothing）。
pub(crate) fn parse_codex_models_output(stdout: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("codex debug models の JSON 解析に失敗: {e}"))?;
    let arr = value
        .get("models")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "codex debug models 出力に 'models' 配列がありません".to_string())?;

    let mut result = Vec::with_capacity(arr.len());
    for (idx, entry) in arr.iter().enumerate() {
        let slug = entry.get("slug").and_then(|v| v.as_str()).ok_or_else(|| {
            format!("codex debug models 出力の models[{idx}] に 'slug' がありません")
        })?;
        result.push(slug.to_string());
    }
    Ok(result)
}

/// stderr ログ要約: 末尾 N バイトのみ、制御文字を除去し、機密情報候補を redaction する。
/// 通常運用ログでは `error_message_for_failure` 経由で固定メッセージのみ出すこと。
/// 詳細出力はデバッグ用途に限定し、API key・Bearer/Authorization トークン等は伏字化する。
fn summarize_stderr(bytes: &[u8]) -> String {
    let start = bytes.len().saturating_sub(CODEX_STDERR_LOG_SUMMARY_BYTES);
    let tail = &bytes[start..];
    let s = String::from_utf8_lossy(tail);
    let sanitized: String = s
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                c
            } else if c.is_control() {
                '?'
            } else {
                c
            }
        })
        .collect();
    let redacted = redact_secrets(&sanitized);
    if start > 0 {
        format!("[...truncated...]{redacted}")
    } else {
        redacted
    }
}

/// 機密情報候補を伏字化する。
/// - `Authorization: <scheme> <token>` / `Bearer <token>`
/// - `api[_-]?key`, `secret`, `token`, `password` 等のキー直後の値
/// - 長めの英数字混在トークン（24 文字以上）
fn redact_secrets(s: &str) -> String {
    use std::sync::OnceLock;
    static PATTERNS: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            // Authorization ヘッダ全体（scheme + value）
            regex::RegexBuilder::new(r#"(?i)authorization\s*[:=]\s*\S+(\s+\S+)?"#)
                .build()
                .unwrap(),
            // Bearer / Basic などスキーム付きトークン
            regex::RegexBuilder::new(r#"(?i)\b(?:bearer|basic|token)\s+[A-Za-z0-9._\-/+=]{8,}"#)
                .build()
                .unwrap(),
            // api_key / api-key / apikey / secret / password / token = VALUE
            regex::RegexBuilder::new(
                r#"(?i)\b(?:api[_-]?key|secret(?:[_-]?key)?|password|access[_-]?token|refresh[_-]?token|token)\b\s*[:=]\s*['"]?[^\s'"]+"#,
            )
            .build()
            .unwrap(),
            // 単独で現れる長い英数字混在トークン（誤検知許容）
            regex::RegexBuilder::new(r#"\b(?:sk|pk|tok|key|sess|bearer)[_-][A-Za-z0-9_\-]{16,}"#)
                .case_insensitive(true)
                .build()
                .unwrap(),
        ]
    });
    let mut out = s.to_string();
    for re in patterns {
        out = re.replace_all(&out, "[REDACTED]").into_owned();
    }
    out
}

/// 失敗時の通常ログ用メッセージ（exit status と固定文言のみ）。
/// stderr 本文は debug! へ降格して出力するため、ここでは含めない。
fn error_message_for_failure(status_code: Option<i32>) -> String {
    format!(
        "codex debug models が失敗しました (status={:?})",
        status_code
    )
}

pub(crate) fn configured_cli_path(app: &tauri::AppHandle) -> Option<String> {
    app.try_state::<std::sync::Arc<crate::config::AppConfig>>()
        .and_then(|cfg_state| cfg_state.get_config().ok())
        .and_then(|cfg| cfg.agents.codex.cli_path)
        .filter(|path| !path.trim().is_empty())
}

#[allow(dead_code)]
impl CodexBackend {
    pub fn new() -> Self {
        Self {
            runtime: None,
            cli_path: None,
        }
    }

    pub fn with_agent_process_runtime(
        app: AppHandle,
        handles: Arc<Mutex<AgentProcessMap>>,
        session_store: Arc<SessionStore>,
    ) -> Self {
        let cli_path = configured_cli_path(&app);
        Self {
            runtime: Some(Arc::new(AgentProcessCodexRuntime {
                app,
                handles,
                session_store,
            })),
            cli_path,
        }
    }

    fn runtime(&self) -> Result<Arc<dyn CodexBackendRuntime>, String> {
        self.runtime.clone().ok_or_else(|| {
            "CodexBackend runtime is not attached; build the registry with app runtime".to_string()
        })
    }

    fn cli_path(&self) -> String {
        self.cli_path.clone().unwrap_or_else(|| "codex".to_string())
    }
}

#[allow(dead_code)]
#[async_trait]
trait CodexBackendRuntime: Send + Sync {
    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String>;
    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String>;
    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String>;
    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String>;
    async fn close_session(&self, session: &SessionHandle) -> Result<(), String>;
}

#[allow(dead_code)]
struct AgentProcessCodexRuntime {
    app: AppHandle,
    handles: Arc<Mutex<AgentProcessMap>>,
    session_store: Arc<SessionStore>,
}

#[async_trait]
impl CodexBackendRuntime for AgentProcessCodexRuntime {
    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
        start_agent_session_internal(
            &self.app,
            &self.handles,
            &self.session_store,
            &config.chat_session_id,
            &config.cwd,
            config.permission_mode,
            config.system_prompt,
        )
        .await?;

        Ok(SessionHandle {
            chat_session_id: config.chat_session_id,
            backend_id: CODEX_BACKEND_ID.to_string(),
        })
    }

    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String> {
        ensure_codex_session(session)?;
        let data_dir = resolve_data_dir(&self.app)?;
        let stored_session = self
            .session_store
            .get_session(&data_dir, &session.chat_session_id)?
            .ok_or_else(|| format!("Session not found: {}", session.chat_session_id))?;

        start_agent_turn(
            &self.app,
            &self.handles,
            &self.session_store,
            &session.chat_session_id,
            &stored_session.worktree_path,
            &message.permission_mode,
            &message.content,
            &message.streaming_message_id,
            &message.images,
        )
        .await
    }

    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String> {
        ensure_codex_session(session)?;
        write_bridge_command(
            &self.handles,
            &session.chat_session_id,
            json!({"type": "interrupt"}),
        )
        .await
    }

    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String> {
        ensure_codex_session(session)?;
        if response.behavior != "allow" && response.behavior != "deny" {
            return Err(format!("Invalid behavior: {}", response.behavior));
        }

        let mut result = json!({ "behavior": response.behavior });
        if let Some(message) = response.message {
            result["message"] = serde_json::Value::String(message);
        }
        if let Some(updated_input) = response.updated_input {
            match serde_json::from_str::<serde_json::Value>(&updated_input) {
                Ok(parsed) => result["updatedInput"] = parsed,
                Err(e) => log::warn!("Failed to parse updated_input JSON: {e}"),
            }
        }

        write_bridge_command(
            &self.handles,
            &session.chat_session_id,
            json!({
                "type": "permission_response",
                "request_id": response.request_id,
                "result": result,
            }),
        )
        .await
    }

    async fn close_session(&self, session: &SessionHandle) -> Result<(), String> {
        ensure_codex_session(session)?;
        write_bridge_command(
            &self.handles,
            &session.chat_session_id,
            json!({"type": "close"}),
        )
        .await
    }
}

#[allow(dead_code)]
fn ensure_codex_session(session: &SessionHandle) -> Result<(), String> {
    if session.backend_id == CODEX_BACKEND_ID {
        return Ok(());
    }
    Err(format!(
        "Session {} belongs to backend {}, not {}",
        session.chat_session_id, session.backend_id, CODEX_BACKEND_ID
    ))
}

#[async_trait]
impl AgentBackend for CodexBackend {
    fn id(&self) -> &str {
        CODEX_BACKEND_ID
    }

    fn name(&self) -> &str {
        "Codex"
    }

    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
        self.runtime()?.start_session(config).await
    }

    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String> {
        self.runtime()?.send_message(session, message).await
    }

    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String> {
        self.runtime()?.interrupt(session).await
    }

    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String> {
        self.runtime()?.respond_permission(session, response).await
    }

    async fn fetch_models_from_cli(&self) -> Result<Vec<String>, String> {
        let cli = self.cli_path();
        let mut cmd = Command::new(&cli);
        cmd.arg("debug").arg("models");

        let output = run_command_with_output_limit(
            cmd,
            CODEX_FETCH_MODELS_TIMEOUT,
            CODEX_OUTPUT_MAX_BYTES,
            "codex CLI",
            &format!("codex CLI 起動失敗 ({cli})"),
            "codex CLI 待機失敗",
            format!(
                "codex CLI 取得タイムアウト ({} 秒)",
                CODEX_FETCH_MODELS_TIMEOUT.as_secs()
            ),
        )
        .await?;

        if !output.status.success() {
            // 通常ログは exit status と固定メッセージのみ。stderr 詳細は debug! 限定。
            let message = error_message_for_failure(output.status.code());
            log::debug!("{}: stderr={}", message, summarize_stderr(&output.stderr));
            return Err(message);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let models = parse_codex_models_output(&stdout)?;
        if models.is_empty() {
            return Err("codex debug models の出力が空でした".to_string());
        }
        Ok(models)
    }

    fn runtime_config(&self, app: &tauri::AppHandle) -> BackendRuntimeConfig {
        let mut bridge_init_options = serde_json::Map::new();
        bridge_init_options.insert(
            "codexCliPath".to_string(),
            serde_json::Value::String(
                configured_cli_path(app).unwrap_or_else(|| "codex".to_string()),
            ),
        );

        BackendRuntimeConfig {
            bridge_init_options,
        }
    }

    async fn close_session(&self, session: &SessionHandle) -> Result<(), String> {
        self.runtime()?.close_session(session).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codex_models_output_extracts_slugs_from_json_catalog() {
        let stdout = r#"{"models":[{"slug":"gpt-5.5","display_name":"GPT-5.5"},{"slug":"o3","display_name":"o3"}]}"#;
        let parsed = parse_codex_models_output(stdout).unwrap();
        assert_eq!(parsed, vec!["gpt-5.5".to_string(), "o3".to_string()]);
    }

    #[test]
    fn parse_codex_models_output_does_not_normalize_slug() {
        // CLI 由来でも値は加工せずそのまま返す（trim 等の正規化禁止）。
        let stdout = r#"{"models":[{"slug":"  spaced-slug  "}]}"#;
        let parsed = parse_codex_models_output(stdout).unwrap();
        assert_eq!(parsed, vec!["  spaced-slug  ".to_string()]);
    }

    #[test]
    fn parse_codex_models_output_rejects_invalid_json() {
        assert!(parse_codex_models_output("not json").is_err());
    }

    #[test]
    fn parse_codex_models_output_rejects_missing_models_array() {
        assert!(parse_codex_models_output(r#"{"foo":"bar"}"#).is_err());
    }

    #[test]
    fn parse_codex_models_output_rejects_entry_without_slug() {
        // models[].slug が欠落していた場合は all-or-nothing で拒否。
        assert!(
            parse_codex_models_output(r#"{"models":[{"slug":"a"},{"display_name":"b"}]}"#).is_err()
        );
    }

    #[test]
    fn summarize_stderr_truncates_and_sanitizes_control_chars() {
        let mut input = Vec::new();
        input.extend(std::iter::repeat_n(b'a', 1024));
        input.extend_from_slice(b"\x01tail");
        let s = summarize_stderr(&input);
        assert!(s.starts_with("[...truncated...]"));
        assert!(s.contains("?tail"));
    }

    #[test]
    fn redact_secrets_redacts_authorization_header() {
        let s = "request failed: Authorization: Bearer abcdef1234567890XYZ";
        let redacted = redact_secrets(s);
        assert!(!redacted.contains("abcdef1234567890XYZ"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn redact_secrets_redacts_bearer_token_alone() {
        let s = "got token bearer abcDEF1234567890";
        let redacted = redact_secrets(s);
        assert!(!redacted.contains("abcDEF1234567890"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn redact_secrets_redacts_api_key_value() {
        let s = "api_key=sk_live_supersecretvalue123";
        let redacted = redact_secrets(s);
        assert!(!redacted.contains("supersecretvalue123"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn redact_secrets_redacts_secret_assignment() {
        let s = "secret: \"top-secret-value-1234\"";
        let redacted = redact_secrets(s);
        assert!(!redacted.contains("top-secret-value-1234"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn summarize_stderr_redacts_authorization() {
        let bytes = b"Authorization: Bearer abcdef1234567890SECRET";
        let s = summarize_stderr(bytes);
        assert!(!s.contains("abcdef1234567890SECRET"));
        assert!(s.contains("[REDACTED]"));
    }

    #[test]
    fn error_message_for_failure_does_not_include_stderr() {
        let msg = error_message_for_failure(Some(1));
        assert!(msg.contains("status="));
        // 通常ログ用メッセージには stderr 由来の文字列を含めない
        assert!(!msg.to_lowercase().contains("bearer"));
        assert!(!msg.to_lowercase().contains("authorization"));
    }

    #[cfg(unix)]
    fn write_codex_cli_script(body: &str) -> (tempfile::TempDir, String) {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("codex-test-cli");
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path.to_string_lossy().into_owned())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fetch_models_from_cli_runs_debug_models_and_returns_stdout_models() {
        let (_dir, cli_path) = write_codex_cli_script(
            r#"#!/bin/sh
if [ "$1" != "debug" ] || [ "$2" != "models" ]; then
  echo "unexpected args: $*" >&2
  exit 42
fi
printf '{"models":[{"slug":"gpt-5.5"},{"slug":"o3"}]}'
"#,
        );
        let mut backend = CodexBackend::new();
        backend.cli_path = Some(cli_path);

        let models = backend.fetch_models_from_cli().await.unwrap();

        assert_eq!(models, vec!["gpt-5.5".to_string(), "o3".to_string()]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fetch_models_from_cli_errors_on_nonzero_exit() {
        let (_dir, cli_path) = write_codex_cli_script(
            r#"#!/bin/sh
echo "boom" >&2
exit 7
"#,
        );
        let mut backend = CodexBackend::new();
        backend.cli_path = Some(cli_path);

        let err = backend.fetch_models_from_cli().await.unwrap_err();

        assert!(err.contains("status"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fetch_models_from_cli_rejects_empty_models() {
        let (_dir, cli_path) = write_codex_cli_script(
            r#"#!/bin/sh
printf '{"models":[]}'
"#,
        );
        let mut backend = CodexBackend::new();
        backend.cli_path = Some(cli_path);

        let err = backend.fetch_models_from_cli().await.unwrap_err();

        assert!(err.contains("空"));
    }

    #[tokio::test]
    async fn fetch_models_from_cli_errors_when_cli_missing() {
        let mut backend = CodexBackend::new();
        backend.cli_path = Some("/nonexistent-codex-binary-for-test".to_string());
        let result = backend.fetch_models_from_cli().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn runtime_methods_require_attached_runtime() {
        let backend = CodexBackend::new();
        let session = SessionHandle {
            chat_session_id: "session-1".to_string(),
            backend_id: CODEX_BACKEND_ID.to_string(),
        };
        let message = AgentMessage {
            content: "hello".to_string(),
            streaming_message_id: "message-1".to_string(),
            images: vec![],
            permission_mode: "acceptEdits".to_string(),
        };

        assert!(backend.send_message(&session, message).await.is_err());
        assert!(backend.interrupt(&session).await.is_err());
        assert!(backend.close_session(&session).await.is_err());
    }
}
