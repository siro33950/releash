pub mod bridge_common;
pub mod claude;
pub mod codex;
pub mod codex_app_server;
mod permission_flags;
pub mod runtime_coordinator;

pub(crate) use bridge_common::*;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::Manager;
use tauri::State;
use tokio::sync::Mutex;

use crate::domain::app_config::AgentConfigRepository;
use crate::usecase::agent_session::session::SessionStore;

impl From<ModelInfo> for crate::usecase::agent_session::session::ModelInfo {
    fn from(info: ModelInfo) -> Self {
        Self {
            id: info.id,
            display_name: info.display_name,
            backend: info.backend,
            model_id: info.model_id,
        }
    }
}

impl From<TurnPhase> for crate::usecase::agent_session::status::TurnPhase {
    fn from(phase: TurnPhase) -> Self {
        match phase {
            TurnPhase::Idle => Self::Idle,
            TurnPhase::Streaming => Self::Streaming,
            TurnPhase::WaitingPermission => Self::WaitingPermission,
        }
    }
}

impl From<crate::usecase::agent_session::status::TurnPhase> for TurnPhase {
    fn from(phase: crate::usecase::agent_session::status::TurnPhase) -> Self {
        match phase {
            crate::usecase::agent_session::status::TurnPhase::Idle => Self::Idle,
            crate::usecase::agent_session::status::TurnPhase::Streaming => Self::Streaming,
            crate::usecase::agent_session::status::TurnPhase::WaitingPermission => {
                Self::WaitingPermission
            }
        }
    }
}

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
    pub id: String,
    pub display_name: String,
    pub backend: String,
    pub model_id: String,
}

impl ModelInfo {
    pub fn new(backend: &str, model_id: &str) -> Self {
        let entry = crate::domain::agent_session::model_entry_for_backend_model(backend, model_id);
        Self {
            id: entry.id,
            display_name: entry.display_name,
            backend: entry.backend,
            model_id: entry.model_id,
        }
    }
}

/// 画像添付（共通型）。全バックエンドで使用する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    pub data: String,
    pub media_type: String,
}

