use opentelemetry::KeyValue;

use crate::domain::agent_session::{InvalidPermissionMode, PermissionMode};

pub(crate) const KEY_OPERATION: &str = "releash.operation";
pub(crate) const KEY_STATUS: &str = "releash.status";
#[allow(dead_code)] // issues-1301 B-6/G-1: retained for turn-latency outcome dimensions while telemetry wiring is completed.
pub(crate) const KEY_OUTCOME: &str = "releash.outcome";
pub(crate) const KEY_CHANNEL: &str = "releash.channel";
pub(crate) const KEY_USAGE_EVENT: &str = "releash.usage_event";
pub(crate) const KEY_AGENT_RESUME: &str = "releash.agent.resume";
pub(crate) const KEY_AGENT_HAS_SESSION: &str = "releash.agent.has_session";
pub(crate) const KEY_AGENT_PERMISSION_MODE: &str = "releash.agent.permission_mode";
pub(crate) const KEY_AGENT_MODEL: &str = "releash.agent.model";
pub(crate) const KEY_AGENT_CONTEXT: &str = "releash.agent.context";
pub(crate) const KEY_AGENT_WARM_PATH: &str = "releash.agent.warm_path";
pub(crate) const KEY_FAILURE_KIND: &str = "failure.kind";
pub(crate) const KEY_FAILURE_DISPOSITION: &str = "failure.disposition";
pub(crate) const KEY_RETRY_COUNT: &str = "failure.retry_count";
pub(crate) const KEY_TIMEOUT_KIND: &str = "failure.timeout_kind";

#[cfg(test)]
pub(crate) fn allowed_resource_attribute_keys() -> [&'static str; 4] {
    [
        "service.version",
        "os.type",
        "releash.build_type",
        "service.name",
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpStatus {
    Success,
    Failure,
}

impl OpStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HotPathMetric {
    GitStatusScan,
    DiffStats,
    ReviewFileOpen,
    SessionList,
    SessionGetMeta,
    SessionGetPage,
    SessionLoadFull,
    SessionAppend,
    SessionPersistParts,
    SessionSaveFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // issues-1301 B-6: retained latency dimensions while full turn-latency telemetry is reconnected.
pub(crate) enum AgentTurnMetric {
    UiToStart,
    BackendSpawn,
    QueryInit,
    FirstBackendEvent,
    FirstAssistantEvent,
    PermissionWait,
    Complete,
}

impl AgentTurnMetric {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 7] = [
        Self::UiToStart,
        Self::BackendSpawn,
        Self::QueryInit,
        Self::FirstBackendEvent,
        Self::FirstAssistantEvent,
        Self::PermissionWait,
        Self::Complete,
    ];

    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::UiToStart => "agent.turn.ui_to_start",
            Self::BackendSpawn => "agent.turn.backend_spawn",
            Self::QueryInit => "agent.turn.query_init",
            Self::FirstBackendEvent => "agent.turn.first_backend_event",
            Self::FirstAssistantEvent => "agent.turn.first_assistant_event",
            Self::PermissionWait => "agent.turn.permission_wait",
            Self::Complete => "agent.turn.complete",
        }
    }
}

