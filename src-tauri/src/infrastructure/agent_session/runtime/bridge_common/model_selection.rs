use super::process_registry::AgentProcessMap;
use super::session_lifecycle::{
    can_change_session_backend_from_meta, remove_stale_unstarted_agent_process,
};
use super::shared::CLAUDE_BACKEND_ID;
use crate::infrastructure::agent_session::runtime::ModelInfo;
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::session::SessionStore;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub(super) fn available_models_for_backend(
    backend_id: &str,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
) -> Result<Vec<ModelInfo>, String> {
    match registry {
        Some(registry) => registry.available_models(backend_id),
        None => Ok(Vec::new()),
    }
}

/// 既存セッションの `selected_model` を「常に非 null」へ解決する lazy migration ヘルパ。
///
/// モデル「未選択（None）」状態は廃止されたが、`ChatSession.selected_model` の永続化型は
/// 既存 JSON 互換のため `Option<String>` のまま。応答・Bridge 送信時に `None` を backend の
/// 既定モデル（[`crate::infrastructure::agent_session::runtime::AgentBackendRegistry::default_model_for`]）へ解決してから使う。
///
/// - `Some(model)`: そのまま採用する。
/// - `None` + registry あり: 既定モデルに解決する。registry 取得失敗時は warn を残し `None` を返す
///   （表示専用・emit 経路で UI 描画を妨げないため）。
/// - `None` + registry なし（テスト等）: `None` のまま。
pub(super) fn resolve_selected_model(
    selected_model: Option<String>,
    backend_id: &str,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
) -> Option<String> {
    if selected_model.is_some() {
        return selected_model;
    }
    let registry = registry?;
    match registry.default_model_for(backend_id) {
        Ok(model) => Some(model),
        Err(e) => {
            log::warn!("selected_model の既定解決に失敗（backend '{backend_id}'）: {e}");
            None
        }
    }
}

/// 応答（[`GetSessionResponse`]）向けの厳格な `selected_model` 解決。
///
/// 契約: `GetSessionResponse.selected_model`（`ChatSession` から flatten）は常に非 null で
/// シリアライズされる。`ChatSession.selected_model` は `skip_serializing_if = Option::is_none`
/// のため、`None` のまま応答へ載せると JSON からフィールドが脱落し、フロントの必須 `string`
/// 契約に反する。registry が与えられている本番経路では既定モデルへ解決できない場合に `Err` を
/// 返し、フィールド脱落を防ぐ。registry 未指定（テスト等）では `None` のままとし、緩い
/// [`resolve_selected_model`] と挙動を合わせる。
pub(super) fn resolve_selected_model_for_response(
    selected_model: Option<String>,
    backend_id: &str,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
) -> Result<Option<String>, String> {
    if selected_model.is_some() {
        return Ok(selected_model);
    }
    let Some(registry) = registry else {
        return Ok(None);
    };
    registry.default_model_for(backend_id).map(Some)
}

pub(super) fn build_set_model_command(model_id: &str) -> String {
    let cmd = serde_json::json!({
        "type": "setModel",
        "modelId": model_id,
    });
    format!("{}\n", cmd)
}

/// Append text/thinking chunk to streaming parts as an individual part.
/// Each chunk is retained as a separate `MessagePart`; consolidation into
/// merged same-type runs is performed by `consolidate_parts_from_slice` when
pub(super) fn build_agent_models_updated_payload(
    chat_session_id: &str,
    available_models: &[ModelInfo],
    selected_model: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "chat_session_id": chat_session_id,
        "available_models": available_models,
        "selected_model": selected_model,
    })
}

pub async fn set_agent_model(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<
        '_,
        Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    >,
    chat_session_id: String,
    model_id: String,
) -> Result<(), String> {
    set_agent_model_internal(
        &app,
        handles.inner(),
        session_store.inner(),
        Some(registry.inner()),
        &chat_session_id,
        model_id,
    )
    .await
}

