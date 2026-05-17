pub mod bridge_common;
pub mod claude;
pub mod codex;
pub(crate) mod model_catalog_sync;
mod permission_flags;
pub(crate) mod process_io;
pub mod runtime_coordinator;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

use crate::config::AppConfig;
use crate::domain::agent_session::{escaped_for_log, ModelId, ModelIdList};
use crate::session::SessionStore;

/// Backend-specific runtime values consumed by the generic bridge process runner.
#[derive(Debug, Clone, Default)]
pub struct BackendRuntimeConfig {
    pub bridge_init_options: serde_json::Map<String, serde_json::Value>,
}

/// バックエンドの表示情報。レジストリからUI向けに返却する。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendInfo {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub available_models: Vec<ModelInfo>,
}

/// モデル情報（共通型）。全バックエンドで使用する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialModelResolution {
    Unset,
    Registered(String),
    Invalid { model: String, reason: String },
    Unregistered { model: String },
}

/// 画像添付（共通型）。全バックエンドで使用する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    pub data: String,
    pub media_type: String,
}

/// セッション開始時の共通設定。
#[allow(dead_code)]
pub struct SessionConfig {
    pub chat_session_id: String,
    pub cwd: String,
    pub permission_mode: Option<String>,
    pub system_prompt: Option<String>,
}

/// セッションのハンドル。バックエンド操作の識別子。
#[allow(dead_code)]
pub struct SessionHandle {
    pub chat_session_id: String,
    pub backend_id: String,
}

/// ユーザーメッセージ。
#[allow(dead_code)]
pub struct AgentMessage {
    pub content: String,
    pub streaming_message_id: String,
    pub images: Vec<ImageAttachment>,
    pub permission_mode: String,
}

/// ツール実行許可への応答。
#[allow(dead_code)]
pub struct PermissionResponse {
    pub request_id: String,
    pub behavior: String,
    pub message: Option<String>,
    pub updated_input: Option<String>,
}

/// エージェントバックエンドの共通インターフェース。
/// 全てのバックエンド実装がこの trait を満たす。
#[allow(dead_code)]
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// バックエンドの一意識別子。
    fn id(&self) -> &str;

    /// バックエンドの表示名。
    fn name(&self) -> &str;

    /// セッションを開始する。
    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String>;

    /// メッセージを送信し、ストリーミングを開始する。
    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String>;

    /// 実行中のターンを中断する。
    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String>;

    /// ツール実行許可に応答する。
    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String>;

    /// 起動時の CLI 由来モデル一覧取得をサポートするか。
    /// `false` を返すバックエンドは `refresh_models_to_config_for` から skip され、
    /// `fetch_models_from_cli` は呼ばれない。
    fn supports_cli_model_fetch(&self) -> bool {
        true
    }

    /// バックエンドCLIから生のモデル識別子リストを取得する。
    /// 検証・永続化は呼び出し側（Registry）で行うため、本メソッドは
    /// 取得した生の値を返すだけで config を書き換えない。
    ///
    /// `supports_cli_model_fetch()` が `false` のバックエンドでは呼ばれない。
    async fn fetch_models_from_cli(&self) -> Result<Vec<String>, String> {
        Err("このバックエンドは起動時CLIモデル取得をサポートしていません".to_string())
    }

    /// Bridge 起動時に必要なバックエンド固有の設定を返す。
    fn runtime_config(&self, _app: &tauri::AppHandle) -> BackendRuntimeConfig {
        BackendRuntimeConfig::default()
    }

    /// セッションを終了する。
    async fn close_session(&self, session: &SessionHandle) -> Result<(), String>;
}

/// バックエンドの登録・取得・一覧を管理するレジストリ。
/// 挿入順を保持し、config.toml の記載順に対応する。
pub struct AgentBackendRegistry {
    backends: Vec<(String, Arc<dyn AgentBackend>, bool)>,
    default_id: Option<String>,
    config: Option<Arc<AppConfig>>,
}