impl HotPathMetric {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 10] = [
        Self::GitStatusScan,
        Self::DiffStats,
        Self::ReviewFileOpen,
        Self::SessionList,
        Self::SessionGetMeta,
        Self::SessionGetPage,
        Self::SessionLoadFull,
        Self::SessionAppend,
        Self::SessionPersistParts,
        Self::SessionSaveFull,
    ];

    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::GitStatusScan => "git.status_scan",
            Self::DiffStats => "git.diff_stats",
            Self::ReviewFileOpen => "review.file_open",
            Self::SessionList => "session.list",
            Self::SessionGetMeta => "session.get_meta",
            Self::SessionGetPage => "session.get_page",
            Self::SessionLoadFull => "session.load_full",
            Self::SessionAppend => "session.append",
            Self::SessionPersistParts => "session.persist_parts",
            Self::SessionSaveFull => "session.save_full",
        }
    }

    pub(crate) fn span_name(self) -> &'static str {
        match self {
            Self::GitStatusScan => "Git status scan",
            Self::DiffStats => "Git diff stats",
            Self::ReviewFileOpen => "Review file open",
            Self::SessionList => "Session list",
            Self::SessionGetMeta => "Session get meta",
            Self::SessionGetPage => "Session get page",
            Self::SessionLoadFull => "Session load full",
            Self::SessionAppend => "Session append",
            Self::SessionPersistParts => "Session persist parts",
            Self::SessionSaveFull => "Session save full",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PayloadChannel {
    TauriEvent,
}

impl PayloadChannel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TauriEvent => "tauri_event",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PermissionModeDim {
    Ask,
    Edit,
    Full,
    Other,
}

impl PermissionModeDim {
    pub(crate) fn normalize(permission_mode: &str) -> Self {
        let parsed: Result<PermissionMode, InvalidPermissionMode> =
            PermissionMode::parse(permission_mode);
        match parsed {
            Ok(PermissionMode::Ask) => Self::Ask,
            Ok(PermissionMode::Edit) => Self::Edit,
            Ok(PermissionMode::Full) => Self::Full,
            Err(_) => Self::Other,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Edit => "edit",
            Self::Full => "full",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelFamily {
    Opus,
    Sonnet,
    Haiku,
    Other,
}

impl ModelFamily {
    pub(crate) fn normalize(model_id: Option<&str>) -> Self {
        let Some(model_id) = model_id else {
            return Self::Other;
        };
        let model_id = model_id.to_ascii_lowercase();
        if model_id.contains("opus") {
            Self::Opus
        } else if model_id.contains("sonnet") {
            Self::Sonnet
        } else if model_id.contains("haiku") {
            Self::Haiku
        } else {
            Self::Other
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnContext {
    Chat,
    WorkflowStep,
}

impl TurnContext {
    pub(crate) fn from_workflow_step(is_workflow_step: bool) -> Self {
        if is_workflow_step {
            Self::WorkflowStep
        } else {
            Self::Chat
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::WorkflowStep => "workflow_step",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WarmPath {
    QueryDirect,
}

impl WarmPath {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::QueryDirect => "query_direct",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TurnDimensions {
    pub(crate) resume: bool,
    pub(crate) has_session: bool,
    pub(crate) permission_mode: PermissionModeDim,
    pub(crate) model: ModelFamily,
    pub(crate) context: TurnContext,
    pub(crate) channel: PayloadChannel,
    pub(crate) warm_path: WarmPath,
}

impl TurnDimensions {
    #[cfg(test)]
    pub(crate) const ALLOWED_ATTRIBUTE_KEYS: [&'static str; 8] = [
        KEY_OPERATION,
        KEY_AGENT_RESUME,
        KEY_AGENT_HAS_SESSION,
        KEY_AGENT_PERMISSION_MODE,
        KEY_AGENT_MODEL,
        KEY_AGENT_CONTEXT,
        KEY_CHANNEL,
        KEY_AGENT_WARM_PATH,
    ];

    fn bool_str(value: bool) -> &'static str {
        if value {
            "true"
        } else {
            "false"
        }
    }

    #[cfg(test)]
    pub(crate) fn to_attrs(self) -> [KeyValue; 7] {
        [
            KeyValue::new(KEY_AGENT_RESUME, Self::bool_str(self.resume)),
            KeyValue::new(KEY_AGENT_HAS_SESSION, Self::bool_str(self.has_session)),
            KeyValue::new(KEY_AGENT_PERMISSION_MODE, self.permission_mode.as_str()),
            KeyValue::new(KEY_AGENT_MODEL, self.model.as_str()),
            KeyValue::new(KEY_AGENT_CONTEXT, self.context.as_str()),
            KeyValue::new(KEY_CHANNEL, self.channel.as_str()),
            KeyValue::new(KEY_AGENT_WARM_PATH, self.warm_path.as_str()),
        ]
    }

    pub(crate) fn to_metric_attrs(self, operation: &'static str) -> [KeyValue; 8] {
        [
            KeyValue::new(KEY_OPERATION, operation),
            KeyValue::new(KEY_AGENT_RESUME, Self::bool_str(self.resume)),
            KeyValue::new(KEY_AGENT_HAS_SESSION, Self::bool_str(self.has_session)),
            KeyValue::new(KEY_AGENT_PERMISSION_MODE, self.permission_mode.as_str()),
            KeyValue::new(KEY_AGENT_MODEL, self.model.as_str()),
            KeyValue::new(KEY_AGENT_CONTEXT, self.context.as_str()),
            KeyValue::new(KEY_CHANNEL, self.channel.as_str()),
            KeyValue::new(KEY_AGENT_WARM_PATH, self.warm_path.as_str()),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupMetric {
    AppStartup,
    FirstWindowReady,
    FirstRepoSnapshotReady,
}

impl StartupMetric {
    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::AppStartup => "startup.app",
            Self::FirstWindowReady => "startup.first_window_ready",
            Self::FirstRepoSnapshotReady => "startup.first_repo_snapshot_ready",
        }
    }
}

pub(crate) fn usage_event_allowed(name: &str) -> bool {
    matches!(
        name,
        "settings_saved" | "worktree_created" | "worktree_removed"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_keys_are_allowlisted() {
        assert_eq!(
            allowed_resource_attribute_keys(),
            [
                "service.version",
                "os.type",
                "releash.build_type",
                "service.name"
            ]
        );
    }

    #[test]
    fn usage_events_are_allowlisted() {
        assert!(usage_event_allowed("settings_saved"));
        assert!(!usage_event_allowed("repo_path_added"));
    }

    #[test]
    fn hot_path_operations_are_canonical() {
        let operations: Vec<_> = HotPathMetric::ALL
            .iter()
            .map(|metric| metric.operation())
            .collect();

        assert_eq!(
            operations,
            [
                "git.status_scan",
                "git.diff_stats",
                "review.file_open",
                "session.list",
                "session.get_meta",
                "session.get_page",
                "session.load_full",
                "session.append",
                "session.persist_parts",
                "session.save_full",
            ]
        );
    }

    #[test]
    fn agent_turn_operations_are_canonical() {
        let operations: Vec<_> = AgentTurnMetric::ALL
            .iter()
            .map(|metric| metric.operation())
            .collect();

        assert_eq!(
            operations,
            [
                "agent.turn.ui_to_start",
                "agent.turn.backend_spawn",
                "agent.turn.query_init",
                "agent.turn.first_backend_event",
                "agent.turn.first_assistant_event",
                "agent.turn.permission_wait",
                "agent.turn.complete",
            ]
        );
    }

    #[test]
    fn agent_turn_model_family_is_bounded() {
        assert_eq!(
            ModelFamily::normalize(Some("claude-opus-4-8")),
            ModelFamily::Opus
        );
        assert_eq!(
            ModelFamily::normalize(Some("claude-sonnet-4-6")),
            ModelFamily::Sonnet
        );
        assert_eq!(
            ModelFamily::normalize(Some("claude-haiku-4-5")),
            ModelFamily::Haiku
        );
        assert_eq!(
            ModelFamily::normalize(Some("unknown-model")),
            ModelFamily::Other
        );
        assert_eq!(ModelFamily::normalize(None), ModelFamily::Other);
    }

    #[test]
    fn agent_turn_permission_mode_is_bounded() {
        assert_eq!(PermissionModeDim::normalize("ask"), PermissionModeDim::Ask);
        assert_eq!(
            PermissionModeDim::normalize("edit"),
            PermissionModeDim::Edit
        );
        assert_eq!(
            PermissionModeDim::normalize("full"),
            PermissionModeDim::Full
        );
        assert_eq!(
            PermissionModeDim::normalize("acceptEdits"),
            PermissionModeDim::Other
        );
    }

    #[test]
    fn agent_turn_dimensions_emit_only_allowlisted_attrs() {
        let dims = TurnDimensions {
            resume: true,
            has_session: true,
            permission_mode: PermissionModeDim::Edit,
            model: ModelFamily::Sonnet,
            context: TurnContext::WorkflowStep,
            channel: PayloadChannel::TauriEvent,
            warm_path: WarmPath::QueryDirect,
        };

        let attrs = dims.to_metric_attrs(AgentTurnMetric::Complete.operation());
        let dim_attrs = dims.to_attrs();
        let keys: Vec<_> = attrs.iter().map(|attr| attr.key.as_str()).collect();
        let values: Vec<_> = attrs.iter().map(|attr| attr.value.to_string()).collect();

        assert_eq!(dim_attrs.len(), 7);
        assert_eq!(keys, TurnDimensions::ALLOWED_ATTRIBUTE_KEYS);
        assert_eq!(
            values,
            [
                "agent.turn.complete",
                "true",
                "true",
                "edit",
                "sonnet",
                "workflow_step",
                "tauri_event",
                "query_direct",
            ]
        );
    }
}
