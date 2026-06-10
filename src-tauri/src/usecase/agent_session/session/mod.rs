pub(crate) mod errors;
pub(crate) mod lifecycle_controller;
mod open_tabs;
mod store;

use serde::{Deserialize, Serialize};

pub use crate::usecase::agent_session::status::TurnPhase;
pub use open_tabs::OpenTabRegistry;
pub use store::SessionStore;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
    Thinking {
        content: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    Text {
        content: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    ToolUse {
        tool: String,
        input: serde_json::Value,
        id: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    ToolResult {
        content: String,
        #[serde(rename = "isError")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "toolUseId")]
        tool_use_id: Option<String>,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    Error {
        content: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    Permission {
        request: serde_json::Value,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        answers: Option<serde_json::Value>,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    TaskStatus {
        #[serde(rename = "taskToolUseId")]
        task_tool_use_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        summary: Option<String>,
    },
    SystemNotification {
        #[serde(rename = "notificationType")]
        notification_type: String,
        status: String,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "hookId")]
        hook_id: Option<String>,
    },
    Image {
        /// Base64-encoded image data
        data: String,
        /// MIME type (e.g. "image/png", "image/jpeg")
        #[serde(rename = "mediaType")]
        media_type: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    Human,
    Agent,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Idle,
    Done,
    Error,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityEntry {
    ToolUse {
        tool: String,
        input: serde_json::Value,
        id: String,
    },
    ToolResult {
        content: String,
        #[serde(rename = "isError")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "toolUseId")]
        tool_use_id: Option<String>,
    },
    PermissionResult {
        #[serde(rename = "toolName")]
        tool_name: String,
        status: String,
        summary: String,
    },
}

pub trait SessionBackendResolver {
    fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String>;
    fn default_model_for(&self, backend_id: &str) -> Result<String, String>;
    fn backend_exists(&self, backend_id: &str) -> bool;
    fn resolve_default_id(&self) -> Result<String, String>;
}

impl<T> SessionBackendResolver for std::sync::Arc<T>
where
    T: SessionBackendResolver + ?Sized,
{
    fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
        self.as_ref().resolve_backend_id(backend_id)
    }

    fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
        self.as_ref().default_model_for(backend_id)
    }

    fn backend_exists(&self, backend_id: &str) -> bool {
        self.as_ref().backend_exists(backend_id)
    }

    fn resolve_default_id(&self) -> Result<String, String> {
        self.as_ref().resolve_default_id()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub activities: Option<Vec<ActivityEntry>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parts: Option<Vec<MessagePart>>,
    pub timestamp: f64,
    /// 永続化／フロント転送向けの表現。domain VO は serde 非依存（純粋データ）のため、
    /// 転送境界に置く本フィールドは adaptor の入出力型を保持する。serialize 表現
    /// （camelCase・行範囲省略）は移行前と等価。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mentions: Option<Vec<crate::adaptor::protocol::mention::MentionReferenceInput>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    pub id: String,
    pub worktree_path: String,
    pub messages: Vec<ChatMessage>,
    pub state: SessionState,
    pub created_at: f64,
    pub updated_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_session_id: Option<String>,
    /// 抽象モード文字列（"ask" / "edit" / "full"）。
    /// serde の default を意図的に付けない: 保存済みセッションで欠落していた場合は
    /// デシリアライズエラーで起動を拒否する（破壊的変更、Spec issues-947 参照）。
    pub permission_mode: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub selected_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub workflow_step_session: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSessionResponse {
    #[serde(flatten)]
    pub session: ChatSession,
    pub turn_phase: TurnPhase,
    pub available_models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub worktree_path: String,
    pub state: SessionState,
    pub created_at: f64,
    pub updated_at: f64,
    pub first_message: String,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_session_id: Option<String>,
    pub permission_mode: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub workflow_step_session: bool,
}

impl ChatSession {
    pub fn to_summary(&self) -> SessionSummary {
        let first_message = self
            .messages
            .first()
            .map(|m| {
                let content = if m.content.is_empty() {
                    if m.parts.as_ref().is_some_and(|parts| {
                        parts.iter().any(|p| matches!(p, MessagePart::Image { .. }))
                    }) {
                        "[Image]".to_string()
                    } else {
                        m.content.clone()
                    }
                } else {
                    m.content.clone()
                };
                match content.char_indices().nth(100) {
                    Some((byte_pos, _)) => format!("{}…", &content[..byte_pos]),
                    None => content,
                }
            })
            .unwrap_or_default();

        SessionSummary {
            id: self.id.clone(),
            worktree_path: self.worktree_path.clone(),
            state: self.state.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            first_message,
            message_count: self.messages.len(),
            agent_session_id: self.agent_session_id.clone(),
            permission_mode: self.permission_mode.clone(),
            backend_id: self.backend_id.clone(),
            workflow_step_session: self.workflow_step_session,
        }
    }
}

pub(crate) fn now_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn parts_to_legacy(
    parts: &[MessagePart],
) -> (String, Option<String>, Option<Vec<ActivityEntry>>) {
    let mut content = String::new();
    let mut thinking = String::new();
    let mut activities: Vec<ActivityEntry> = Vec::new();
    for part in parts {
        match part {
            MessagePart::Text { content: c, .. } => content.push_str(c),
            MessagePart::Error { content: c, .. } => content.push_str(c),
            MessagePart::Thinking { content: c, .. } => thinking.push_str(c),
            MessagePart::ToolUse {
                tool, input, id, ..
            } => {
                activities.push(ActivityEntry::ToolUse {
                    tool: tool.clone(),
                    input: input.clone(),
                    id: id.clone(),
                });
            }
            MessagePart::ToolResult {
                content: c,
                is_error,
                tool_use_id,
                ..
            } => {
                activities.push(ActivityEntry::ToolResult {
                    content: c.clone(),
                    is_error: *is_error,
                    tool_use_id: tool_use_id.clone(),
                });
            }
            MessagePart::Permission {
                request,
                status,
                answers,
                ..
            } => {
                if status != "pending" {
                    let tool_name = request
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let summary = answers
                        .as_ref()
                        .and_then(|a| a.as_object())
                        .map(|obj| {
                            obj.values()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_else(|| status.clone());
                    activities.push(ActivityEntry::PermissionResult {
                        tool_name,
                        status: status.clone(),
                        summary,
                    });
                }
            }
            MessagePart::TaskStatus { .. } => {}
            MessagePart::SystemNotification { .. } => {}
            MessagePart::Image { .. } => {}
        }
    }
    let thinking = if thinking.is_empty() {
        None
    } else {
        Some(thinking)
    };
    let activities = if activities.is_empty() {
        None
    } else {
        Some(activities)
    };
    (content, thinking, activities)
}

/// Internal (non-command) version of create_session, callable from agent_sdk.
/// `permission_mode` 未指定の経路（ワークフロー engine 起点の step session 等）向けに
/// `PermissionMode::Edit` を既定値として用いる。検証済み抽象モードを保有する経路
/// （WS handler / message → 新規 session）は [`create_session_internal_with_permission`] を呼ぶこと。
#[cfg(test)]
pub fn create_session_internal(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: Option<String>,
) -> Result<ChatSession, String> {
    create_session_internal_with_permission(
        session_store,
        data_dir,
        worktree_path,
        backend_id,
        crate::permission::PermissionMode::Edit,
    )
}

/// 検証済みの抽象 [`crate::permission::PermissionMode`] を初回保存で確定するセッション生成 API。
/// WS handler や message → 新規 session 経路から呼び、edit デフォルトで保存→update の二段階を回避する
/// （Spec issues-947: セッション保存層が permission_mode の正典）。
pub fn create_session_internal_with_permission(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: Option<String>,
    permission_mode: crate::permission::PermissionMode,
) -> Result<ChatSession, String> {
    create_session_internal_with_attributes(
        session_store,
        data_dir,
        worktree_path,
        backend_id,
        permission_mode,
        None,
        false,
    )
}

/// 検証済み抽象モード・selected_model・workflow_step_session フラグを初回保存で確定する内部 API。
/// ワークフロー engine の step session 生成経路から呼び、edit デフォルトで保存→属性上書きの
/// 二段階保存を回避する（Spec issues-947: 途中失敗時の不正中間状態の排除）。
pub fn create_session_internal_with_attributes(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: Option<String>,
    permission_mode: crate::permission::PermissionMode,
    selected_model: Option<String>,
    workflow_step_session: bool,
) -> Result<ChatSession, String> {
    let session = build_new_session(
        worktree_path,
        backend_id,
        permission_mode,
        selected_model,
        workflow_step_session,
    );
    session_store.save_session(data_dir, &session)?;
    Ok(session)
}

fn build_new_session(
    worktree_path: &str,
    backend_id: Option<String>,
    permission_mode: crate::permission::PermissionMode,
    selected_model: Option<String>,
    workflow_step_session: bool,
) -> ChatSession {
    let now = now_timestamp();
    ChatSession {
        id: uuid::Uuid::new_v4().to_string(),
        worktree_path: worktree_path.to_string(),
        messages: Vec::new(),
        state: SessionState::Active,
        created_at: now,
        updated_at: now,
        agent_session_id: None,
        permission_mode: permission_mode.as_str().to_string(),
        selected_model,
        backend_id,
        workflow_step_session,
    }
}

/// 新規セッションを作成し、当該 backend の既定モデルを `selected_model` に永続化する。
///
/// モデル「未選択（None）」状態は廃止したため、新規セッションは常に backend の既定モデル
/// （[`SessionBackendResolver::default_model_for`] = 固定リスト先頭）を
/// `selected_model` に持つ。既定モデルが解決できない場合はセッション作成エラーとする。
///
/// `permission_mode` は検証済みの抽象 [`crate::permission::PermissionMode`] を要求し、
/// 初回保存で確定する（Spec issues-947: セッション保存層が permission_mode の正典）。
pub fn create_session_with_initial_model(
    session_store: &SessionStore,
    registry: &impl SessionBackendResolver,
    data_dir: &std::path::Path,
    worktree_path: &str,
    backend_id: String,
    permission_mode: crate::permission::PermissionMode,
) -> Result<ChatSession, String> {
    let default_model = registry.default_model_for(&backend_id)?;
    let session = build_new_session(
        worktree_path,
        Some(backend_id),
        permission_mode,
        Some(default_model),
        false,
    );
    session_store.save_session(data_dir, &session)?;
    Ok(session)
}

/// Internal (non-command) version of add_message, callable from agent_sdk.
pub fn add_message_internal(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
    role: MessageRole,
    content: &str,
    parts: Option<Vec<MessagePart>>,
    mentions: Option<Vec<crate::domain::code::MentionReference>>,
) -> Result<ChatMessage, String> {
    let mut session = session_store
        .get_session(data_dir, session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    let now = now_timestamp();
    // 永続化境界（ChatMessage の serde JSON）では adaptor の Input 型へ詰め替える。
    // serialize 表現（camelCase・行範囲省略）は移行前と等価。
    let mentions_for_persist = mentions.map(|v| {
        v.into_iter()
            .map(crate::adaptor::protocol::mention::MentionReferenceInput::from_domain)
            .collect::<Vec<_>>()
    });
    let message = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content: content.to_string(),
        thinking: None,
        activities: None,
        parts,
        timestamp: now,
        mentions: mentions_for_persist,
    };
    session.messages.push(message.clone());
    session.updated_at = now;
    session_store.save_session(data_dir, &session)?;
    Ok(message)
}

pub(crate) fn create_session_command_inner(
    session_store: &SessionStore,
    registry: &impl SessionBackendResolver,
    data_dir: &std::path::Path,
    worktree_path: &str,
    permission_mode: &str,
    backend_id: Option<String>,
) -> Result<ChatSession, String> {
    let permission_mode =
        crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;
    let resolved_backend_id = registry.resolve_backend_id(backend_id)?;
    create_session_with_initial_model(
        session_store,
        registry,
        data_dir,
        worktree_path,
        resolved_backend_id,
        permission_mode,
    )
}

pub(crate) fn update_session_state_in_data_dir(
    state: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
    new_state: SessionState,
) -> Result<(), String> {
    let mut session = state
        .get_session(data_dir, session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    if session.workflow_step_session && session.state == SessionState::Closed {
        return Ok(());
    }
    session.state = new_state;
    session.updated_at = now_timestamp();
    state.save_session(data_dir, &session)?;
    Ok(())
}

/// セッション復元時の backend_id 検証・解決ロジック。
/// - backend_id が Some かつ registry に存在 → Ok
/// - backend_id が Some だが registry に不在 → Err
/// - backend_id が None → デフォルトを代入して Ok
pub fn resolve_session_backend(
    session: &mut ChatSession,
    registry: &impl SessionBackendResolver,
) -> Result<(), String> {
    if let Some(ref bid) = session.backend_id {
        if !registry.backend_exists(bid) {
            return Err(format!(
                "バックエンド '{}' がレジストリに登録されていません",
                bid
            ));
        }
    } else {
        let default_id = registry.resolve_default_id()?;
        session.backend_id = Some(default_id);
    }
    Ok(())
}

/// セッション起動時の permission_mode 検証。
/// 対象外の値（旧語彙 acceptEdits / bypassPermissions / plan / default、未知語彙、空文字）が
/// 保存されていた場合はバリデーションエラーで拒否し、ユーザに手動更新を求める（破壊的変更）。
pub fn validate_session_permission_mode(session: &ChatSession) -> Result<(), String> {
    crate::permission::PermissionMode::parse(&session.permission_mode)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSessionResponse {
    pub restored_workflow_step: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use crate::workflow::schema::{NodeDefinition, NodeType, Workflow};
    use crate::workflow::state::{
        StepHistoryEntry, TokenUsage, WorkflowExecutionState, WorkflowState,
    };

    #[derive(Default)]
    struct TestBackendResolver {
        default_id: Option<String>,
        models: BTreeMap<String, String>,
        existing: BTreeSet<String>,
    }

    impl TestBackendResolver {
        fn with_backend(mut self, backend_id: &str, default_model: &str) -> Self {
            self.existing.insert(backend_id.to_string());
            self.models
                .insert(backend_id.to_string(), default_model.to_string());
            self
        }

        fn with_default(mut self, backend_id: &str) -> Self {
            self.default_id = Some(backend_id.to_string());
            self
        }
    }

    impl SessionBackendResolver for TestBackendResolver {
        fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
            match backend_id {
                Some(id) if self.backend_exists(&id) => Ok(id),
                Some(id) => Err(format!(
                    "Backend '{}' not found. Available backends: claude, codex",
                    id
                )),
                None => self.resolve_default_id(),
            }
        }

        fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
            self.models
                .get(backend_id)
                .cloned()
                .ok_or_else(|| format!("No models configured for backend '{backend_id}'"))
        }

        fn backend_exists(&self, backend_id: &str) -> bool {
            self.existing.contains(backend_id)
        }

        fn resolve_default_id(&self) -> Result<String, String> {
            self.default_id
                .clone()
                .ok_or_else(|| "No default backend configured".to_string())
        }
    }

    #[test]
    fn chat_message_mentions_persist_serializeは移行前のcamelcase等価() {
        // domain VO `MentionReference` から serde を剥がしたため、永続化境界（ChatMessage）
        // の `mentions` は adaptor の `MentionReferenceInput` を保持する。serialize 表現
        // （camelCase / 行範囲省略）が移行前と等価であることを担保する。
        use crate::adaptor::protocol::mention::MentionReferenceInput;

        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Human,
            content: "hello".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            timestamp: 1.0,
            mentions: Some(vec![
                MentionReferenceInput {
                    file_path: "src/a.rs".to_string(),
                    start_line: None,
                    end_line: None,
                },
                MentionReferenceInput {
                    file_path: "src/b.rs".to_string(),
                    start_line: Some(3),
                    end_line: Some(5),
                },
            ]),
        };
        let v = serde_json::to_value(&msg).unwrap();
        let mentions = &v["mentions"];
        assert_eq!(mentions[0]["filePath"], serde_json::json!("src/a.rs"));
        assert!(mentions[0].get("startLine").is_none());
        assert!(mentions[0].get("endLine").is_none());
        assert_eq!(mentions[1]["filePath"], serde_json::json!("src/b.rs"));
        assert_eq!(mentions[1]["startLine"], serde_json::json!(3));
        assert_eq!(mentions[1]["endLine"], serde_json::json!(5));

        // None の場合は mentions キー自体が省略される（移行前と等価）。
        let msg_none = ChatMessage {
            mentions: None,
            ..msg
        };
        let v = serde_json::to_value(&msg_none).unwrap();
        assert!(v.get("mentions").is_none());
    }

    #[test]
    fn chat_session_missing_permission_mode_rejected_on_deserialize() {
        // Spec issues-947: 保存済みセッションで permissionMode フィールドが欠落していた場合は、
        // serde default で補完せず、デシリアライズエラーで起動を拒否する（破壊的変更）。
        let json = r#"{"id":"s1","worktreePath":"/repo","messages":[],"state":"active","createdAt":1000.0,"updatedAt":1000.0}"#;
        let err = serde_json::from_str::<ChatSession>(json).unwrap_err();
        assert!(
            err.to_string().contains("permissionMode"),
            "missing permissionMode must be rejected, got: {err}"
        );
    }

    #[test]
    fn chat_session_legacy_permission_mode_rejected_by_validation() {
        // 保存済みセッションが旧語彙や未知語彙を持っていた場合、validate_session_permission_mode が拒否する。
        for legacy in [
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
            "unknown",
            "",
        ] {
            let session = ChatSession {
                id: "s1".to_string(),
                worktree_path: "/repo".to_string(),
                messages: vec![],
                state: SessionState::Active,
                created_at: 1000.0,
                updated_at: 1000.0,
                agent_session_id: None,
                permission_mode: legacy.to_string(),
                selected_model: None,
                backend_id: None,
                workflow_step_session: false,
            };
            let err = validate_session_permission_mode(&session).unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "legacy '{legacy}' must be rejected with allowed list, got: {err}"
            );
        }
    }

    #[test]
    fn chat_session_to_summary_basic() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: "Hello agent".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
        };
        let summary = session.to_summary();
        assert_eq!(summary.id, "s1");
        assert_eq!(summary.first_message, "Hello agent");
        // Verify selected_model not in summary (summary doesn't expose model)
        assert_eq!(summary.message_count, 1);
    }

    #[test]
    fn chat_session_to_summary_truncates_long_message() {
        let long_content = "a".repeat(200);
        let session = ChatSession {
            id: "s2".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: long_content,
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Idle,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
        };
        let summary = session.to_summary();
        assert_eq!(summary.first_message.len(), 100 + "…".len());
        assert!(summary.first_message.ends_with('…'));
    }

    #[test]
    fn chat_session_to_summary_truncates_multibyte_message() {
        // 200 Japanese characters (3 bytes each in UTF-8)
        let long_content = "あ".repeat(200);
        let session = ChatSession {
            id: "s2mb".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: long_content,
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Idle,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
        };
        let summary = session.to_summary();
        // 100 chars of "あ" (300 bytes) + "…" (3 bytes)
        assert_eq!(summary.first_message.chars().count(), 101); // 100 + 1 for "…"
        assert!(summary.first_message.ends_with('…'));
        assert!(summary.first_message.starts_with("あ"));
    }

    #[test]
    fn chat_session_to_summary_empty_messages() {
        let session = ChatSession {
            id: "s3".to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: SessionState::Done,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
        };
        let summary = session.to_summary();
        assert_eq!(summary.first_message, "");
        assert_eq!(summary.message_count, 0);
    }

    #[test]
    fn generic_state_update_ignores_closed_workflow_step_session_but_updates_regular_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = SessionStore::default();

        let workflow_step_id = uuid::Uuid::new_v4().to_string();
        let regular_id = uuid::Uuid::new_v4().to_string();
        let mut workflow_step = ChatSession {
            id: workflow_step_id.clone(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: SessionState::Closed,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: Some("agent-session".to_string()),
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: None,
            workflow_step_session: true,
        };
        let mut regular = workflow_step.clone();
        regular.id = regular_id.clone();
        regular.workflow_step_session = false;

        store.save_session(tmp.path(), &workflow_step).unwrap();
        store.save_session(tmp.path(), &regular).unwrap();

        update_session_state_in_data_dir(&store, tmp.path(), &workflow_step.id, SessionState::Idle)
            .unwrap();
        update_session_state_in_data_dir(&store, tmp.path(), &regular.id, SessionState::Idle)
            .unwrap();

        workflow_step = store
            .get_session(tmp.path(), &workflow_step_id)
            .unwrap()
            .unwrap();
        let regular = store.get_session(tmp.path(), &regular_id).unwrap().unwrap();
        assert_eq!(workflow_step.state, SessionState::Closed);
        assert_eq!(regular.state, SessionState::Idle);
    }

    #[test]
    fn message_role_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&MessageRole::Human).unwrap(),
            "\"human\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Agent).unwrap(),
            "\"agent\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn session_state_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SessionState::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Idle).unwrap(),
            "\"idle\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Done).unwrap(),
            "\"done\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Error).unwrap(),
            "\"error\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Closed).unwrap(),
            "\"closed\""
        );
    }

    #[test]
    fn chat_message_thinking_field_serialization() {
        let msg_with = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "response".to_string(),
            thinking: Some("deep thought".to_string()),
            activities: None,
            parts: None,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg_with).unwrap();
        assert!(json.contains("\"thinking\":\"deep thought\""));
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.thinking, Some("deep thought".to_string()));

        let msg_without = ChatMessage {
            id: "m2".to_string(),
            role: MessageRole::Agent,
            content: "response".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg_without).unwrap();
        assert!(!json.contains("thinking"));
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.thinking, None);
    }

    #[test]
    fn chat_message_without_thinking_field_deserializes() {
        let json = r#"{"id":"m1","role":"agent","content":"hello","timestamp":1000.0}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.thinking, None);
    }

    #[test]
    fn chat_session_roundtrip() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![
                ChatMessage {
                    id: "m1".to_string(),
                    role: MessageRole::Human,
                    content: "Hello".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    timestamp: 1000.0,
                    mentions: None,
                },
                ChatMessage {
                    id: "m2".to_string(),
                    role: MessageRole::Agent,
                    content: "Hi there!".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    timestamp: 1001.0,
                    mentions: None,
                },
            ],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
        };
        let json = serde_json::to_string(&session).unwrap();
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "s1");
        assert_eq!(back.messages.len(), 2);
        assert_eq!(back.messages[0].role, MessageRole::Human);
        assert_eq!(back.messages[1].role, MessageRole::Agent);
    }

    #[test]
    fn chat_session_without_selected_model_deserializes() {
        let json = r#"{"id":"s1","worktreePath":"/repo","messages":[],"state":"active","createdAt":1000.0,"updatedAt":1000.0,"permissionMode":"edit"}"#;
        let session: ChatSession = serde_json::from_str(json).unwrap();
        assert_eq!(session.selected_model, None);
    }

    #[test]
    fn chat_session_roundtrip_with_selected_model() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: Some("claude-opus-4-6".to_string()),
            backend_id: None,
            workflow_step_session: false,
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("selectedModel"));
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.selected_model, Some("claude-opus-4-6".to_string()));
    }

    #[test]
    fn activity_entry_tool_use_serialization() {
        let entry = ActivityEntry::ToolUse {
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "/src/main.ts"}),
            id: "toolu_001".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "tool_use");
        assert_eq!(v["tool"], "Read");
        assert_eq!(v["id"], "toolu_001");
        assert_eq!(v["input"]["file_path"], "/src/main.ts");
    }

    #[test]
    fn activity_entry_tool_result_serialization() {
        let entry = ActivityEntry::ToolResult {
            content: "file contents".to_string(),
            is_error: false,
            tool_use_id: Some("toolu_001".into()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "tool_result");
        assert_eq!(v["content"], "file contents");
        assert_eq!(v["isError"], false);
        assert_eq!(v["toolUseId"], "toolu_001");
    }

    #[test]
    fn activity_entry_permission_result_serialization() {
        let entry = ActivityEntry::PermissionResult {
            tool_name: "Bash".to_string(),
            status: "allowed".to_string(),
            summary: "Bash: allowed".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "permission_result");
        assert_eq!(v["toolName"], "Bash");
        assert_eq!(v["status"], "allowed");
        assert_eq!(v["summary"], "Bash: allowed");

        let back: ActivityEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn activity_entry_permission_result_backward_compat() {
        // Existing session files without permission_result should still deserialize
        let json = r#"{"type":"tool_use","tool":"Read","input":{},"id":"t1"}"#;
        let entry: ActivityEntry = serde_json::from_str(json).unwrap();
        assert!(matches!(entry, ActivityEntry::ToolUse { .. }));
    }

    #[test]
    fn chat_message_without_activities_field_deserializes() {
        let json = r#"{"id":"m1","role":"agent","content":"hello","timestamp":1000.0}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.activities, None);
    }

    #[test]
    fn chat_message_with_activities_roundtrip() {
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "done".to_string(),
            thinking: None,
            activities: Some(vec![
                ActivityEntry::ToolUse {
                    tool: "Read".to_string(),
                    input: serde_json::json!({}),
                    id: "t1".to_string(),
                },
                ActivityEntry::ToolResult {
                    content: "ok".to_string(),
                    is_error: false,
                    tool_use_id: None,
                },
            ]),
            parts: None,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.activities.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn message_part_serde_roundtrip() {
        let parts = vec![
            MessagePart::Thinking {
                content: "hmm".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Error {
                content: "something went wrong".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Read".to_string(),
                input: serde_json::json!({"file_path": "/a.ts"}),
                id: "t1".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolResult {
                content: "ok".to_string(),
                is_error: false,
                tool_use_id: None,
                parent_tool_use_id: None,
            },
            MessagePart::Permission {
                request: serde_json::json!({"request_id": "r1", "tool_name": "Bash"}),
                status: "allowed".to_string(),
                answers: Some(serde_json::json!({"q1": "yes"})),
                parent_tool_use_id: None,
            },
        ];
        let json = serde_json::to_string(&parts).unwrap();
        let back: Vec<MessagePart> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 6);
        assert_eq!(back, parts);
    }

    #[test]
    fn message_part_error_serialization() {
        let part = MessagePart::Error {
            content: "fail".to_string(),
            parent_tool_use_id: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "error");
        assert_eq!(v["content"], "fail");
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn chat_message_with_parts_roundtrip() {
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "hi".to_string(),
            thinking: Some("think".to_string()),
            activities: None,
            parts: Some(vec![
                MessagePart::Thinking {
                    content: "think".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Text {
                    content: "hi".to_string(),
                    parent_tool_use_id: None,
                },
            ]),
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parts.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn old_json_without_parts_deserializes_to_none() {
        let json = r#"{"id":"m1","role":"agent","content":"hello","timestamp":1000.0}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.parts, None);
    }

    #[test]
    fn message_part_permission_without_answers_serializes() {
        let part = MessagePart::Permission {
            request: serde_json::json!({"request_id": "r1"}),
            status: "pending".to_string(),
            answers: None,
            parent_tool_use_id: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(!json.contains("answers"));
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        if let MessagePart::Permission { answers, .. } = back {
            assert_eq!(answers, None);
        } else {
            panic!("Expected Permission variant");
        }
    }

    #[test]
    fn tool_result_without_tool_use_id_deserializes() {
        let json = r#"{"type":"tool_result","content":"ok","isError":false}"#;
        let part: MessagePart = serde_json::from_str(json).unwrap();
        if let MessagePart::ToolResult { tool_use_id, .. } = part {
            assert_eq!(tool_use_id, None);
        } else {
            panic!("Expected ToolResult variant");
        }

        let entry: ActivityEntry = serde_json::from_str(json).unwrap();
        if let ActivityEntry::ToolResult { tool_use_id, .. } = entry {
            assert_eq!(tool_use_id, None);
        } else {
            panic!("Expected ToolResult variant");
        }
    }

    #[test]
    fn task_status_serde_roundtrip() {
        let part = MessagePart::TaskStatus {
            task_tool_use_id: "toolu_task_001".to_string(),
            status: "completed".to_string(),
            description: Some("Search codebase".to_string()),
            summary: Some("Found 3 files".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "task_status");
        assert_eq!(v["taskToolUseId"], "toolu_task_001");
        assert_eq!(v["status"], "completed");
        assert_eq!(v["description"], "Search codebase");
        assert_eq!(v["summary"], "Found 3 files");
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn task_status_without_optional_fields_deserializes() {
        let json = r#"{"type":"task_status","taskToolUseId":"t1","status":"started"}"#;
        let part: MessagePart = serde_json::from_str(json).unwrap();
        if let MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } = part
        {
            assert_eq!(task_tool_use_id, "t1");
            assert_eq!(status, "started");
            assert_eq!(description, None);
            assert_eq!(summary, None);
        } else {
            panic!("Expected TaskStatus variant");
        }
    }

    #[test]
    fn parent_tool_use_id_serde() {
        let part = MessagePart::Text {
            content: "sub-agent text".to_string(),
            parent_tool_use_id: Some("toolu_parent".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("parentToolUseId"));
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        if let MessagePart::Text {
            parent_tool_use_id, ..
        } = back
        {
            assert_eq!(parent_tool_use_id, Some("toolu_parent".to_string()));
        } else {
            panic!("Expected Text variant");
        }
    }

    #[test]
    fn system_notification_serde_roundtrip() {
        let part = MessagePart::SystemNotification {
            notification_type: "compaction".to_string(),
            status: "completed".to_string(),
            label: "Conversation compacted".to_string(),
            detail: Some("trigger=auto, 50000 tokens".to_string()),
            hook_id: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "system_notification");
        assert_eq!(v["notificationType"], "compaction");
        assert_eq!(v["status"], "completed");
        assert_eq!(v["label"], "Conversation compacted");
        assert_eq!(v["detail"], "trigger=auto, 50000 tokens");
        assert!(v.get("hookId").is_none());
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn system_notification_with_hook_id_serde_roundtrip() {
        let part = MessagePart::SystemNotification {
            notification_type: "hook".to_string(),
            status: "in_progress".to_string(),
            label: "SessionEnd (StopSession)".to_string(),
            detail: None,
            hook_id: Some("hook-001".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["hookId"], "hook-001");
        assert!(v.get("detail").is_none());
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn old_json_without_system_notification_deserializes() {
        // Backward compat: old session JSON without system_notification parts
        let json = r#"[{"type":"text","content":"hello"},{"type":"task_status","taskToolUseId":"t1","status":"started"}]"#;
        let parts: Vec<MessagePart> = serde_json::from_str(json).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], MessagePart::Text { .. }));
        assert!(matches!(&parts[1], MessagePart::TaskStatus { .. }));
    }

    #[test]
    fn old_json_without_parent_tool_use_id_deserializes() {
        let json = r#"{"type":"text","content":"hello"}"#;
        let part: MessagePart = serde_json::from_str(json).unwrap();
        if let MessagePart::Text {
            parent_tool_use_id, ..
        } = part
        {
            assert_eq!(parent_tool_use_id, None);
        } else {
            panic!("Expected Text variant");
        }
    }

    #[test]
    fn message_part_image_serde_roundtrip() {
        let part = MessagePart::Image {
            data: "iVBORw0KGgoAAAA==".to_string(),
            media_type: "image/png".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "image");
        assert_eq!(v["data"], "iVBORw0KGgoAAAA==");
        assert_eq!(v["mediaType"], "image/png");
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn chat_message_with_image_parts_roundtrip() {
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Human,
            content: "Check this image".to_string(),
            thinking: None,
            activities: None,
            parts: Some(vec![
                MessagePart::Text {
                    content: "Check this image".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Image {
                    data: "base64data".to_string(),
                    media_type: "image/jpeg".to_string(),
                },
            ]),
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parts.as_ref().unwrap().len(), 2);
        assert!(matches!(
            &back.parts.as_ref().unwrap()[1],
            MessagePart::Image { .. }
        ));
    }

    #[test]
    fn parts_to_legacy_ignores_image() {
        let parts = vec![
            MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Image {
                data: "base64".to_string(),
                media_type: "image/png".to_string(),
            },
        ];
        let (content, thinking, activities) = parts_to_legacy(&parts);
        assert_eq!(content, "hello");
        assert_eq!(thinking, None);
        assert_eq!(activities, None);
    }

    // ---- WorkflowState serde ----

    fn make_session_test_node(
        name: &str,
        node_type: NodeType,
        instruction: &str,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            node_type,
            instruction: Some(instruction.to_string()),
            ..NodeDefinition::default()
        }
    }

    fn make_test_workflow_for_session() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "review-cycle".to_string(),
            description: "Test".to_string(),
            builtin: false,
            nodes: vec![
                make_session_test_node("plan", NodeType::Agent, "plan"),
                make_session_test_node("implement", NodeType::Agent, "implement"),
                make_session_test_node("review", NodeType::Agent, "review"),
                NodeDefinition {
                    name: "report".to_string(),
                    node_type: NodeType::Approval,
                    instruction: Some("report".to_string()),
                    ..NodeDefinition::default()
                },
            ],
        }
    }

    #[test]
    fn workflow_state_serde_roundtrip() {
        let state = WorkflowState {
            execution_id: "exec-1".to_string(),
            workflow_name: "review-cycle".to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 2,
            current_step_name: "review".to_string(),
            current_session_id: Some("sess-current".to_string()),
            total_steps: 4,
            step_history: vec![
                StepHistoryEntry {
                    step_name: "plan".to_string(),
                    completed_at: 1000.0,
                    result: None,
                    session_id: None,
                    token_usage: None,
                    structured_output: None,

                    run_index: 0,
                    child_outputs: None,
                    state: crate::workflow::state::default_step_entry_state(),
                },
                StepHistoryEntry {
                    step_name: "implement".to_string(),
                    completed_at: 1001.0,
                    result: Some("done".to_string()),
                    session_id: Some("sess-1".to_string()),
                    token_usage: Some(TokenUsage {
                        input_tokens: 100,
                        output_tokens: 50,
                    }),
                    structured_output: None,

                    run_index: 0,
                    child_outputs: None,
                    state: crate::workflow::state::default_step_entry_state(),
                },
            ],
            step_execution_counts: std::collections::HashMap::new(),
            step_outputs: std::collections::HashMap::new(),
            workflow_definition: make_test_workflow_for_session(),
            total_token_usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            },
            step_states: std::collections::HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: std::collections::HashMap::new(),
            approval_operations: None,
            started_at: 999.0,
            updated_at: 1001.0,
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: WorkflowState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.execution_id, "exec-1");
        assert_eq!(back.workflow_name, "review-cycle");
        assert_eq!(back.state, WorkflowExecutionState::Running);
        assert_eq!(back.current_step_index, 2);
        assert_eq!(back.current_step_name, "review");
        assert_eq!(back.total_steps, 4);
        assert_eq!(back.step_history.len(), 2);
        assert_eq!(back.step_history[0].step_name, "plan");
        assert_eq!(back.step_history[1].result, Some("done".to_string()));
    }

    #[test]
    fn workflow_execution_state_failed_tagged_enum_format() {
        let state = WorkflowExecutionState::Failed {
            reason: "exit code 1".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "failed");
        assert_eq!(v["reason"], "exit code 1");
        let back: WorkflowExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn workflow_execution_state_all_variants_serde() {
        let variants = vec![
            WorkflowExecutionState::Running,
            WorkflowExecutionState::WaitingApproval,
            WorkflowExecutionState::Completed,
            WorkflowExecutionState::Failed {
                reason: "err".to_string(),
            },
            WorkflowExecutionState::Aborted,
        ];
        for state in variants {
            let json = serde_json::to_string(&state).unwrap();
            let back: WorkflowExecutionState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }

    // 撤去済み: `chat_session_with_workflow_state_roundtrip` は ChatSession.workflow_state
    // フィールド廃止により役目を終えた。WorkflowState の serde roundtrip は
    // workflow/state.rs 側の単体テストで担保される。在庫 JSON の workflow_state は
    // serde の unknown_field 既定挙動で silently 読み捨てられる（破棄前提）。

    /// [02] schema 境界: 旧表現（`workflowDefinition.steps`）を含む WorkflowState JSON は
    /// 新 `Workflow` schema（`nodes` 必須 + `deny_unknown_fields`）として deserialize に失敗する。
    /// これにより旧表現の進行中状態は新バージョンに引き継がれない。
    #[test]
    fn legacy_workflow_state_with_steps_fails_to_deserialize() {
        let json = r#"{
            "executionId": "exec-1",
            "workflowName": "legacy",
            "state": { "type": "running" },
            "currentStepIndex": 0,
            "currentStepName": "x",
            "totalSteps": 1,
            "stepHistory": [],
            "stepExecutionCounts": {},
            "workflowDefinition": {
                "name": "legacy",
                "description": "",
                "builtin": false,
                "steps": [{"name":"x","mode":"auto","instruction":"x"}]
            },
            "totalTokenUsage": { "inputTokens": 0, "outputTokens": 0 },
            "stepStates": {},
            "startedAt": 1.0,
            "updatedAt": 1.0
        }"#;
        let result: Result<WorkflowState, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "旧 workflowDefinition.steps を含む WorkflowState は新 schema で deserialize 失敗する"
        );
    }

    /// parent ChatSession 廃止後の在庫 JSON 互換性: 旧 `ChatSession.workflowState` フィールドを
    /// 含む JSON は serde の unknown_fields 既定挙動で silently 読み捨てられ、deserialize は
    /// 成功する（破棄前提でロスレスではないが、起動の阻害にならない）。
    #[test]
    fn legacy_chat_session_with_old_workflow_state_is_silently_ignored() {
        let json = r#"{
            "id": "s1",
            "worktreePath": "/repo",
            "messages": [],
            "state": "active",
            "createdAt": 1.0,
            "updatedAt": 1.0,
            "permissionMode": "edit",
            "workflowStepSession": false,
            "workflowState": {
                "executionId": "exec-1",
                "workflowName": "legacy"
            }
        }"#;
        let result: Result<ChatSession, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "ChatSession.workflowState は撤去フィールド扱いで silently 読み捨てられる"
        );
    }

    #[test]
    fn create_session_internal_with_backend_id() {
        let store = SessionStore::default();
        let dir = tempfile::tempdir().unwrap();
        let session =
            create_session_internal(&store, dir.path(), "/repo", Some("claude".to_string()))
                .unwrap();
        assert_eq!(session.backend_id, Some("claude".to_string()));
        assert_eq!(session.state, SessionState::Active);
        assert_eq!(session.worktree_path, "/repo");
    }

    #[test]
    fn create_session_internal_without_backend_id() {
        let store = SessionStore::default();
        let dir = tempfile::tempdir().unwrap();
        let session = create_session_internal(&store, dir.path(), "/repo", None).unwrap();
        assert_eq!(session.backend_id, None);
    }

    // Spec issues-947: WS handler の AgentSessionStartRequest 経路は
    // `create_session_internal_with_permission` で session を生成し、検証済み抽象モードを
    // 初回保存で確定する。ask / edit / full それぞれが保存済みセッションの
    // permission_mode として選択値どおりに記録されることを確認する。
    #[test]
    fn create_session_with_permission_persists_selected_abstract_mode() {
        for mode in [
            crate::permission::PermissionMode::Ask,
            crate::permission::PermissionMode::Edit,
            crate::permission::PermissionMode::Full,
        ] {
            let store = SessionStore::default();
            let dir = tempfile::tempdir().unwrap();
            let created = create_session_internal_with_permission(
                &store,
                dir.path(),
                "/repo",
                Some("claude".to_string()),
                mode,
            )
            .unwrap();
            assert_eq!(created.permission_mode, mode.as_str());

            let loaded = store.get_session(dir.path(), &created.id).unwrap().unwrap();
            assert_eq!(loaded.permission_mode, mode.as_str());
        }
    }

    fn test_backend_registry() -> TestBackendResolver {
        TestBackendResolver::default()
            .with_backend(
                "claude",
                crate::domain::agent_session::CLAUDE_FIXED_MODELS[0],
            )
            .with_default("claude")
    }

    #[test]
    fn create_session_command_inner_persists_valid_permission_modes() {
        for mode in ["ask", "edit", "full"] {
            let store = SessionStore::default();
            let dir = tempfile::tempdir().unwrap();
            let registry = test_backend_registry();

            let created = create_session_command_inner(
                &store,
                &registry,
                dir.path(),
                "/repo",
                mode,
                Some("claude".to_string()),
            )
            .unwrap();

            assert_eq!(created.permission_mode, mode);
            let loaded = store.get_session(dir.path(), &created.id).unwrap().unwrap();
            assert_eq!(loaded.permission_mode, mode);
        }
    }

    #[test]
    fn create_session_command_inner_rejects_invalid_permission_without_creating_session() {
        for invalid in [
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
            "unknown",
            "",
        ] {
            let store = SessionStore::default();
            let dir = tempfile::tempdir().unwrap();
            let registry = test_backend_registry();

            let err = create_session_command_inner(
                &store,
                &registry,
                dir.path(),
                "/repo",
                invalid,
                Some("claude".to_string()),
            )
            .unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "invalid mode '{invalid}' must include allowed list, got: {err}"
            );
            assert!(store
                .list_worktree_sessions(dir.path(), "/repo")
                .unwrap()
                .is_empty());
        }
    }

    fn fixed_model_registry() -> TestBackendResolver {
        TestBackendResolver::default()
            .with_backend(
                "claude",
                crate::domain::agent_session::CLAUDE_FIXED_MODELS[0],
            )
            .with_backend("codex", crate::domain::agent_session::CODEX_FIXED_MODELS[0])
            .with_default("claude")
    }

    #[test]
    fn create_session_with_initial_model_persists_default_for_claude() {
        // spec: モデル未選択状態は廃止。新規セッションは常に backend の既定モデル
        // （固定リスト先頭）を selected_model に持ち、永続化される。
        let store = SessionStore::default();
        let dir = tempfile::tempdir().unwrap();
        let registry = fixed_model_registry();

        let default_model = crate::domain::agent_session::CLAUDE_FIXED_MODELS[0].to_string();

        let session = create_session_with_initial_model(
            &store,
            &registry,
            dir.path(),
            "/repo",
            "claude".to_string(),
            crate::permission::PermissionMode::Edit,
        )
        .unwrap();
        assert_eq!(session.selected_model, Some(default_model.clone()));

        // 永続化されている (on-disk から再ロードしても保持される)
        let reloaded = store.get_session(dir.path(), &session.id).unwrap().unwrap();
        assert_eq!(reloaded.selected_model, Some(default_model));
    }

    #[test]
    fn create_session_with_initial_model_persists_default_for_codex() {
        // spec: codex バックエンドも固定リスト先頭が既定モデルになる。
        let store = SessionStore::default();
        let dir = tempfile::tempdir().unwrap();
        let registry = fixed_model_registry();

        let default_model = crate::domain::agent_session::CODEX_FIXED_MODELS[0].to_string();

        let session = create_session_with_initial_model(
            &store,
            &registry,
            dir.path(),
            "/repo",
            "codex".to_string(),
            crate::permission::PermissionMode::Edit,
        )
        .unwrap();
        assert_eq!(session.selected_model, Some(default_model));
    }

    #[test]
    fn chat_session_without_backend_id_deserializes() {
        let json = r#"{"id":"s1","worktreePath":"/repo","messages":[],"state":"active","createdAt":1000.0,"updatedAt":1000.0,"permissionMode":"edit"}"#;
        let session: ChatSession = serde_json::from_str(json).unwrap();
        assert_eq!(session.backend_id, None);
    }

    #[test]
    fn chat_session_with_backend_id_roundtrip() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("\"backendId\":\"claude\""));
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.backend_id, Some("claude".to_string()));
    }

    #[test]
    fn session_summary_includes_backend_id() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: "Hello".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
        };
        let summary = session.to_summary();
        assert_eq!(summary.backend_id, Some("claude".to_string()));
    }

    // --- resolve_session_backend テスト ---

    mod resolve_session_backend_tests {
        use super::*;

        fn make_session(backend_id: Option<&str>) -> ChatSession {
            ChatSession {
                id: "s_test".to_string(),
                worktree_path: "/repo".to_string(),
                messages: vec![],
                state: SessionState::Closed,
                created_at: 1000.0,
                updated_at: 1000.0,
                agent_session_id: None,
                permission_mode: "edit".to_string(),
                selected_model: None,
                backend_id: backend_id.map(str::to_string),
                workflow_step_session: false,
            }
        }

        fn make_registry_with_claude() -> TestBackendResolver {
            TestBackendResolver::default()
                .with_backend(
                    "claude",
                    crate::domain::agent_session::CLAUDE_FIXED_MODELS[0],
                )
                .with_default("claude")
        }

        #[test]
        fn restore_with_valid_backend_id_succeeds() {
            let registry = make_registry_with_claude();
            let mut session = make_session(Some("claude"));
            let result = resolve_session_backend(&mut session, &registry);
            assert!(result.is_ok());
            assert_eq!(session.backend_id, Some("claude".to_string()));
        }

        #[test]
        fn restore_with_invalid_backend_id_returns_error() {
            let registry = make_registry_with_claude();
            let mut session = make_session(Some("codex"));
            let result = resolve_session_backend(&mut session, &registry);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("codex"));
        }

        #[test]
        fn restore_without_backend_id_assigns_default() {
            let registry = make_registry_with_claude();
            let mut session = make_session(None);
            assert_eq!(session.backend_id, None);

            let result = resolve_session_backend(&mut session, &registry);

            assert!(result.is_ok());
            assert_eq!(session.backend_id, Some("claude".to_string()));
        }
    }
}
