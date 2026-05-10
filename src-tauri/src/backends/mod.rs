pub mod bridge_common;
pub mod claude;
pub mod codex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

use crate::config::AppConfig;
use crate::session::SessionStore;

/// Backend-specific runtime values consumed by the generic bridge process runner.
#[derive(Debug, Clone, Default)]
pub struct BackendRuntimeConfig {
    pub initial_model: Option<String>,
    pub bridge_init_options: serde_json::Map<String, serde_json::Value>,
}

/// バックエンドの表示情報。レジストリからUI向けに返却する。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendInfo {
    pub id: String,
    pub name: String,
    pub available: bool,
}

/// モデル情報（共通型）。全バックエンドで使用する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub value: String,
    pub display_name: String,
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

    /// 利用可能なモデル一覧を取得する。
    async fn available_models(&self) -> Result<Vec<ModelInfo>, String>;

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
}

impl AgentBackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
            default_id: None,
        }
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

    /// 指定バックエンドのモデル一覧を返す。
    pub async fn available_models(&self, id: &str) -> Result<Vec<ModelInfo>, String> {
        let backend = self
            .get(id)
            .ok_or_else(|| format!("バックエンド '{id}' がレジストリに登録されていません"))?;
        backend.available_models().await
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
pub fn build_registry(config: &AppConfig) -> AgentBackendRegistry {
    build_registry_inner(config, None)
}

/// 実アプリ用: CodexBackend に AgentProcess bridge runtime を接続して登録する。
pub fn build_registry_with_runtime(
    config: &AppConfig,
    app: tauri::AppHandle,
    handles: Arc<Mutex<bridge_common::AgentProcessMap>>,
    session_store: Arc<SessionStore>,
) -> AgentBackendRegistry {
    build_registry_inner(
        config,
        Some(Arc::new(codex::CodexBackend::with_agent_process_runtime(
            app,
            handles,
            session_store,
        ))),
    )
}

fn build_registry_inner(
    config: &AppConfig,
    codex_backend: Option<Arc<dyn AgentBackend>>,
) -> AgentBackendRegistry {
    let mut registry = AgentBackendRegistry::new();

    // Claude バックエンドは常に利用可能（組み込み）
    let claude = Arc::new(claude::ClaudeBackend::new());
    registry.register(claude);

    // Codex バックエンドもプロジェクト依存として常に利用可能
    let codex = codex_backend.unwrap_or_else(|| Arc::new(codex::CodexBackend::new()));
    registry.register(codex);

    // config.toml の設定を適用
    if let Ok(cfg) = config.get_config() {
        registry.set_default(cfg.agents.default.clone());
    }

    registry
}

#[cfg(test)]
mod tests {
    use super::*;

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
        async fn available_models(&self) -> Result<Vec<ModelInfo>, String> {
            Ok(vec![])
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
        let config = AppConfig::new(
            crate::config::ReleashConfig::default(),
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        );
        let reg = build_registry(&config);
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
        let config = AppConfig::new(cfg, std::path::PathBuf::from("/tmp/test-releash.toml"));
        let reg = build_registry(&config);
        assert_eq!(reg.resolve_default_id().unwrap(), "codex");
    }

    #[test]
    fn build_registry_without_default_config_falls_back_to_first() {
        let config = AppConfig::new(
            crate::config::ReleashConfig::default(),
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        );
        let reg = build_registry(&config);
        assert_eq!(reg.resolve_default_id().unwrap(), "claude");
    }
}