impl AgentBackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
            default_id: None,
            config: None,
        }
    }

    /// `AppConfig` を関連付ける。`available_models()` / `resolve_backend_for_model()` は
    /// この config の `agents.<backend>.models` を参照する。
    pub fn set_config(&mut self, config: Arc<AppConfig>) {
        self.config = Some(config);
    }

    /// 指定 backend の config 由来の初期モデルを返す。
    /// `agents.<backend>.model` が `agents.<backend>.models` に含まれる場合のみ採用する。
    /// 未紐付け／schema 未対応／未登録の場合は `None` を返す。
    pub fn initial_model_for(&self, backend_id: &str) -> Option<String> {
        match self.initial_model_resolution_for(backend_id) {
            InitialModelResolution::Registered(model) => Some(model),
            InitialModelResolution::Invalid { model, reason } => {
                let model = escaped_for_log(&model);
                log::warn!(
                    "backend '{backend_id}' configured initial model {model} is invalid and will be ignored: {reason}"
                );
                None
            }
            InitialModelResolution::Unregistered { model } => {
                let model = escaped_for_log(&model);
                log::warn!(
                    "backend '{backend_id}' configured initial model {model} is not registered in current models and will be ignored"
                );
                None
            }
            InitialModelResolution::Unset => None,
        }
    }

    /// 起動時警告とセッション作成で共有する初期モデル整合性判定。
    /// 判定のみを返し、ログ出力や config 書き換えは行わない。
    pub fn initial_model_resolution_for(&self, backend_id: &str) -> InitialModelResolution {
        let Some(config) = self.config.as_ref() else {
            return InitialModelResolution::Unset;
        };
        let Some(model) = config.configured_initial_model_for_backend(backend_id) else {
            return InitialModelResolution::Unset;
        };
        if let Err(reason) = ModelId::parse(model.clone()) {
            return InitialModelResolution::Invalid { model, reason };
        }
        let Ok(models) = self.config_models_for(backend_id) else {
            return InitialModelResolution::Unset;
        };
        if models.iter().any(|value| value == &model) {
            InitialModelResolution::Registered(model)
        } else {
            InitialModelResolution::Unregistered { model }
        }
    }

    /// config に保存されているバックエンドごとのモデルID一覧を取得する。
    /// config 未紐付け／未知バックエンドの場合は `Err` を返し、登録済みモデルが
    /// 0 件の場合と区別できるようにする。
    pub fn config_models_for(&self, backend_id: &str) -> Result<Vec<String>, String> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| "AppConfig not attached to registry".to_string())?;
        config.models_for_backend(backend_id)
    }

    /// config の `agents.<backend>.models` を入力検証 → 重複除去した上で原子的に書き換える。
    /// 検証に失敗した場合・config schema に当該バックエンドが存在しない場合は
    /// config を変更せず `Err` を返す。他バックエンドの一覧は影響を受けない。
    pub fn write_models_to_config(
        &self,
        backend_id: &str,
        models: Vec<String>,
    ) -> Result<Vec<String>, String> {
        let validated = ModelIdList::parse_many(&models)?.into_strings();
        let Some(config) = &self.config else {
            return Err("AppConfig not attached to registry".to_string());
        };
        if self.get(backend_id).is_none() {
            return Err(format!(
                "バックエンド '{backend_id}' がレジストリに登録されていません"
            ));
        }
        config.set_models_for_backend(backend_id, validated.clone())?;
        Ok(validated)
    }

    /// 指定バックエンドのCLIへ問い合わせて結果を config に反映する。
    /// 失敗時は config を変更せず Err を返す。他バックエンドの一覧には影響を与えない。
    /// `supports_cli_model_fetch()` が `false` のバックエンドは `Ok(None)` を返す。
    pub async fn refresh_models_to_config_for(
        &self,
        backend_id: &str,
    ) -> Result<Option<Vec<String>>, String> {
        let backend = self.get(backend_id).ok_or_else(|| {
            format!("バックエンド '{backend_id}' がレジストリに登録されていません")
        })?;
        if !backend.supports_cli_model_fetch() {
            return Ok(None);
        }
        let raw = backend.fetch_models_from_cli().await?;
        if raw.is_empty() {
            return Err("CLI 取得結果が空です".to_string());
        }
        self.write_models_to_config(backend_id, raw).map(Some)
    }

    /// バックエンドを登録する。同一IDの重複登録は無視する。
    pub fn register(&mut self, backend: Arc<dyn AgentBackend>) {
        let id = backend.id().to_string();
        if self.backends.iter().any(|(bid, _, _)| bid == &id) {
            return;
        }
        self.backends.push((id, backend, true));
    }

    /// バックエンドの利用可否を設定する。
    /// Phase 2 で本番コードからも使用予定。
    #[cfg(test)]
    pub fn set_available(&mut self, id: &str, available: bool) {
        if let Some(entry) = self.backends.iter_mut().find(|(bid, _, _)| bid == id) {
            entry.2 = available;
        }
    }

    /// デフォルトバックエンドIDを設定する。
    pub fn set_default(&mut self, id: Option<String>) {
        self.default_id = id;
    }

    /// 登録済みバックエンドの一覧を返す。
    pub fn list(&self) -> Vec<BackendInfo> {
        self.backends
            .iter()
            .map(|(id, b, available)| BackendInfo {
                id: id.clone(),
                name: b.name().to_string(),
                available: *available,
                available_models: self.available_models(id).unwrap_or_else(|e| {
                    log::warn!("backend '{id}' model list could not be read for backend list: {e}");
                    Vec::new()
                }),
            })
            .collect()
    }

    /// IDでバックエンドを取得する。
    pub fn get(&self, id: &str) -> Option<Arc<dyn AgentBackend>> {
        self.backends
            .iter()
            .find(|(bid, _, _)| bid == id)
            .map(|(_, b, _)| Arc::clone(b))
    }

    /// 指定バックエンドのモデル一覧を返す。供給元は config.toml の
    /// `agents.<backend>.models`（順序保持）。
    /// config 未紐付け／schema 未対応／lock 失敗は `Err` として伝播し、
    /// 「登録済みモデルが 0 件」と区別できるようにする。
    pub fn available_models(&self, id: &str) -> Result<Vec<ModelInfo>, String> {
        if self.get(id).is_none() {
            return Err(format!(
                "バックエンド '{id}' がレジストリに登録されていません"
            ));
        }
        Ok(self
            .config_models_for(id)?
            .into_iter()
            .map(|value| ModelInfo { value })
            .collect())
    }

    pub fn runtime_config(&self, id: &str, app: &tauri::AppHandle) -> Option<BackendRuntimeConfig> {
        self.get(id).map(|backend| backend.runtime_config(app))
    }

    /// 指定されたバックエンドIDを検証し、未指定の場合はデフォルトを解決する。
    /// - Some(id): レジストリに登録されているか検証し、登録されていればそのIDを返す
    /// - None: resolve_default_id() に委譲してデフォルトを解決する
    pub fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
        match backend_id {
            Some(bid) => {
                if self.get(&bid).is_none() {
                    return Err(format!(
                        "バックエンド '{}' がレジストリに登録されていません",
                        bid
                    ));
                }
                Ok(bid)
            }
            None => self.resolve_default_id(),
        }
    }

    /// デフォルトバックエンドIDを解決する。
    /// 1. config.toml で指定されたデフォルト → そのバックエンドが利用可能なら使用、なければエラー
    /// 2. 未指定 → 登録順で最初の利用可能なバックエンド
    /// 3. 利用可能なバックエンドが1つもない → エラー
    pub fn resolve_default_id(&self) -> Result<String, String> {
        if let Some(ref default_id) = self.default_id {
            if self
                .backends
                .iter()
                .any(|(id, _, avail)| id == default_id && *avail)
            {
                return Ok(default_id.clone());
            }
            return Err(format!(
                "デフォルトバックエンド '{}' がレジストリに登録されていないか利用不可です",
                default_id
            ));
        }

        self.backends
            .iter()
            .find(|(_, _, avail)| *avail)
            .map(|(id, _, _)| id.clone())
            .ok_or_else(|| "利用可能なバックエンドが登録されていません".to_string())
    }

    /// モデルIDから対応するバックエンドIDを解決する。
    /// 同一モデルIDが複数バックエンドに登録されている場合は一意特定不能として
    /// `Err` を返す（サイレントフォールバックを避けるため）。
    /// 該当するバックエンドが存在しない場合は `Ok(None)` を返す。
    /// いずれかのバックエンドで config 取得に失敗した場合も `Err` を返す。
    pub fn resolve_backend_for_model(&self, model: &str) -> Result<Option<String>, String> {
        let mut matched: Vec<String> = Vec::new();
        for (id, _, available) in &self.backends {
            if !*available {
                continue;
            }
            let models = self.config_models_for(id)?;
            if models.iter().any(|v| v == model) {
                matched.push(id.clone());
            }
        }
        match matched.len() {
            0 => Ok(None),
            1 => Ok(matched.into_iter().next()),
            _ => Err(format!(
                "モデル '{model}' が複数のバックエンドに登録されているため一意特定できません: {}",
                matched.join(", ")
            )),
        }
    }
}

