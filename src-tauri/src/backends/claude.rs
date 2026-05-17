use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Duration;
use tauri::Manager;
use tokio::process::Command;

use crate::backends::bridge_common::CLAUDE_BACKEND_ID;
use crate::backends::process_io::run_command_with_output_limit;
use crate::backends::{
    AgentBackend, AgentMessage, BackendRuntimeConfig, PermissionResponse, SessionConfig,
    SessionHandle,
};

/// 起動時 Claude モデル一覧 probe のタイムアウト。SDK 側で 8 秒のタイムアウトを
/// 持っているため、外側はそれより少し長めに取って包む。
const CLAUDE_PROBE_TIMEOUT: Duration = Duration::from_secs(12);

/// probe スクリプトの stdout/stderr の取り込み上限（バイト）。
const CLAUDE_PROBE_OUTPUT_MAX_BYTES: usize = 1024 * 1024;

/// Claude Agent SDK Bridge バックエンド。
/// Node.js ブリッジプロセスを経由して Claude Agent SDK と通信する。
pub struct ClaudeBackend {
    /// 起動時 probe で使う `claude-sdk-bridge-list-models` スクリプトの解決結果。
    /// AppHandle が無い構成（テスト等）では `None` を保持し、CLI fetch を「サポートしない」
    /// 扱いとして起動時同期から skip する。
    /// 実アプリ（`with_app`）では Result を保持し、解決失敗（resource_dir 失敗等）は
    /// infrastructure failure として `fetch_models_from_cli` で Err を返す。
    list_models_script: Option<Result<PathBuf, String>>,
}

impl ClaudeBackend {
    pub fn new() -> Self {
        Self {
            list_models_script: None,
        }
    }

    /// AppHandle 経由で probe スクリプトのパスを解決して保持する。
    /// 解決失敗は capability ではなく infrastructure failure として保持し、
    /// `fetch_models_from_cli` から Err を返すことで起動時 refresh の warn 経路に乗せる。
    pub fn with_app(app: &tauri::AppHandle) -> Self {
        Self {
            list_models_script: Some(resolve_claude_list_models_script(app)),
        }
    }
}

/// Claude 起動時モデル一覧 probe 用スクリプト名（dev / production）。
/// `claude-sdk-bridge.mjs` の通常ブリッジとは分離しており、initializationResult のみ
/// を返して即時終了する一発スクリプト。
pub(crate) fn claude_list_models_script_names() -> (&'static str, &'static str) {
    (
        "claude-sdk-bridge-list-models.mjs",
        "generated/bridges/claude-sdk-bridge-list-models.bundled.mjs",
    )
}

/// Claude 起動時モデル一覧 probe スクリプトのパスを解決する。
/// dev 環境では `src-tauri/resources/` を優先、production では resource_dir を参照。
pub(crate) fn resolve_claude_list_models_script(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let (dev_name, bundled_name) = claude_list_models_script_names();
    #[cfg(debug_assertions)]
    {
        let dev_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(dev_name);
        if dev_path.exists() {
            return Ok(dev_path);
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = dev_name;

    app.path()
        .resource_dir()
        .map(|d| d.join(bundled_name))
        .map_err(|e| format!("resource_dir 解決失敗: {e}"))
}

/// probe スクリプトの stdout を解析して `models[].value` のリストを抽出する。
/// 出力フォーマットは `{"models":[{"value":"..."}, ...]}` を想定。
pub(crate) fn parse_claude_list_models_output(stdout: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("claude list-models の JSON 解析に失敗: {e}"))?;
    let arr = value
        .get("models")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "claude list-models 出力に 'models' 配列がありません".to_string())?;
    let mut result = Vec::with_capacity(arr.len());
    for (idx, entry) in arr.iter().enumerate() {
        let id = entry
            .get("value")
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("id").and_then(|v| v.as_str()))
            .ok_or_else(|| {
                format!("claude list-models 出力の models[{idx}] に有効な value/id がありません")
            })?;
        result.push(id.to_string());
    }
    Ok(result)
}

#[async_trait]
impl AgentBackend for ClaudeBackend {
    fn id(&self) -> &str {
        CLAUDE_BACKEND_ID
    }

