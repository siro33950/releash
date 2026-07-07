use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use futures_util::stream::Stream;

use crate::domain::agent_session::entities::{
    AttachmentPayload, MessagePart, PermissionRequest, PermissionResponse, TokenUsage, TurnResult,
};
use crate::domain::agent_session::value_objects::{
    BackendCapabilities, EditorContext, ModelDescriptor, ModelId, PermissionMode, SkillEntry,
    SlashCommand,
};

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub session_id: String,
    pub cwd: String,
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub model: ModelId,
    pub system_prompt: Option<String>,
    pub resume: Option<String>,
    pub base_branch: Option<String>,
    pub startup_timeout: Option<Duration>,
    pub startup_max_retries: Option<u32>,
    pub stale_timeout: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct TurnInput {
    pub prompt: String,
    pub images: Vec<AttachmentPayload>,
    pub system_prompt: Option<String>,
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub editor_context: Option<EditorContext>,
}

#[allow(dead_code)]
// issues-1301 F-2: fork request fields are consumed by backend lifecycle implementations; production call coverage is restored with Codex thread lifecycle.
#[derive(Debug, Clone)]
pub struct ForkSessionRequest {
    pub backend_session_id: String,
    pub cwd: String,
    pub model: Option<String>,
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRuntimeEvent {
    SessionEstablished {
        backend_session_id: String,
        resume: ResumeOutcome,
    },
    #[allow(dead_code)]
    // issues-1301 D-2/D-7: emitted by resume-mismatch/recovery paths once backend session clearing is fully wired.
    BackendSessionCleared,
    PartsMerged(Vec<MessagePart>),
    PermissionRequested(PermissionRequest),
    PermissionModeChanged(PermissionMode),
    SlashCommandsUpdated(Vec<SlashCommand>),
    TokenUsageUpdated(TokenUsage),
    /// backend が turn 継続中であることを示す生存通知。
    /// message part を伴わず、stale 監視の progress 更新のみに使う。
    KeepAlive,
    TurnCompleted(TurnResult),
    Fatal {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    NotRequested,
    Resumed,
    Mismatch { actual: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentBackendError {
    StartupTimeout { retry_count: u32, max_retries: u32 },
    Unavailable(String),
    Invalid(String),
    Other(String),
}

impl std::fmt::Display for AgentBackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartupTimeout {
                retry_count,
                max_retries,
            } => write!(
                f,
                "Timed out waiting for agent session startup (retry_count={retry_count}, max_retries={max_retries})"
            ),
            Self::Unavailable(message) | Self::Invalid(message) | Self::Other(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for AgentBackendError {}

#[async_trait::async_trait]
pub trait AgentBackend: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn available_models(&self) -> Vec<ModelDescriptor>;
    fn capabilities(&self) -> BackendCapabilities;

    async fn open_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError>;

    async fn archive_session(
        &self,
        backend_session_id: &str,
        cwd: &str,
    ) -> Result<(), AgentBackendError>;

    async fn unarchive_session(
        &self,
        backend_session_id: &str,
        cwd: &str,
    ) -> Result<(), AgentBackendError>;

    async fn fork_session(
        &self,
        req: ForkSessionRequest,
    ) -> Result<Option<String>, AgentBackendError>;

    async fn skill_catalog(
        &self,
        cwd: &Path,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, AgentBackendError>;

    async fn fuzzy_file_search(
        &self,
        root: &Path,
        query: &str,
        limit: usize,
    ) -> Result<Option<Vec<String>>, AgentBackendError>;
}

#[async_trait::async_trait]
pub trait AgentSessionRuntime: Send + Sync {
    fn take_events(&mut self) -> Pin<Box<dyn Stream<Item = AgentRuntimeEvent> + Send>>;
    async fn start_turn(&self, input: TurnInput) -> Result<(), AgentBackendError>;

    #[allow(dead_code)] // issues-1301 D16/F-2: optional capability method; backends that do not support steering inherit queue behavior.
    async fn steer(&self, _input: TurnInput) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Unavailable(
            "active-turn steering is not available for this backend".to_string(),
        ))
    }

    #[allow(dead_code)]
    async fn reconnect(&self) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Unavailable(
            "session reconnect is not available for this backend".to_string(),
        ))
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError>;
    async fn respond_permission(
        &self,
        response: PermissionResponse,
    ) -> Result<(), AgentBackendError>;
    async fn set_permission_mode(
        &self,
        mode: PermissionMode,
        plan_mode: bool,
    ) -> Result<(), AgentBackendError>;
    async fn set_model(&self, model: &ModelId) -> Result<(), AgentBackendError>;

    async fn set_session_title(&self, _title: &str) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn close(&self);
}