// --- Tauri コマンド ---

/// バックエンド一覧取得のレスポンス。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendListResult {
    pub backends: Vec<BackendInfo>,
    pub default_id: Option<String>,
}

#[tauri::command]
pub fn list_agent_backends(registry: State<'_, Arc<AgentBackendRegistry>>) -> BackendListResult {
    BackendListResult {
        backends: registry.list(),
        default_id: registry.resolve_default_id().ok(),
    }
}

/// config.toml `[agents]` セクションからレジストリを構築する。
#[allow(dead_code)]
pub fn build_registry(config: Arc<AppConfig>) -> AgentBackendRegistry {
    build_registry_inner(config, None, None)
}

/// 実アプリ用: CodexBackend に AgentProcess bridge runtime を接続して登録する。
pub fn build_registry_with_runtime(
    config: Arc<AppConfig>,
    app: tauri::AppHandle,
    handles: Arc<Mutex<bridge_common::AgentProcessMap>>,
    session_store: Arc<SessionStore>,
) -> AgentBackendRegistry {
    build_registry_inner(
        config,
        Some(Arc::new(claude::ClaudeBackend::with_app(&app))),
        Some(Arc::new(codex::CodexBackend::with_agent_process_runtime(
            app,
            handles,
            session_store,
        ))),
    )
}