    fn name(&self) -> &str {
        "Claude"
    }

    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
        Ok(SessionHandle {
            chat_session_id: config.chat_session_id,
            backend_id: CLAUDE_BACKEND_ID.to_string(),
        })
    }

    async fn send_message(
        &self,
        _session: &SessionHandle,
        _message: AgentMessage,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn interrupt(&self, _session: &SessionHandle) -> Result<(), String> {
        Ok(())
    }

    async fn respond_permission(
        &self,
        _session: &SessionHandle,
        _response: PermissionResponse,
    ) -> Result<(), String> {
        Ok(())
    }

    fn supports_cli_model_fetch(&self) -> bool {
        // `with_app` 経由で構築された場合は capability あり（解決失敗も Err として
        // fetch から返す）。`new()` 由来（テスト時等）は skip。
        self.list_models_script.is_some()
    }

    async fn fetch_models_from_cli(&self) -> Result<Vec<String>, String> {
        let script = match self.list_models_script.as_ref() {
            Some(Ok(path)) => path,
            Some(Err(e)) => {
                return Err(format!(
                    "Claude probe スクリプトの resource 解決に失敗: {e}"
                ));
            }
            None => {
                return Err("Claude probe スクリプトが未解決のため取得できません".to_string());
            }
        };
        let script_str = script
            .to_str()
            .ok_or_else(|| "Claude probe スクリプトパスに不正な UTF-8 が含まれます".to_string())?;

        let mut cmd = Command::new("node");
        cmd.arg(script_str);

        let output = run_command_with_output_limit(
            cmd,
            CLAUDE_PROBE_TIMEOUT,
            CLAUDE_PROBE_OUTPUT_MAX_BYTES,
            "claude probe",
            &format!("claude probe 起動失敗 ({script_str})"),
            "claude probe 待機失敗",
            format!(
                "claude probe タイムアウト ({} 秒)",
                CLAUDE_PROBE_TIMEOUT.as_secs()
            ),
        )
        .await?;

        if !output.status.success() {
            // stderr 本文には SDK/CLI 側の診断文字列が含まれ、認証ヘッダや token などの
            // 機密文字列が混ざる可能性があるためログに残さない。
            let _ = output.stderr;
            log::debug!("claude probe failed status={:?}", output.status.code());
            return Err(format!(
                "claude probe が失敗しました (status={:?})",
                output.status.code()
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let models = parse_claude_list_models_output(&stdout)?;
        if models.is_empty() {
            return Err("claude probe の出力が空でした".to_string());
        }
        Ok(models)
    }

    fn runtime_config(&self, _app: &tauri::AppHandle) -> BackendRuntimeConfig {
        BackendRuntimeConfig {
            bridge_init_options: serde_json::Map::new(),
        }
    }

    async fn close_session(&self, _session: &SessionHandle) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_cli_model_fetch_is_false() {
        let backend = ClaudeBackend::new();
        assert!(!backend.supports_cli_model_fetch());
    }

    #[test]
    fn parse_claude_list_models_output_extracts_values_in_order() {
        let stdout = r#"{"models":[{"value":"sonnet"},{"value":"haiku"}]}"#;
        let result = parse_claude_list_models_output(stdout).unwrap();
        assert_eq!(result, vec!["sonnet".to_string(), "haiku".to_string()]);
    }

    #[test]
    fn parse_claude_list_models_output_falls_back_to_id_when_value_missing() {
        let stdout = r#"{"models":[{"id":"only-id"},{"value":"vv","id":"ignored"}]}"#;
        let result = parse_claude_list_models_output(stdout).unwrap();
        // value 優先、無ければ id を採用する
        assert_eq!(result, vec!["only-id".to_string(), "vv".to_string()]);
    }

    #[test]
    fn parse_claude_list_models_output_errors_on_invalid_json() {
        let stdout = "not-json";
        let err = parse_claude_list_models_output(stdout).unwrap_err();
        assert!(err.contains("JSON 解析"));
    }

    #[test]
    fn parse_claude_list_models_output_errors_when_models_missing() {
        let stdout = r#"{"other": []}"#;
        let err = parse_claude_list_models_output(stdout).unwrap_err();
        assert!(err.contains("'models' 配列"));
    }

    #[test]
    fn parse_claude_list_models_output_errors_when_entry_has_no_id() {
        let stdout = r#"{"models":[{"foo":"bar"}]}"#;
        let err = parse_claude_list_models_output(stdout).unwrap_err();
        assert!(err.contains("value/id"));
    }

    #[test]
    fn parse_claude_list_models_output_accepts_empty_models_array() {
        // 空配列は parser としては成功、空 ID 拒否は呼び出し側（fetch_models_from_cli）の責務
        let stdout = r#"{"models":[]}"#;
        let result = parse_claude_list_models_output(stdout).unwrap();
        assert!(result.is_empty());
    }

    fn write_probe_script(body: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.mjs");
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn fetch_models_from_cli_runs_probe_script_and_returns_models() {
        let (_dir, path) = write_probe_script(
            r#"console.log(JSON.stringify({models:[{value:"sonnet"},{id:"haiku"}]}));"#,
        );
        let backend = ClaudeBackend {
            list_models_script: Some(Ok(path)),
        };

        let models = backend.fetch_models_from_cli().await.unwrap();

        assert_eq!(models, vec!["sonnet".to_string(), "haiku".to_string()]);
    }

    #[tokio::test]
    async fn fetch_models_from_cli_rejects_empty_models() {
        let (_dir, path) = write_probe_script(r#"console.log(JSON.stringify({models:[]}));"#);
        let backend = ClaudeBackend {
            list_models_script: Some(Ok(path)),
        };

        let err = backend.fetch_models_from_cli().await.unwrap_err();

        assert!(err.contains("空"));
    }

    #[tokio::test]
    async fn fetch_models_from_cli_errors_on_nonzero_exit() {
        let (_dir, path) = write_probe_script(r#"console.error("boom"); process.exit(7);"#);
        let backend = ClaudeBackend {
            list_models_script: Some(Ok(path)),
        };

        let err = backend.fetch_models_from_cli().await.unwrap_err();

        assert!(err.contains("失敗"));
    }
}
