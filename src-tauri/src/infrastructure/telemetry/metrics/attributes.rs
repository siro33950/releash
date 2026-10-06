pub(crate) const KEY_OPERATION: &str = "releash.operation";
pub(crate) const KEY_STATUS: &str = "releash.status";
pub(crate) const KEY_USAGE_EVENT: &str = "releash.usage_event";
pub(crate) const KEY_FAILURE_KIND: &str = "failure.kind";
pub(crate) const KEY_FAILURE_DISPOSITION: &str = "failure.disposition";
pub(crate) const KEY_RETRY_COUNT: &str = "failure.retry_count";
pub(crate) const KEY_TIMEOUT_KIND: &str = "failure.timeout_kind";

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
}

impl HotPathMetric {
    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::GitStatusScan => "git.status_scan",
            Self::DiffStats => "git.diff_stats",
        }
    }

    pub(crate) fn span_name(self) -> &'static str {
        match self {
            Self::GitStatusScan => "Git status scan",
            Self::DiffStats => "Git diff stats",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupMetric {
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
    HookIngress,
}

impl TerminalLaunchMetric {
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
#[path = "attributes_test.rs"]
mod attributes_tests;
