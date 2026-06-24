pub(crate) const KEY_OPERATION: &str = "releash.operation";
pub(crate) const KEY_STATUS: &str = "releash.status";
pub(crate) const KEY_OUTCOME: &str = "releash.outcome";
pub(crate) const KEY_CHANNEL: &str = "releash.channel";
pub(crate) const KEY_USAGE_EVENT: &str = "releash.usage_event";

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
    WebSocket,
}

impl PayloadChannel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TauriEvent => "tauri_event",
            Self::WebSocket => "websocket",
        }
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
}