/// Editor state supplied by the desktop UI for runtime-native contextual input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEditorContext {
    #[serde(default)]
    pub active_editor_path: Option<String>,
    #[serde(default)]
    pub open_editor_paths: Vec<String>,
    #[serde(default)]
    pub selection: Option<AgentEditorSelection>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEditorSelection {
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// セッション開始時の共通設定。
#[allow(dead_code)]
pub struct SessionConfig {
    pub chat_session_id: String,
    pub cwd: String,
    pub permission_mode: Option<String>,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
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
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub editor_context: Option<AgentEditorContext>,
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

    /// Send runtime-native input to the currently active turn when supported.
    async fn steer_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String> {
        let _ = message;
        Err(format!(
            "Backend '{}' does not expose active-turn steering",
            session.backend_id
        ))
    }

    /// Returns true only when a backend has a ready active turn that can accept steering input.
    async fn active_turn_steering_ready(&self, _session: &SessionHandle) -> bool {
        false
    }

    /// 実行中のターンを中断する。
    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String>;

    /// ツール実行許可に応答する。
    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String>;

    /// Runtime-owned thread title update.
    async fn set_thread_name(&self, session: &SessionHandle, name: &str) -> Result<(), String> {
        let _ = name;
        Err(format!(
            "Backend '{}' does not expose runtime thread naming",
            session.backend_id
        ))
    }

    /// Runtime-owned permission settings update for already-started sessions.
    async fn set_permission_mode(
        &self,
        session: &SessionHandle,
        cwd: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        let _ = cwd;
        let _ = permission_mode;
        Err(format!(
            "Backend '{}' does not expose runtime permission mode updates",
            session.backend_id
        ))
    }

    /// Runtime-owned named permission profile update for already-started sessions.
    async fn set_permission_profile(
        &self,
        session: &SessionHandle,
        cwd: &str,
        permission_mode: &str,
        permission_profile_id: Option<&str>,
    ) -> Result<(), String> {
        let _ = cwd;
        let _ = permission_mode;
        let _ = permission_profile_id;
        Err(format!(
            "Backend '{}' does not expose runtime permission profiles",
            session.backend_id
        ))
    }

    /// バックエンドが選択肢として提供する固定モデル一覧。
    /// `Some` を返すバックエンドは config.toml を参照せず、この一覧を
    /// 表示・検証・モデル解決の供給元とする（完全固定）。
    /// `None`（デフォルト）の場合は config.toml の `agents.<backend>.models` を参照する。
    fn fixed_models(&self) -> Option<Vec<String>> {
        None
    }

    /// Bridge 起動時に必要なバックエンド固有の設定を返す。
    fn runtime_config(
        &self,
        _app_config: Option<&dyn AgentConfigRepository>,
    ) -> BackendRuntimeConfig {
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
    config: Option<Arc<dyn AgentConfigRepository>>,
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
    pub fn set_config(&mut self, config: Arc<dyn AgentConfigRepository>) {
        self.config = Some(config);
    }

    /// 指定 backend の既定モデルを返す。
    /// モデル一覧（`config_models_for` = backend の `fixed_models()` 先頭）の先頭要素を採用する。
    /// 一覧取得に失敗した場合、または一覧が空の場合は `Err` を返す。
    /// 「モデル未選択」状態を廃止するため、新規セッション・既存セッションの lazy 解決の
    /// 双方でこの既定モデルを使う。
    pub fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
        let models = self.config_models_for(backend_id)?;
        models.into_iter().next().ok_or_else(|| {
            format!("バックエンド '{backend_id}' に既定モデルがありません（モデル一覧が空）")
        })
    }

    /// 指定バックエンドのモデルID一覧を取得する。
    /// backend が `fixed_models()` で `Some` を返す場合は config.toml を参照せず
    /// その固定一覧を返す。`None` の場合のみ config 由来の一覧へフォールバックする。
    /// config 未紐付け／未知バックエンドの場合は `Err` を返し、登録済みモデルが
    /// 0 件の場合と区別できるようにする。
    pub fn config_models_for(&self, backend_id: &str) -> Result<Vec<String>, String> {
        if let Some(backend) = self.get(backend_id) {
            if let Some(fixed) = backend.fixed_models() {
                return Ok(fixed);
            }
        }
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| "AppConfig not attached to registry".to_string())?;
        config
            .models_for_backend(backend_id)
            .map_err(|e| e.to_string())
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

    /// 指定バックエンドのモデル一覧を返す。供給元は backend の `fixed_models()`
    /// が `Some` ならその固定一覧、`None` の場合のみ config.toml の
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
            .map(|model_id| ModelInfo::new(id, &model_id))
            .collect())
    }

    pub fn resolve_model_entry(&self, entry_id: &str) -> Result<ModelInfo, String> {
        if let Some((backend_id, model_id)) = entry_id.split_once(':') {
            let parsed_model_id = crate::domain::agent_session::ModelId::parse(model_id)?;
            if self.get(backend_id).is_none() {
                return Err(format!(
                    "バックエンド '{backend_id}' がレジストリに登録されていません"
                ));
            }
            let models = self.config_models_for(backend_id)?;
            if models
                .iter()
                .any(|candidate| candidate == parsed_model_id.as_str())
            {
                return Ok(ModelInfo::new(backend_id, parsed_model_id.as_str()));
            }
            return Err(format!(
                "モデル '{model_id}' はバックエンド '{backend_id}' に登録されていません"
            ));
        }

        let parsed_model_id = crate::domain::agent_session::ModelId::parse(entry_id)?;
        let backend_id = self
            .resolve_backend_for_model(parsed_model_id.as_str())?
            .ok_or_else(|| {
                format!("モデル '{entry_id}' はどのバックエンドにも登録されていません")
            })?;
        Ok(ModelInfo::new(&backend_id, parsed_model_id.as_str()))
    }

    /// Bridge 起動時に必要な backend 固有 runtime config を registry 経由で解決する。
    pub fn runtime_config_for<R: tauri::Runtime>(
        &self,
        id: &str,
        app: &tauri::AppHandle<R>,
    ) -> Result<BackendRuntimeConfig, String> {
        let backend = self
            .get(id)
            .ok_or_else(|| format!("バックエンド '{id}' がレジストリに登録されていません"))?;
        let app_config = app.try_state::<Arc<dyn AgentConfigRepository>>();
        Ok(backend.runtime_config(app_config.as_deref().map(Arc::as_ref)))
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

pub fn list_agent_backends(registry: State<'_, Arc<AgentBackendRegistry>>) -> BackendListResult {
    BackendListResult {
        backends: registry.list(),
        default_id: registry.resolve_default_id().ok(),
    }
}

/// config.toml `[agents]` セクションからレジストリを構築する。
#[allow(dead_code)]
pub fn build_registry(config: Arc<dyn AgentConfigRepository>) -> AgentBackendRegistry {
    build_registry_inner(config, None, None)
}

/// 実アプリ用: CodexBackend に AgentProcess bridge runtime を接続して登録する。
pub fn build_registry_with_runtime(
    config: Arc<dyn AgentConfigRepository>,
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
    config: Arc<dyn AgentConfigRepository>,
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
    if let Ok(default_id) = config.default_agent_backend() {
        registry.set_default(default_id);
    }
    registry.set_config(config);

    registry
}

impl crate::usecase::agent_session::session::SessionBackendResolver for AgentBackendRegistry {
    #[cfg(test)]
    fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
        AgentBackendRegistry::resolve_backend_id(self, backend_id)
    }

    fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
        AgentBackendRegistry::default_model_for(self, backend_id)
    }

    fn backend_exists(&self, backend_id: &str) -> bool {
        self.get(backend_id).is_some()
    }

    fn resolve_default_id(&self) -> Result<String, String> {
        AgentBackendRegistry::resolve_default_id(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::app_config::AppConfig;
    struct MockBackend {
        backend_id: String,
        backend_name: String,
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
        async fn close_session(&self, _session: &SessionHandle) -> Result<(), String> {
            Ok(())
        }
    }

    fn mock_backend(id: &str, name: &str) -> Arc<dyn AgentBackend> {
        Arc::new(MockBackend {
            backend_id: id.to_string(),
            backend_name: name.to_string(),
        })
    }

    fn make_test_app_config() -> Arc<dyn AgentConfigRepository> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Arc::new(AppConfig::new(
            crate::adaptor::gateway::app_config::ReleashConfig::default(),
            tmp.path().to_path_buf(),
        ))
    }

    fn make_test_app_config_with_models(
        claude_models: &[&str],
        codex_models: &[&str],
    ) -> Arc<dyn AgentConfigRepository> {
        let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
        cfg.agents.claude.models = claude_models.iter().map(|s| s.to_string()).collect();
        cfg.agents.codex.models = codex_models.iter().map(|s| s.to_string()).collect();
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
    fn default_model_for_returns_first_claude_fixed_model() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(Arc::new(claude::ClaudeBackend::new()));
        reg.set_config(make_test_app_config());

        assert_eq!(
            reg.default_model_for("claude").unwrap(),
            crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string()
        );
    }

    #[test]
    fn default_model_for_returns_first_codex_fixed_model() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(Arc::new(codex::CodexBackend::new()));
        reg.set_config(make_test_app_config());

        assert_eq!(
            reg.default_model_for("codex").unwrap(),
            crate::domain::agent_session::CODEX_FIXED_MODELS[0].to_string()
        );
    }

    #[test]
    fn default_model_for_errors_when_model_list_empty() {
        // fixed_models を持たない mock backend + 空の config では既定モデルが無くエラー。
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(make_test_app_config());

        assert!(reg.default_model_for("claude").is_err());
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
        let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
        cfg.agents.default = Some("codex".to_string());
        let config: Arc<dyn AgentConfigRepository> = Arc::new(AppConfig::new(
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
        assert_eq!(claude_models[0].id, "claude:opus-4");
        assert_eq!(claude_models[0].display_name, "opus-4");
        assert_eq!(claude_models[0].backend, "claude");
        assert_eq!(claude_models[0].model_id, "opus-4");
        assert_eq!(claude_models[1].model_id, "haiku");
        let codex_models = reg.available_models("codex").unwrap();
        assert_eq!(codex_models.len(), 1);
        assert_eq!(codex_models[0].id, "codex:codex-mini");
        assert_eq!(codex_models[0].backend, "codex");
        assert_eq!(codex_models[0].model_id, "codex-mini");
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
    fn resolve_model_entry_returns_registered_entry_id() {
        let config = make_test_app_config_with_models(&["opus-4"], &["codex-mini"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_config(config);

        let model = reg.resolve_model_entry("codex:codex-mini").unwrap();

        assert_eq!(model.id, "codex:codex-mini");
        assert_eq!(model.backend, "codex");
        assert_eq!(model.model_id, "codex-mini");
    }

    #[test]
    fn resolve_model_entry_errors_for_unregistered_entry_backend() {
        let config = make_test_app_config_with_models(&["opus-4"], &["codex-mini"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.set_config(config);

        let err = reg.resolve_model_entry("missing:codex-mini").unwrap_err();

        assert!(err.contains("missing"));
        assert!(err.contains("レジストリに登録されていません"));
    }

    #[test]
    fn resolve_model_entry_errors_for_model_missing_from_entry_backend_config() {
        let config = make_test_app_config_with_models(&["opus-4"], &["codex-mini"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_config(config);

        let err = reg.resolve_model_entry("codex:not-listed").unwrap_err();

        assert!(err.contains("not-listed"));
        assert!(err.contains("codex"));
        assert!(err.contains("登録されていません"));
    }

    #[test]
    fn resolve_model_entry_resolves_bare_model_id_via_backend_lookup() {
        let config = make_test_app_config_with_models(&["opus-4"], &["codex-mini"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_config(config);

        let model = reg.resolve_model_entry("codex-mini").unwrap();

        assert_eq!(model.id, "codex:codex-mini");
        assert_eq!(model.backend, "codex");
        assert_eq!(model.model_id, "codex-mini");
    }

    #[test]
    fn resolve_model_entry_errors_when_bare_model_id_is_not_registered_anywhere() {
        let config = make_test_app_config_with_models(&["opus-4"], &["codex-mini"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(mock_backend("claude", "Claude"));
        reg.register(mock_backend("codex", "Codex"));
        reg.set_config(config);

        let err = reg.resolve_model_entry("unknown-model").unwrap_err();

        assert!(err.contains("unknown-model"));
        assert!(err.contains("どのバックエンドにも登録されていません"));
    }

    // --- 固定モデル一覧（fixed_models）優先の検証 ---

    #[test]
    fn config_models_for_returns_fixed_list_for_real_claude_backend() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(Arc::new(claude::ClaudeBackend::new()));
        reg.set_config(make_test_app_config());

        let expected: Vec<String> = crate::domain::agent_session::CLAUDE_FIXED_MODELS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(reg.config_models_for("claude").unwrap(), expected);
    }

    #[test]
    fn config_models_for_returns_fixed_list_for_real_codex_backend() {
        let mut reg = AgentBackendRegistry::new();
        reg.register(Arc::new(codex::CodexBackend::new()));
        reg.set_config(make_test_app_config());

        let expected: Vec<String> = crate::domain::agent_session::CODEX_FIXED_MODELS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(reg.config_models_for("codex").unwrap(), expected);
    }

    #[test]
    fn available_models_returns_fixed_list_and_ignores_config_override() {
        // config の agents.*.models に別値を入れても固定一覧が優先される。
        let config = make_test_app_config_with_models(&["should-be-ignored"], &["also-ignored"]);
        let mut reg = AgentBackendRegistry::new();
        reg.register(Arc::new(claude::ClaudeBackend::new()));
        reg.register(Arc::new(codex::CodexBackend::new()));
        reg.set_config(config);

        let claude_values: Vec<String> = reg
            .available_models("claude")
            .unwrap()
            .into_iter()
            .map(|m| m.model_id)
            .collect();
        let expected_claude: Vec<String> = crate::domain::agent_session::CLAUDE_FIXED_MODELS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(claude_values, expected_claude);

        let codex_values: Vec<String> = reg
            .available_models("codex")
            .unwrap()
            .into_iter()
            .map(|m| m.model_id)
            .collect();
        let expected_codex: Vec<String> = crate::domain::agent_session::CODEX_FIXED_MODELS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(codex_values, expected_codex);
    }
}