pub(super) async fn set_active_process_model(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    model_id: String,
) -> Result<(), String> {
    let data = build_set_model_command(&model_id);
    let stdin = {
        let map = handles.lock().await;
        map.get(chat_session_id)
            .map(|proc| (Arc::clone(&proc.stdin), proc.generation_id))
    };
    if let Some((stdin, generation_id)) = stdin {
        let mut writer = stdin.lock().await;
        writer
            .write_all(data.as_bytes())
            .await
            .map_err(|e| format!("Failed to write setModel: {e}"))?;
        writer
            .flush()
            .await
            .map_err(|e| format!("Failed to flush setModel: {e}"))?;
        drop(writer);

        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            if proc.generation_id == generation_id && Arc::ptr_eq(&proc.stdin, &stdin) {
                proc.selected_model = Some(model_id);
            }
        }
    }
    Ok(())
}

pub(super) async fn set_agent_model_internal(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    chat_session_id: &str,
    model_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(app)?;
    set_agent_model_internal_with_data_dir(
        Some(app),
        handles,
        session_store,
        registry,
        &data_dir,
        chat_session_id,
        model_id,
    )
    .await
}

/// `set_agent_model_internal` のテスト用バリエーション。
/// `tauri::AppHandle` の代わりに `data_dir` を直接受け取り、emit は AppHandle が
/// 渡された場合のみ行う。検証ロジックの単体テストに用いる。
pub(super) async fn set_agent_model_internal_with_data_dir(
    app: Option<&tauri::AppHandle>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_store: &Arc<SessionStore>,
    registry: Option<&Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>,
    data_dir: &Path,
    chat_session_id: &str,
    model_id: String,
) -> Result<(), String> {
    let meta = session_store
        .get_session_meta(data_dir, chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;
    let backend_id = meta
        .backend_id
        .clone()
        .unwrap_or_else(|| CLAUDE_BACKEND_ID.to_string());
    let resolved_model = match registry {
        Some(reg) => reg.resolve_model_entry(&model_id)?,
        None => ModelInfo::new(&backend_id, &model_id),
    };
    let target_backend_id = resolved_model.backend.clone();
    let target_model_id = resolved_model.model_id.clone();

    // モデルは必須。常に形式検証 + 固定リスト照合を通す（モデル未選択状態は廃止）。
    let model = target_model_id.as_str();
    crate::domain::agent_session::ModelId::parse(model)?;
    if target_backend_id != backend_id {
        if !can_change_session_backend_from_meta(&meta) {
            return Err(format!(
                "Cannot change backend after the first message has been sent: {chat_session_id}"
            ));
        }
        remove_stale_unstarted_agent_process(handles, data_dir, chat_session_id).await;
    }
    if let Some(reg) = registry {
        let session_models: Vec<String> =
            reg.config_models_for(&target_backend_id).map_err(|e| {
                log::warn!(
                "set_agent_model: backend '{target_backend_id}' の登録済みモデル一覧取得に失敗: {e}"
            );
                format!(
                    "バックエンド '{target_backend_id}' の登録済みモデル一覧を取得できません: {e}"
                )
            })?;
        if !session_models.iter().any(|v| v == model) {
            // 「未登録」を伝える前に、別バックエンドに登録されていないかを問い合わせる。
            // - Ok(Some(other)) かつ other != current backend: backend mismatch として返す
            // - Ok(Some(same)) / Ok(None): 当該 backend への未登録として返す
            // - Err: infrastructure 故障。warn を残して当該 backend への未登録として返す
            //   （別バックエンドに登録されているかは判定できないため、ヒントは付けない）
            match reg.resolve_backend_for_model(model) {
                Ok(Some(bid)) if bid != target_backend_id => {
                    return Err(format!(
                        "モデル '{model}' はバックエンド '{target_backend_id}' に登録されていません (別バックエンド '{bid}' に登録)"
                    ));
                }
                Ok(_) => {}
                Err(e) => {
                    log::warn!(
                        "set_agent_model: モデル '{model}' の所属バックエンド解決に失敗（未登録として扱う）: {e}"
                    );
                }
            }
            return Err(format!(
                "モデル '{model}' はバックエンド '{target_backend_id}' に登録されていません"
            ));
        }
    }

    // 1. Resolve the config-owned model list before mutating any runtime state.
    //    proc.available_models は config 単一 owner に追従させるため、active process が
    //    存在する場合も config 由来の最新値で同期する。
    //    infrastructure 故障時は Err を伝播し、proc キャッシュを空一覧で上書きしない。
    let models_from_config = available_models_for_backend(&target_backend_id, registry).map_err(|e| {
        log::warn!(
            "set_agent_model: backend '{target_backend_id}' のモデル一覧取得に失敗したため proc キャッシュ同期を中止: {e}"
        );
        format!("バックエンド '{target_backend_id}' のモデル一覧を取得できません: {e}")
    })?;

    // 2. Persist metadata without loading message body.
    session_store.update_backend_selection(
        data_dir,
        chat_session_id,
        target_backend_id.clone(),
        Some(target_model_id.clone()),
    )?;

    // 3. Send setModel command to Bridge + update process state only after persistence succeeds.
    sync_active_process_available_models(handles, chat_session_id, &models_from_config).await;
    set_active_process_model(handles, chat_session_id, target_model_id.clone()).await?;

    // 4. Always emit event to keep frontend in sync.
    //    供給元は常に config.toml（registry 経由）に統一する。
    if let Some(app) = app {
        use tauri::Emitter;
        let _ = app.emit(
            "agent-models-updated",
            build_agent_models_updated_payload(
                chat_session_id,
                &models_from_config,
                Some(resolved_model.id.as_str()),
            ),
        );
    }

    Ok(())
}

/// active process の `available_models` キャッシュを config 由来の最新値で同期する。
/// 永続的なモデル一覧の owner は config.toml 単一であり、process 側のキャッシュは
/// emit 整合用にのみ維持する。
pub(super) async fn sync_active_process_available_models(
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    models: &[ModelInfo],
) {
    let mut map = handles.lock().await;
    if let Some(proc) = map.get_mut(chat_session_id) {
        proc.available_models = models.to_vec();
    }
}
#[cfg(test)]
mod moved_tests {

    use super::super::model_selection::*;

    use super::super::process_registry::*;

    use super::super::shared::test_support::*;
    use super::super::shared::*;

    use crate::infrastructure::agent_session::runtime::{AgentBackendRegistry, ModelInfo};

    use crate::usecase::agent_session::session::{
        add_message_internal, create_session_internal, MessageRole,
    };

    use std::sync::Arc;

    use tokio::sync::Mutex;

    #[test]
    fn available_models_for_backend_reads_from_config_via_registry() {
        let mut cfg = crate::adaptor::gateway::app_config::ReleashConfig::default();
        cfg.agents.claude.models = vec!["mock-model".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            cfg,
            tmp.path().to_path_buf(),
        ));

        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
        }));
        registry.set_config(config);
        let registry = Arc::new(registry);

        let models = available_models_for_backend(CLAUDE_BACKEND_ID, Some(&registry)).unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "claude:mock-model");
        assert_eq!(models[0].model_id, "mock-model");
    }

    #[test]
    fn available_models_for_backend_propagates_registry_error() {
        // registry に config が未紐付けの状態では Err が返り、空配列で潰れない。
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(MockModelBackend {
            backend_id: CLAUDE_BACKEND_ID.to_string(),
        }));
        let registry = Arc::new(registry);
        let err = available_models_for_backend(CLAUDE_BACKEND_ID, Some(&registry));
        assert!(err.is_err(), "config 未紐付けは Err として伝播する");
    }

    #[test]
    fn available_models_for_backend_returns_empty_without_registry() {
        // registry が無い経路（テスト等）は Ok(empty) として扱う。
        let models = available_models_for_backend(CLAUDE_BACKEND_ID, None).unwrap();
        assert!(models.is_empty());
    }

    #[tokio::test]
    async fn set_agent_model_accepts_registered_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &["gpt-5"]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "claude-4".to_string(),
        )
        .await
        .unwrap();

        let updated = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some("claude-4".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_does_not_read_message_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();
        let chunk = temp
            .path()
            .join("sessions")
            .join(&session.id)
            .join("messages")
            .join("1.json");
        std::fs::write(chunk, "{not valid json").unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "claude-4".to_string(),
        )
        .await
        .unwrap();

        let updated = session_store
            .get_session_meta(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some("claude-4".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_preserves_surrounding_whitespace() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let model = "  claude-4  ";
        let registry = make_test_registry_with_models(&[model], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            model.to_string(),
        )
        .await
        .unwrap();

        let updated = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some(model.to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_unregistered_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = Some("existing".to_string());
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let registry = make_test_registry_with_models(&["claude-4"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "unknown".to_string(),
        )
        .await;
        assert!(err.is_err());

        // 拒否時は selected_model を維持
        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, Some("existing".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_allows_other_backend_model_before_first_message() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &["gpt-5"]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "gpt-5".to_string(),
        )
        .await
        .unwrap();

        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.backend_id.as_deref(), Some(CODEX_BACKEND_ID));
        assert_eq!(after.selected_model.as_deref(), Some("gpt-5"));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_backend_change_after_first_message() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        add_message_internal(
            &session_store,
            temp.path(),
            &session.id,
            MessageRole::Human,
            "hello",
            None,
            None,
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &["gpt-5"]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "gpt-5".to_string(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("Cannot change backend"));

        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.backend_id.as_deref(), Some(CLAUDE_BACKEND_ID));
        assert_eq!(after.selected_model, None);
    }

    #[tokio::test]
    async fn set_agent_model_rejects_empty_model() {
        // モデルは必須。空文字は形式不正として登録判定の前に拒否する。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let mut session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        session.selected_model = Some("claude-4".to_string());
        session_store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let registry = make_test_registry_with_models(&["claude-4"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            String::new(),
        )
        .await;
        assert!(err.is_err());

        // 拒否時は既存の selected_model を維持
        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, Some("claude-4".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_accepts_claude_fixed_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let model = crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string();

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            model.clone(),
        )
        .await
        .unwrap();

        let updated = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some(model));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_model_outside_claude_fixed_list() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "not-a-fixed-claude-model".to_string(),
        )
        .await;
        assert!(err.is_err());

        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, None);
    }

    #[tokio::test]
    async fn set_agent_model_accepts_codex_fixed_model() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let model = crate::domain::agent_session::CODEX_FIXED_MODELS[0].to_string();

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            model.clone(),
        )
        .await
        .unwrap();

        let updated = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.selected_model, Some(model));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_model_outside_codex_fixed_list() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CODEX_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_fixed_model_registry();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "not-a-fixed-codex-model".to_string(),
        )
        .await;
        assert!(err.is_err());

        let after = session_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, None);
    }

    #[test]
    fn build_agent_models_updated_payload_emits_event_contract_fields() {
        let models = vec![
            ModelInfo::new(CLAUDE_BACKEND_ID, "a"),
            ModelInfo::new(CLAUDE_BACKEND_ID, "b"),
        ];
        let payload = build_agent_models_updated_payload("sess-1", &models, Some("a"));

        assert_eq!(payload["chat_session_id"], "sess-1");
        let candidates = payload["available_models"]
            .as_array()
            .expect("available_models is array");
        let values: Vec<String> = candidates
            .iter()
            .map(|v| v["modelId"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(values, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(candidates[0]["id"], "claude:a");
        assert_eq!(candidates[0]["displayName"], "a");
        assert_eq!(candidates[0]["backend"], "claude");
        assert_eq!(payload["selected_model"], "a");
    }

    #[test]
    fn build_agent_models_updated_payload_carries_selected_model_non_null() {
        // モデル未選択状態は廃止。set_agent_model は常に非 null の selected_model を emit する。
        let payload =
            build_agent_models_updated_payload("sess-2", &[], Some("claude:claude-opus-4-8"));
        assert_eq!(payload["selected_model"], "claude:claude-opus-4-8");
        assert_eq!(payload["available_models"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn resolve_selected_model_for_response_keeps_existing_value() {
        let registry = make_test_registry_with_models(&[], &[]);
        let resolved = resolve_selected_model_for_response(
            Some("explicit-model".to_string()),
            CLAUDE_BACKEND_ID,
            Some(&registry),
        )
        .unwrap();
        assert_eq!(resolved, Some("explicit-model".to_string()));
    }

    #[test]
    fn resolve_selected_model_for_response_resolves_default_when_none() {
        let registry = make_fixed_model_registry();
        let resolved =
            resolve_selected_model_for_response(None, CLAUDE_BACKEND_ID, Some(&registry)).unwrap();
        assert_eq!(
            resolved,
            Some(crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string())
        );
    }

    #[test]
    fn resolve_selected_model_for_response_errors_when_unresolvable() {
        let registry = make_test_registry_with_models(&[], &[]);
        let result = resolve_selected_model_for_response(None, CLAUDE_BACKEND_ID, Some(&registry));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_selected_model_for_response_keeps_none_without_registry() {
        let resolved = resolve_selected_model_for_response(None, CLAUDE_BACKEND_ID, None).unwrap();
        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn set_agent_model_syncs_active_process_available_models_from_config() {
        // active process が居る状態で set_agent_model を呼ぶと、proc.available_models が
        // config 由来の最新値で同期される（spec: モデル選択候補は config 単一 owner、
        // process キャッシュは emit 整合用にのみ維持）。
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4", "haiku"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        {
            let mut map = handles.lock().await;
            let mut proc = make_test_agent_process();
            proc.backend_id = CLAUDE_BACKEND_ID.to_string();
            // process cache が stale な状態
            proc.available_models = vec![ModelInfo::new(CLAUDE_BACKEND_ID, "stale")];
            map.insert(session.id.clone(), proc);
        }

        set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "claude-4".to_string(),
        )
        .await
        .unwrap();

        // proc.available_models が registry/config 由来の最新値に同期される
        let map = handles.lock().await;
        let proc = map.get(&session.id).unwrap();
        let values: Vec<String> = proc
            .available_models
            .iter()
            .map(|m| m.model_id.clone())
            .collect();
        assert_eq!(values, vec!["claude-4".to_string(), "haiku".to_string()]);
        // selected_model も反映される
        assert_eq!(proc.selected_model, Some("claude-4".to_string()));
    }

    #[tokio::test]
    async fn set_agent_model_rejects_invalid_format_before_registry_check() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some(CLAUDE_BACKEND_ID.to_string()),
        )
        .unwrap();
        let registry = make_test_registry_with_models(&["claude-4"], &[]);
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        // 制御文字（形式不正）は登録判定に進む前に拒否
        let err = set_agent_model_internal_with_data_dir(
            None,
            &handles,
            &session_store,
            Some(&registry),
            temp.path(),
            &session.id,
            "bad\u{0001}model".to_string(),
        )
        .await;
        assert!(err.is_err());
    }

    #[test]
    fn set_model_command_format() {
        let result = build_set_model_command("claude-opus");
        let cmd: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert_eq!(cmd["type"], "setModel");
        assert_eq!(cmd["modelId"], "claude-opus");
    }
}