fn build_registry_inner(
    config: Arc<AppConfig>,
    claude_backend: Option<Arc<dyn AgentBackend>>,
    codex_backend: Option<Arc<dyn AgentBackend>>,
) -> AgentBackendRegistry {
    let mut registry = AgentBackendRegistry::new();

    // Claude バックエンドは常に利用可能（組み込み）
    let claude = claude_backend.unwrap_or_else(|| Arc::new(claude::ClaudeBackend::new()));
    registry.register(claude);

    // Codex バックエンドもプロジェクト依存として常に利用可能
    let codex = codex_backend.unwrap_or_else(|| Arc::new(codex::CodexBackend::new()));
    registry.register(codex);

    // config.toml の設定を適用
    if let Ok(cfg) = config.get_config() {
        registry.set_default(cfg.agents.default.clone());
    }
    registry.set_config(config);

    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    struct MockBackend {
        backend_id: String,
        backend_name: String,
        cli_models: Result<Vec<String>, String>,
        supports_cli_fetch: bool,
    }

    #[async_trait]
    impl AgentBackend for MockBackend {
        fn id(&self) -> &str {
            &self.backend_id
        }
        fn name(&self) -> &str {
            &self.backend_name
        }
        async fn start_session(&self, _config: SessionConfig) -> Result<SessionHandle, String> {
            Ok(SessionHandle {
                chat_session_id: "test".to_string(),
                backend_id: self.backend_id.clone(),
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
            self.supports_cli_fetch
        }
        async fn fetch_models_from_cli(&self) -> Result<Vec<String>, String> {
            self.cli_models.clone()
        }
        async fn close_session(&self, _session: &SessionHandle) -> Result<(), String> {
            Ok(())
        }
    }

    fn mock_backend(id: &str, name: &str) -> Arc<dyn AgentBackend> {
        Arc::new(MockBackend {
            backend_id: id.to_string(),
            backend_name: name.to_string(),
            cli_models: Ok(Vec::new()),
            supports_cli_fetch: true,
        })
    }

    fn mock_backend_with_cli(
        id: &str,
        name: &str,
        cli_models: Result<Vec<String>, String>,
    ) -> Arc<dyn AgentBackend> {
        Arc::new(MockBackend {
            backend_id: id.to_string(),
            backend_name: name.to_string(),
            cli_models,
            supports_cli_fetch: true,
        })
    }

    fn mock_backend_unsupported_cli(id: &str, name: &str) -> Arc<dyn AgentBackend> {
        Arc::new(MockBackend {
            backend_id: id.to_string(),
            backend_name: name.to_string(),
            cli_models: Err("never called".to_string()),
            supports_cli_fetch: false,
        })
    }

    fn make_test_app_config() -> Arc<AppConfig> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Arc::new(AppConfig::new(
            crate::config::ReleashConfig::default(),
            tmp.path().to_path_buf(),
        ))
    }

    fn make_test_app_config_with_models(
        claude_models: &[&str],
        codex_models: &[&str],
    ) -> Arc<AppConfig> {
        let mut cfg = crate::config::ReleashConfig::default();
        cfg.agents.claude.models = claude_models.iter().map(|s| s.to_string()).collect();
        cfg.agents.codex.models = codex_models.iter().map(|s| s.to_string()).collect();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Arc::new(AppConfig::new(cfg, tmp.path().to_path_buf()))
    }

    fn make_test_app_config_with_initial_model(
        backend_id: &str,
        model: Option<&str>,
        models: &[&str],
    ) -> Arc<AppConfig> {
        let mut cfg = crate::config::ReleashConfig::default();
        match backend_id {
            "claude" => {
                cfg.agents.claude.model = model.map(str::to_string);
                cfg.agents.claude.models = models.iter().map(|s| s.to_string()).collect();
            }
            "codex" => {
                cfg.agents.codex.model = model.map(str::to_string);
                cfg.agents.codex.models = models.iter().map(|s| s.to_string()).collect();
            }
            _ => {}
        }
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Arc::new(AppConfig::new(cfg, tmp.path().to_path_buf()))
    }

    #[test]
    fn registry_starts_empty() {
        let reg = AgentBackendRegistry::new();
        assert!(reg.list().is_empty());
    }

    #[test]
    fn register_and_list_backend() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "claude");
        assert_eq!(list[0].name, "Claude");
        assert!(list[0].available);
    }

    #[test]
    fn initial_model_for_returns_some_when_registered() {
        let config =
            make_test_app_config_with_initial_model("claude", Some("opus-4"), &["opus-4", "haiku"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config);

        assert_eq!(reg.initial_model_for("claude"), Some("opus-4".to_string()));
    }

    #[test]
    fn initial_model_for_returns_none_when_unregistered() {
        let config = make_test_app_config_with_initial_model("claude", Some("opus-4"), &["haiku"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config);

        assert_eq!(reg.initial_model_for("claude"), None);
    }

    #[test]
    fn initial_model_resolution_reports_unregistered_model() {
        let config = make_test_app_config_with_initial_model("claude", Some("opus-4"), &["haiku"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config);

        assert_eq!(
            reg.initial_model_resolution_for("claude"),
            InitialModelResolution::Unregistered {
                model: "opus-4".to_string()
            }
        );
    }

    #[test]
    fn initial_model_for_returns_none_when_configured_model_is_invalid() {
        let config =
            make_test_app_config_with_initial_model("claude", Some("bad\u{0001}model"), &[]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config);

        assert_eq!(reg.initial_model_for("claude"), None);
    }

    #[test]
    fn initial_model_for_returns_none_when_configured_model_is_too_long() {
        let model = "x".repeat(129);
        let config = make_test_app_config_with_initial_model("claude", Some(&model), &[]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config);

        assert_eq!(reg.initial_model_for("claude"), None);
    }

    #[test]
    fn initial_model_for_returns_none_when_model_unset() {
        let config = make_test_app_config_with_initial_model("claude", None, &["opus-4"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config);

        assert_eq!(reg.initial_model_for("claude"), None);
    }

    #[test]
    fn duplicate_registration_is_ignored() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("claude", "Claude v2"));
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].name, "Claude");
    }

    #[test]
    fn get_existing_backend() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        assert!(reg.get("claude").is_some());
        assert!(reg.get("codex").is_none());
    }

    #[test]
    fn resolve_default_with_explicit_setting() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_default(Some("codex".to_string()));
        assert_eq!(reg.resolve_default_id().unwrap(), "codex");
    }

    #[test]
    fn resolve_default_falls_back_to_first() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        assert_eq!(reg.resolve_default_id().unwrap(), "claude");
    }

    #[test]
    fn resolve_default_with_nonexistent_returns_error() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_default(Some("nonexistent".to_string()));
        let result = reg.resolve_default_id();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("nonexistent"));
    }

    #[test]
    fn resolve_default_with_empty_registry_returns_error() {
        let reg = AgentBackendRegistry::new();
        assert!(reg.resolve_default_id().is_err());
    }

    #[test]
    fn insertion_order_is_preserved() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("beta", "Beta"));
        reg.register(mock_backend("alpha", "Alpha"));
        reg.register(mock_backend("gamma", "Gamma"));
        let list = reg.list();
        assert_eq!(list[0].id, "beta");
        assert_eq!(list[1].id, "alpha");
        assert_eq!(list[2].id, "gamma");
    }

    #[test]
    fn resolve_default_skips_unavailable_first_backend() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("alpha", "Alpha"));
        reg.register(mock_backend("beta", "Beta"));
        reg.set_available("alpha", false);
        assert_eq!(reg.resolve_default_id().unwrap(), "beta");
    }

    #[test]
    fn resolve_default_explicit_unavailable_returns_error() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_default(Some("claude".to_string()));
        reg.set_available("claude", false);
        let result = reg.resolve_default_id();
        assert!(result.is_err());
    }

    #[test]
    fn list_includes_availability_status() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_available("codex", false);
        let list = reg.list();
        assert!(list[0].available);
        assert!(!list[1].available);
    }

    #[test]
    fn resolve_backend_id_with_existing_id() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        assert_eq!(
            reg.resolve_backend_id(Some("claude".to_string())).unwrap(),
            "claude"
        );
    }

    #[test]
    fn resolve_backend_id_with_nonexistent_id_returns_error() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        let result = reg.resolve_backend_id(Some("codex".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("codex"));
    }

    #[test]
    fn resolve_backend_id_with_none_returns_default() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        assert_eq!(reg.resolve_backend_id(None).unwrap(), "claude");
    }

    #[test]
    fn build_registry_registers_claude_backend() {
        let config = make_test_app_config();
        let reg = build_registry(config);
        let list = reg.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "claude");
        assert_eq!(list[0].name, "Claude");
        assert!(list[0].available);
        assert_eq!(list[1].id, "codex");
        assert_eq!(list[1].name, "Codex");
        assert!(list[1].available);
    }

    #[test]
    fn build_registry_applies_default_from_config() {
        let mut cfg = crate::config::ReleashConfig::default();
        cfg.agents.default = Some("codex".to_string());
        let config = Arc::new(AppConfig::new(
            cfg,
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        ));
        let reg = build_registry(config);
        assert_eq!(reg.resolve_default_id().unwrap(), "codex");
    }

    #[test]
    fn build_registry_without_default_config_falls_back_to_first() {
        let config = make_test_app_config();
        let reg = build_registry(config);
        assert_eq!(reg.resolve_default_id().unwrap(), "claude");
    }

    #[test]
    fn available_models_returns_empty_when_config_empty() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(make_test_app_config());
        assert!(reg.available_models("claude").unwrap().is_empty());
    }

    #[test]
    fn available_models_reads_from_config() {
        let config = make_test_app_config_with_models(&["opus-4", "haiku"], &["codex-mini"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_config(config);
        let claude_models = reg.available_models("claude").unwrap();
        assert_eq!(claude_models.len(), 2);
        assert_eq!(claude_models[0].value, "opus-4");
        assert_eq!(claude_models[1].value, "haiku");
        let codex_models = reg.available_models("codex").unwrap();
        assert_eq!(codex_models.len(), 1);
        assert_eq!(codex_models[0].value, "codex-mini");
    }

    #[test]
    fn resolve_backend_for_model_finds_correct_backend() {
        let config = make_test_app_config_with_models(&["opus-4", "haiku"], &["codex-mini"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_config(config);
        assert_eq!(
            reg.resolve_backend_for_model("opus-4").unwrap(),
            Some("claude".to_string())
        );
        assert_eq!(
            reg.resolve_backend_for_model("codex-mini").unwrap(),
            Some("codex".to_string())
        );
    }

    #[test]
    fn resolve_backend_for_model_returns_none_for_unknown() {
        let config = make_test_app_config_with_models(&["opus-4"], &[]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config);
        assert_eq!(reg.resolve_backend_for_model("unknown").unwrap(), None);
    }

    #[test]
    fn resolve_backend_for_model_errors_when_multiple_backends_match() {
        let config = make_test_app_config_with_models(&["shared"], &["shared"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_config(config);
        let err = reg.resolve_backend_for_model("shared").unwrap_err();
        assert!(err.contains("claude"));
        assert!(err.contains("codex"));
    }

    #[test]
    fn resolve_backend_for_model_skips_unavailable_backends() {
        let config = make_test_app_config_with_models(&["opus-4"], &[]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config);
        reg.set_available("claude", false);
        assert_eq!(reg.resolve_backend_for_model("opus-4").unwrap(), None);
    }

    #[test]
    fn write_models_to_config_dedupes_and_persists() {
        let config = make_test_app_config();
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config.clone());
        let stored = reg
            .write_models_to_config(
                "claude",
                vec!["a".to_string(), "b".to_string(), "a".to_string()],
            )
            .unwrap();
        assert_eq!(stored, vec!["a".to_string(), "b".to_string()]);
        let cfg = config.get_config().unwrap();
        assert_eq!(
            cfg.agents.claude.models,
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn write_models_to_config_rejects_invalid_without_changing_state() {
        let config = make_test_app_config_with_models(&["existing"], &[]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config.clone());
        let err = reg.write_models_to_config("claude", vec!["valid".to_string(), "".to_string()]);
        assert!(err.is_err());
        // 既存値は維持されること
        let cfg = config.get_config().unwrap();
        assert_eq!(cfg.agents.claude.models, vec!["existing".to_string()]);
    }

    #[test]
    fn write_models_to_config_rejects_unregistered_backend() {
        let config = make_test_app_config();
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config.clone());
        let err = reg.write_models_to_config("nonexistent", vec!["a".to_string()]);
        assert!(err.is_err());
        // config が変更されていない（unknown backend は no-op success にならない）
        let cfg = config.get_config().unwrap();
        assert!(cfg.agents.claude.models.is_empty());
        assert!(cfg.agents.codex.models.is_empty());
    }

    #[test]
    fn write_models_to_config_does_not_affect_other_backend() {
        let config = make_test_app_config_with_models(&["c1"], &["x1", "x2"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_config(config.clone());
        reg.write_models_to_config("claude", vec!["c2".to_string()])
            .unwrap();
        let cfg = config.get_config().unwrap();
        assert_eq!(cfg.agents.claude.models, vec!["c2".to_string()]);
        assert_eq!(
            cfg.agents.codex.models,
            vec!["x1".to_string(), "x2".to_string()]
        );
    }

    // --- refresh_models_to_config_for: 仕様の中核 Rule の回帰防止テスト ---

    #[tokio::test]
    async fn refresh_updates_config_on_cli_success() {
        let config = make_test_app_config_with_models(&[], &["stale"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend_with_cli(
            "codex",
            "Codex",
            Ok(vec!["gpt-5.5".to_string(), "o3".to_string()]),
        ));
        reg.set_config(config.clone());

        let result = reg.refresh_models_to_config_for("codex").await.unwrap();
        assert_eq!(result, Some(vec!["gpt-5.5".to_string(), "o3".to_string()]));
        let cfg = config.get_config().unwrap();
        assert_eq!(
            cfg.agents.codex.models,
            vec!["gpt-5.5".to_string(), "o3".to_string()]
        );
    }

    #[tokio::test]
    async fn refresh_keeps_existing_models_on_cli_failure() {
        let config = make_test_app_config_with_models(&[], &["existing"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend_with_cli(
            "codex",
            "Codex",
            Err("CLI が見つかりません".to_string()),
        ));
        reg.set_config(config.clone());

        let err = reg.refresh_models_to_config_for("codex").await;
        assert!(err.is_err());
        // 失敗時は config を変更しない
        let cfg = config.get_config().unwrap();
        assert_eq!(cfg.agents.codex.models, vec!["existing".to_string()]);
    }

    #[tokio::test]
    async fn refresh_rejects_invalid_cli_results_without_changing_state() {
        let config = make_test_app_config_with_models(&[], &["existing"]);
        let mut reg = AgentBackendRegistry::new();
        // 空文字を含む → 検証失敗
        reg.register(mock_backend_with_cli(
            "codex",
            "Codex",
            Ok(vec!["valid".to_string(), "".to_string()]),
        ));
        reg.set_config(config.clone());

        let err = reg.refresh_models_to_config_for("codex").await;
        assert!(err.is_err());
        let cfg = config.get_config().unwrap();
        assert_eq!(cfg.agents.codex.models, vec!["existing".to_string()]);
    }

    #[tokio::test]
    async fn refresh_one_backend_does_not_affect_others() {
        let config = make_test_app_config_with_models(&["c-existing"], &["x-existing"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend_with_cli(
            "claude",
            "Claude",
            Ok(vec!["c-new".to_string()]),
        ));
        reg.register(mock_backend_with_cli(
            "codex",
            "Codex",
            Err("fail".to_string()),
        ));
        reg.set_config(config.clone());

        let _ = reg.refresh_models_to_config_for("claude").await.unwrap();
        let _ = reg.refresh_models_to_config_for("codex").await;

        let cfg = config.get_config().unwrap();
        // 成功側だけ更新、失敗側の既存値は維持
        assert_eq!(cfg.agents.claude.models, vec!["c-new".to_string()]);
        assert_eq!(cfg.agents.codex.models, vec!["x-existing".to_string()]);
    }

    #[tokio::test]
    async fn refresh_skips_when_backend_does_not_support_cli_fetch() {
        let config = make_test_app_config_with_models(&["existing"], &[]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend_unsupported_cli("claude", "Claude"));
        reg.set_config(config.clone());

        let result = reg.refresh_models_to_config_for("claude").await.unwrap();
        assert_eq!(result, None);
        // CLI 未対応バックエンドは fetch を呼ばないので config は変わらない
        let cfg = config.get_config().unwrap();
        assert_eq!(cfg.agents.claude.models, vec!["existing".to_string()]);
    }

    #[tokio::test]
    async fn refresh_concurrent_backends_complete_independently() {
        // 起動時に各バックエンドを並行 spawn しても、片方の遅延/失敗が他方を
        // ブロックしないこと（spec: 起動シーケンスはモデル取得完了を待たない）。
        use std::sync::Arc as StdArc;
        use std::time::Duration;
        use tokio::sync::Mutex as TokioMutex;

        struct SlowBackend {
            backend_id: String,
            delay_ms: u64,
            result: Result<Vec<String>, String>,
        }

        #[async_trait]
        impl AgentBackend for SlowBackend {
            fn id(&self) -> &str {
                &self.backend_id
            }
            fn name(&self) -> &str {
                "Slow"
            }
            async fn start_session(&self, _config: SessionConfig) -> Result<SessionHandle, String> {
                Ok(SessionHandle {
                    chat_session_id: "x".to_string(),
                    backend_id: self.backend_id.clone(),
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
            async fn fetch_models_from_cli(&self) -> Result<Vec<String>, String> {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
                self.result.clone()
            }
            async fn close_session(&self, _session: &SessionHandle) -> Result<(), String> {
                Ok(())
            }
        }

        let config = make_test_app_config_with_models(&["c-existing"], &["x-existing"]);
        let mut registry = AgentBackendRegistry::new();
        // claude: 即時失敗
        registry.register(StdArc::new(SlowBackend {
            backend_id: bridge_common::CLAUDE_BACKEND_ID.to_string(),
            delay_ms: 0,
            result: Err("fail".to_string()),
        }));
        // codex: 遅延あり成功
        registry.register(StdArc::new(SlowBackend {
            backend_id: bridge_common::CODEX_BACKEND_ID.to_string(),
            delay_ms: 200,
            result: Ok(vec!["gpt-5.5".to_string()]),
        }));
        registry.set_config(config.clone());
        let registry = Arc::new(registry);

        // 起動シーケンスを模擬: 並行 spawn
        let r1 = Arc::clone(&registry);
        let r2 = Arc::clone(&registry);
        let started = std::time::Instant::now();
        let setup_completed = StdArc::new(TokioMutex::new(false));
        let setup_completed_clone = StdArc::clone(&setup_completed);

        let claude_task = tokio::spawn(async move {
            r1.refresh_models_to_config_for(bridge_common::CLAUDE_BACKEND_ID)
                .await
        });
        let codex_task = tokio::spawn(async move {
            r2.refresh_models_to_config_for(bridge_common::CODEX_BACKEND_ID)
                .await
        });

        // 起動シーケンスは spawn 後に即時続行する
        {
            let mut flag = setup_completed_clone.lock().await;
            *flag = true;
        }
        let setup_elapsed = started.elapsed();
        // setup 自体は遅い backend の完了を待たない（200ms 未満で完了する想定）
        assert!(
            setup_elapsed < Duration::from_millis(150),
            "startup should not wait for slow backend; took {:?}",
            setup_elapsed
        );
        assert!(*setup_completed.lock().await);

        // 片方が失敗、もう片方は遅延の末に成功する。最終的に両方 join できる。
        let claude_result = claude_task.await.unwrap();
        let codex_result = codex_task.await.unwrap();
        assert!(claude_result.is_err());
        assert_eq!(codex_result.unwrap(), Some(vec!["gpt-5.5".to_string()]));

        // 成功側だけ反映、失敗側は config 既存値を維持
        let cfg = config.get_config().unwrap();
        assert_eq!(cfg.agents.claude.models, vec!["c-existing".to_string()]);
        assert_eq!(cfg.agents.codex.models, vec!["gpt-5.5".to_string()]);
    }

    #[tokio::test]
    async fn refresh_rejects_empty_cli_result() {
        let config = make_test_app_config_with_models(&[], &["existing"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend_with_cli("codex", "Codex", Ok(Vec::new())));
        reg.set_config(config.clone());

        let err = reg.refresh_models_to_config_for("codex").await;
        assert!(err.is_err());
        let cfg = config.get_config().unwrap();
        assert_eq!(cfg.agents.codex.models, vec!["existing".to_string()]);
    }
}
