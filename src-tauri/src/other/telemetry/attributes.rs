pub(crate) const KEY_OPERATION: &str = "releash.operation";
pub(crate) const KEY_STATUS: &str = "releash.status";
pub(crate) const KEY_USAGE_EVENT: &str = "releash.usage_event";
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
}

impl HotPathMetric {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 3] = [Self::GitStatusScan, Self::DiffStats, Self::ReviewFileOpen];

    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::GitStatusScan => "git.status_scan",
            Self::DiffStats => "git.diff_stats",
            Self::ReviewFileOpen => "review.file_open",
        }
    }

    pub(crate) fn span_name(self) -> &'static str {
        match self {
            Self::GitStatusScan => "Git status scan",
            Self::DiffStats => "Git diff stats",
            Self::ReviewFileOpen => "Review file open",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupMetric {
    AppStartup,
    FirstWindowReady,
    FirstRepoSnapshotReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalLaunchMetric {
    CommandIngress,
    AvailabilityAndLock,
    DurableCreateCommit,
    LaunchFileMaterialize,
    CheckpointLookup,
    ChildEnvironment,
    PtyOpenAndSpawn,
    OutputReaderReady,
    FirstProviderByte,
    FirstXtermParsed,
    FirstPaint,
    HookIngress,
}

impl TerminalLaunchMetric {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 12] = [
        Self::CommandIngress,
        Self::AvailabilityAndLock,
        Self::DurableCreateCommit,
        Self::LaunchFileMaterialize,
        Self::CheckpointLookup,
        Self::ChildEnvironment,
        Self::PtyOpenAndSpawn,
        Self::OutputReaderReady,
        Self::FirstProviderByte,
        Self::FirstXtermParsed,
        Self::FirstPaint,
        Self::HookIngress,
    ];

    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::CommandIngress => "terminal.launch.command_ingress",
            Self::AvailabilityAndLock => "terminal.launch.availability_and_lock",
            Self::DurableCreateCommit => "terminal.launch.durable_create_commit",
            Self::LaunchFileMaterialize => "terminal.launch.launch_file_materialize",
            Self::CheckpointLookup => "terminal.launch.checkpoint_lookup",
            Self::ChildEnvironment => "terminal.launch.child_environment",
            Self::PtyOpenAndSpawn => "terminal.launch.pty_open_and_spawn",
            Self::OutputReaderReady => "terminal.launch.output_reader_ready",
            Self::FirstProviderByte => "terminal.launch.first_provider_byte",
            Self::FirstXtermParsed => "terminal.launch.first_xterm_parsed",
            Self::FirstPaint => "terminal.launch.first_paint",
            Self::HookIngress => "terminal.launch.hook_ingress",
        }
    }
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
            ["git.status_scan", "git.diff_stats", "review.file_open",]
        );
    }

    #[test]
    fn test_terminal_launch_operations_are_canonical() {
        let operations: Vec<_> = TerminalLaunchMetric::ALL
            .iter()
            .map(|metric| metric.operation())
            .collect();

        assert_eq!(
            operations,
            [
                "terminal.launch.command_ingress",
                "terminal.launch.availability_and_lock",
                "terminal.launch.durable_create_commit",
                "terminal.launch.launch_file_materialize",
                "terminal.launch.checkpoint_lookup",
                "terminal.launch.child_environment",
                "terminal.launch.pty_open_and_spawn",
                "terminal.launch.output_reader_ready",
                "terminal.launch.first_provider_byte",
                "terminal.launch.first_xterm_parsed",
                "terminal.launch.first_paint",
                "terminal.launch.hook_ingress",
            ]
        );
    }
}
