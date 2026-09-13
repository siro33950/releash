use super::client as wire;

fn cv<T, U: TryFrom<T>>(value: T) -> Result<U, String>
where
    U::Error: std::fmt::Display,
{
    U::try_from(value).map_err(|error| error.to_string())
}
fn req<T>(value: Option<T>, name: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("Missing {name}"))
}

impl TryFrom<crate::adaptor::protocol::workflow::ApprovalTargetView> for wire::ApprovalTargetView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::ApprovalTargetView,
    ) -> Result<Self, String> {
        Ok(Self {
            node_execution_id: Some(cv(value.node_execution_id)?),
            node_name: Some(cv(value.node_name)?),
            session_id: value.session_id.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::ArtifactView> for wire::ArtifactView {
    type Error = String;
    fn try_from(value: crate::adaptor::protocol::workflow::ArtifactView) -> Result<Self, String> {
        Ok(Self {
            node_name: Some(cv(value.node_name)?),
            contract: value.contract.map(cv).transpose()?,
            value: Some(cv(value.value)?),
            produced_at: Some(cv(value.produced_at)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_dto::BranchCardDto> for wire::BranchCardDto {
    type Error = String;
    fn try_from(value: crate::usecase::repository_dto::BranchCardDto) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            is_main_worktree: Some(cv(value.is_main_worktree)?),
            worktree_path: value.worktree_path.map(cv).transpose()?,
            dirty_count: Some(cv(value.dirty_count)?),
            is_merged: Some(cv(value.is_merged)?),
            ahead: Some(cv(value.ahead)?),
            behind: Some(cv(value.behind)?),
            has_upstream: Some(cv(value.has_upstream)?),
            base_ahead: Some(cv(value.base_ahead)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_dto::BranchDto> for wire::BranchDto {
    type Error = String;
    fn try_from(value: crate::usecase::repository_dto::BranchDto) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            is_remote: Some(cv(value.is_remote)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_dto::CommitDto> for wire::CommitDto {
    type Error = String;
    fn try_from(value: crate::usecase::repository_dto::CommitDto) -> Result<Self, String> {
        Ok(Self {
            hash: Some(cv(value.hash)?),
            short_hash: Some(cv(value.short_hash)?),
            message: Some(cv(value.message)?),
            author_name: Some(cv(value.author_name)?),
            author_email: Some(cv(value.author_email)?),
            timestamp: Some(cv(value.timestamp)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::ExecutionInterruptionReasonView>
    for wire::ExecutionInterruptionReasonView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::ExecutionInterruptionReasonView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::workflow::ExecutionInterruptionReasonView::Crash => {
                    wire::execution_interruption_reason_view::Value::Crash as i32
                }
                crate::adaptor::protocol::workflow::ExecutionInterruptionReasonView::Stale => {
                    wire::execution_interruption_reason_view::Value::Stale as i32
                }
                crate::adaptor::protocol::workflow::ExecutionInterruptionReasonView::Stop => {
                    wire::execution_interruption_reason_view::Value::Stop as i32
                }
                crate::adaptor::protocol::workflow::ExecutionInterruptionReasonView::Orphan => {
                    wire::execution_interruption_reason_view::Value::Orphan as i32
                }
            }),
        })
    }
}

impl TryFrom<String> for wire::ExecutionInterruptionReasonView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "crash" => wire::execution_interruption_reason_view::Value::Crash as i32,
                "stale" => wire::execution_interruption_reason_view::Value::Stale as i32,
                "stop" => wire::execution_interruption_reason_view::Value::Stop as i32,
                "orphan" => wire::execution_interruption_reason_view::Value::Orphan as i32,
                _ => return Err(format!("Invalid ExecutionInterruptionReasonView: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::ExecutionInterruptionReasonView {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::ExecutionOriginView>
    for wire::ExecutionOriginView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::ExecutionOriginView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::workflow::ExecutionOriginView::DesktopUi => {
                    wire::execution_origin_view::Value::DesktopUi as i32
                }
                crate::adaptor::protocol::workflow::ExecutionOriginView::Cli => {
                    wire::execution_origin_view::Value::Cli as i32
                }
                crate::adaptor::protocol::workflow::ExecutionOriginView::Agent => {
                    wire::execution_origin_view::Value::Agent as i32
                }
                crate::adaptor::protocol::workflow::ExecutionOriginView::Api => {
                    wire::execution_origin_view::Value::Api as i32
                }
            }),
        })
    }
}

impl TryFrom<String> for wire::ExecutionOriginView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "desktop_ui" => wire::execution_origin_view::Value::DesktopUi as i32,
                "cli" => wire::execution_origin_view::Value::Cli as i32,
                "agent" => wire::execution_origin_view::Value::Agent as i32,
                "api" => wire::execution_origin_view::Value::Api as i32,
                _ => return Err(format!("Invalid ExecutionOriginView: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::ExecutionOriginView {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::ExecutionParentRefView>
    for wire::ExecutionParentRefView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::ExecutionParentRefView,
    ) -> Result<Self, String> {
        Ok(Self {
            parent_id: Some(cv(value.parent_id)?),
            item_index: value.item_index.map(cv).transpose()?,
            child_index: value.child_index.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::ExecutionStatusView>
    for wire::ExecutionStatusView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::ExecutionStatusView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::workflow::ExecutionStatusView::Running => {
                    wire::execution_status_view::Value::Running as i32
                }
                crate::adaptor::protocol::workflow::ExecutionStatusView::WaitingApproval => {
                    wire::execution_status_view::Value::WaitingApproval as i32
                }
                crate::adaptor::protocol::workflow::ExecutionStatusView::Completed => {
                    wire::execution_status_view::Value::Completed as i32
                }
                crate::adaptor::protocol::workflow::ExecutionStatusView::Aborted => {
                    wire::execution_status_view::Value::Aborted as i32
                }
                crate::adaptor::protocol::workflow::ExecutionStatusView::Interrupted => {
                    wire::execution_status_view::Value::Interrupted as i32
                }
            }),
        })
    }
}

impl TryFrom<String> for wire::ExecutionStatusView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "running" => wire::execution_status_view::Value::Running as i32,
                "waiting_approval" => wire::execution_status_view::Value::WaitingApproval as i32,
                "completed" => wire::execution_status_view::Value::Completed as i32,
                "aborted" => wire::execution_status_view::Value::Aborted as i32,
                "interrupted" => wire::execution_status_view::Value::Interrupted as i32,
                _ => return Err(format!("Invalid ExecutionStatusView: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::ExecutionStatusView {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::FanoutView> for wire::FanoutView {
    type Error = String;
    fn try_from(value: crate::adaptor::protocol::workflow::FanoutView) -> Result<Self, String> {
        Ok(Self {
            parent: Some(cv(value.parent)?),
            children: Some(cv(value.children)?),
            artifact: value.artifact.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::gateway::repository::watch::FileChangeEvent>
    for wire::FileChangeEvent
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::gateway::repository::watch::FileChangeEvent,
    ) -> Result<Self, String> {
        Ok(Self {
            watcher_id: Some(cv(value.watcher_id)?),
            path: Some(cv(value.path)?),
            kind: Some(cv(value.kind)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_dto::FileDiffStatDto> for wire::FileDiffStatDto {
    type Error = String;
    fn try_from(value: crate::usecase::repository_dto::FileDiffStatDto) -> Result<Self, String> {
        Ok(Self {
            path: Some(cv(value.path)?),
            index_additions: Some(cv(value.index_additions)?),
            index_deletions: Some(cv(value.index_deletions)?),
            wt_additions: Some(cv(value.wt_additions)?),
            wt_deletions: Some(cv(value.wt_deletions)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_dto::FileStatusDto> for wire::FileStatusDto {
    type Error = String;
    fn try_from(value: crate::usecase::repository_dto::FileStatusDto) -> Result<Self, String> {
        Ok(Self {
            path: Some(cv(value.path)?),
            index_status: Some(cv(value.index_status)?),
            worktree_status: Some(cv(value.worktree_status)?),
        })
    }
}

impl TryFrom<String> for wire::GitIndexStatus {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "new" => wire::git_index_status::Value::New as i32,
                "modified" => wire::git_index_status::Value::Modified as i32,
                "deleted" => wire::git_index_status::Value::Deleted as i32,
                "none" => wire::git_index_status::Value::None as i32,
                "renamed" => wire::git_index_status::Value::Renamed as i32,
                _ => return Err(format!("Invalid GitIndexStatus: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::GitIndexStatus {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::adaptor::gateway::repository::watch::GitStatusChangedEvent>
    for wire::GitStatusChangedEvent
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::gateway::repository::watch::GitStatusChangedEvent,
    ) -> Result<Self, String> {
        Ok(Self {
            repo_path: Some(cv(value.repo_path)?),
        })
    }
}

impl TryFrom<String> for wire::GitWorktreeStatus {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "new" => wire::git_worktree_status::Value::New as i32,
                "modified" => wire::git_worktree_status::Value::Modified as i32,
                "deleted" => wire::git_worktree_status::Value::Deleted as i32,
                "ignored" => wire::git_worktree_status::Value::Ignored as i32,
                "none" => wire::git_worktree_status::Value::None as i32,
                _ => return Err(format!("Invalid GitWorktreeStatus: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::GitWorktreeStatus {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl<T> TryFrom<Vec<T>> for wire::ListArtifactView
where
    wire::ArtifactView: TryFrom<T>,
    <wire::ArtifactView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListBranchCardDto
where
    wire::BranchCardDto: TryFrom<T>,
    <wire::BranchCardDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListBranchDto
where
    wire::BranchDto: TryFrom<T>,
    <wire::BranchDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListCommitDto
where
    wire::CommitDto: TryFrom<T>,
    <wire::CommitDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListFanoutView
where
    wire::FanoutView: TryFrom<T>,
    <wire::FanoutView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListFileDiffStatDto
where
    wire::FileDiffStatDto: TryFrom<T>,
    <wire::FileDiffStatDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListFileStatusDto
where
    wire::FileStatusDto: TryFrom<T>,
    <wire::FileStatusDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListNodeExecutionView
where
    wire::NodeExecutionView: TryFrom<T>,
    <wire::NodeExecutionView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListWorktreeEntryDto
where
    wire::WorktreeEntryDto: TryFrom<T>,
    <wire::WorktreeEntryDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::Liststring
where
    String: TryFrom<T>,
    <String as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T: TryFrom<String>> TryFrom<wire::Liststring> for Vec<T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::Liststring) -> Result<Self, String> {
        value.items.into_iter().map(cv).collect()
    }
}
impl<T> TryFrom<std::collections::BTreeMap<String, T>> for wire::MapListstring
where
    wire::Liststring: TryFrom<T>,
    <wire::Liststring as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: std::collections::BTreeMap<String, T>) -> Result<Self, String> {
        Ok(Self {
            entries: value
                .into_iter()
                .map(|(key, value)| Ok((key, cv(value)?)))
                .collect::<Result<_, String>>()?,
        })
    }
}
impl<T: TryFrom<wire::Liststring>> TryFrom<wire::MapListstring>
    for std::collections::BTreeMap<String, T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::MapListstring) -> Result<Self, String> {
        value
            .entries
            .into_iter()
            .map(|(key, value)| Ok((key, cv(value)?)))
            .collect()
    }
}
impl<T> TryFrom<std::collections::HashMap<String, T>> for wire::MapListstring
where
    wire::Liststring: TryFrom<T>,
    <wire::Liststring as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: std::collections::HashMap<String, T>) -> Result<Self, String> {
        Ok(Self {
            entries: value
                .into_iter()
                .map(|(key, value)| Ok((key, cv(value)?)))
                .collect::<Result<_, String>>()?,
        })
    }
}
impl<T: TryFrom<wire::Liststring>> TryFrom<wire::MapListstring>
    for std::collections::HashMap<String, T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::MapListstring) -> Result<Self, String> {
        value
            .entries
            .into_iter()
            .map(|(key, value)| Ok((key, cv(value)?)))
            .collect()
    }
}
impl TryFrom<crate::adaptor::protocol::workflow::NodeCompletionSignalView>
    for wire::NodeCompletionSignalView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::NodeCompletionSignalView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::workflow::NodeCompletionSignalView::Submit => {
                    wire::node_completion_signal_view::Value::Submit as i32
                }
                crate::adaptor::protocol::workflow::NodeCompletionSignalView::Stop => {
                    wire::node_completion_signal_view::Value::Stop as i32
                }
            }),
        })
    }
}

impl TryFrom<String> for wire::NodeCompletionSignalView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "submit" => wire::node_completion_signal_view::Value::Submit as i32,
                "stop" => wire::node_completion_signal_view::Value::Stop as i32,
                _ => return Err(format!("Invalid NodeCompletionSignalView: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::NodeCompletionSignalView {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::NodeExecutionFailureKindView>
    for wire::NodeExecutionFailureKindView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::NodeExecutionFailureKindView,
    ) -> Result<Self, String> {
        Ok(Self { value: Some(match value { crate::adaptor::protocol::workflow::NodeExecutionFailureKindView::StartupTimeout => wire::node_execution_failure_kind_view::Value::StartupTimeout as i32, crate::adaptor::protocol::workflow::NodeExecutionFailureKindView::StaleRuntimeTimeout => wire::node_execution_failure_kind_view::Value::StaleRuntimeTimeout as i32, crate::adaptor::protocol::workflow::NodeExecutionFailureKindView::ModelRefusal => wire::node_execution_failure_kind_view::Value::ModelRefusal as i32, crate::adaptor::protocol::workflow::NodeExecutionFailureKindView::StructuredOutputMismatch => wire::node_execution_failure_kind_view::Value::StructuredOutputMismatch as i32, crate::adaptor::protocol::workflow::NodeExecutionFailureKindView::ValidationFailure => wire::node_execution_failure_kind_view::Value::ValidationFailure as i32, crate::adaptor::protocol::workflow::NodeExecutionFailureKindView::UserAbort => wire::node_execution_failure_kind_view::Value::UserAbort as i32, crate::adaptor::protocol::workflow::NodeExecutionFailureKindView::InfrastructureCrash => wire::node_execution_failure_kind_view::Value::InfrastructureCrash as i32 }) })
    }
}

impl TryFrom<String> for wire::NodeExecutionFailureKindView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "startup_timeout" => {
                    wire::node_execution_failure_kind_view::Value::StartupTimeout as i32
                }
                "stale_runtime_timeout" => {
                    wire::node_execution_failure_kind_view::Value::StaleRuntimeTimeout as i32
                }
                "model_refusal" => {
                    wire::node_execution_failure_kind_view::Value::ModelRefusal as i32
                }
                "structured_output_mismatch" => {
                    wire::node_execution_failure_kind_view::Value::StructuredOutputMismatch as i32
                }
                "validation_failure" => {
                    wire::node_execution_failure_kind_view::Value::ValidationFailure as i32
                }
                "user_abort" => wire::node_execution_failure_kind_view::Value::UserAbort as i32,
                "infrastructure_crash" => {
                    wire::node_execution_failure_kind_view::Value::InfrastructureCrash as i32
                }
                _ => return Err(format!("Invalid NodeExecutionFailureKindView: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::NodeExecutionFailureKindView {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::NodeExecutionFailureView>
    for wire::NodeExecutionFailureView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::NodeExecutionFailureView,
    ) -> Result<Self, String> {
        Ok(Self {
            reason: Some(cv(value.reason)?),
            kind: Some(cv(value.kind)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::NodeExecutionStatusView>
    for wire::NodeExecutionStatusView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::NodeExecutionStatusView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::Unresolved => {
                    wire::node_execution_status_view::Value::Unresolved as i32
                }
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::Running => {
                    wire::node_execution_status_view::Value::Running as i32
                }
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::Paused => {
                    wire::node_execution_status_view::Value::Paused as i32
                }
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::WaitingApproval => {
                    wire::node_execution_status_view::Value::WaitingApproval as i32
                }
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::Succeeded => {
                    wire::node_execution_status_view::Value::Succeeded as i32
                }
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::Failed => {
                    wire::node_execution_status_view::Value::Failed as i32
                }
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::Aborted => {
                    wire::node_execution_status_view::Value::Aborted as i32
                }
            }),
        })
    }
}

impl TryFrom<String> for wire::NodeExecutionStatusView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "unresolved" => wire::node_execution_status_view::Value::Unresolved as i32,
                "running" => wire::node_execution_status_view::Value::Running as i32,
                "paused" => wire::node_execution_status_view::Value::Paused as i32,
                "waiting_approval" => {
                    wire::node_execution_status_view::Value::WaitingApproval as i32
                }
                "succeeded" => wire::node_execution_status_view::Value::Succeeded as i32,
                "failed" => wire::node_execution_status_view::Value::Failed as i32,
                "aborted" => wire::node_execution_status_view::Value::Aborted as i32,
                _ => return Err(format!("Invalid NodeExecutionStatusView: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::NodeExecutionStatusView {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::NodeExecutionView> for wire::NodeExecutionView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::NodeExecutionView,
    ) -> Result<Self, String> {
        Ok(Self {
            worktree: value.worktree.map(cv).transpose()?,
            recovery_reason: value.recovery_reason.map(cv).transpose()?,
            id: Some(cv(value.id)?),
            execution_id: Some(cv(value.execution_id)?),
            node_name: Some(cv(value.node_name)?),
            kind: Some(cv(value.kind)?),
            attempt: Some(cv(value.attempt)?),
            status: Some(cv(value.status)?),
            submit_received: Some(cv(value.submit_received)?),
            stop_received: Some(cv(value.stop_received)?),
            waiting_for: value.waiting_for.map(cv).transpose()?,
            can_approve: Some(cv(value.can_approve)?),
            can_retry: Some(cv(value.can_retry)?),
            has_artifact: Some(cv(value.has_artifact)?),
            session_id: value.session_id.map(cv).transpose()?,
            display_command: value.display_command.map(cv).transpose()?,
            result_summary: value.result_summary.map(cv).transpose()?,
            artifact: value.artifact.map(cv).transpose()?,
            token_usage: value.token_usage.map(cv).transpose()?,
            failure: value.failure.map(cv).transpose()?,
            parent: value.parent.map(cv).transpose()?,
            started_at: Some(cv(value.started_at)?),
            completed_at: value.completed_at.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::NodeKindView> for wire::NodeKindView {
    type Error = String;
    fn try_from(value: crate::adaptor::protocol::workflow::NodeKindView) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::workflow::NodeKindView::Command => {
                    wire::node_kind_view::Value::Command as i32
                }
                crate::adaptor::protocol::workflow::NodeKindView::Session => {
                    wire::node_kind_view::Value::Session as i32
                }
                crate::adaptor::protocol::workflow::NodeKindView::Fanout => {
                    wire::node_kind_view::Value::Fanout as i32
                }
                crate::adaptor::protocol::workflow::NodeKindView::Sequence => {
                    wire::node_kind_view::Value::Sequence as i32
                }
            }),
        })
    }
}

impl TryFrom<String> for wire::NodeKindView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "command" => wire::node_kind_view::Value::Command as i32,
                "session" => wire::node_kind_view::Value::Session as i32,
                "fanout" => wire::node_kind_view::Value::Fanout as i32,
                "sequence" => wire::node_kind_view::Value::Sequence as i32,
                _ => return Err(format!("Invalid NodeKindView: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::NodeKindView {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::usecase::workflow::NodeWorktreeDto> for wire::NodeWorktreeDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::NodeWorktreeDto) -> Result<Self, String> {
        Ok(Self {
            branch: Some(cv(value.branch)?),
            path: Some(cv(value.path)?),
        })
    }
}

impl<T> TryFrom<Option<T>> for wire::NullableNodeExecutionView
where
    wire::NodeExecutionView: TryFrom<T>,
    <wire::NodeExecutionView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Option<T>) -> Result<Self, String> {
        Ok(Self {
            value: value.map(cv).transpose()?,
        })
    }
}
impl<T> TryFrom<Option<T>> for wire::NullableWorkflowExecutionView
where
    wire::WorkflowExecutionView: TryFrom<T>,
    <wire::WorkflowExecutionView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Option<T>) -> Result<Self, String> {
        Ok(Self {
            value: value.map(cv).transpose()?,
        })
    }
}
impl<T> TryFrom<Option<T>> for wire::Nullablestring
where
    String: TryFrom<T>,
    <String as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Option<T>) -> Result<Self, String> {
        Ok(Self {
            value: value.map(cv).transpose()?,
        })
    }
}
impl TryFrom<crate::usecase::repository_state::snapshot::RepositoryBranchCardsSnapshotDto>
    for wire::RepositoryBranchCardsSnapshotDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::repository_state::snapshot::RepositoryBranchCardsSnapshotDto,
    ) -> Result<Self, String> {
        Ok(Self {
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            loading: Some(cv(value.loading)?),
            limited: Some(cv(value.limited)?),
            branches: Some(cv(value.branches)?),
            worktree_display_groups: Some(cv(value.worktree_display_groups)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_state::snapshot::RepositoryDiffStatsSnapshotDto>
    for wire::RepositoryDiffStatsSnapshotDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::repository_state::snapshot::RepositoryDiffStatsSnapshotDto,
    ) -> Result<Self, String> {
        Ok(Self {
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            loading: Some(cv(value.loading)?),
            limited: Some(cv(value.limited)?),
            diff_stats: Some(cv(value.diff_stats)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_state::snapshot::RepositorySnapshotChangedEvent>
    for wire::RepositorySnapshotChangedEvent
{
    type Error = String;
    fn try_from(
        value: crate::usecase::repository_state::snapshot::RepositorySnapshotChangedEvent,
    ) -> Result<Self, String> {
        Ok(Self {
            worktree_path: Some(cv(value.worktree_path)?),
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            loading: Some(cv(value.loading)?),
            limited: Some(cv(value.limited)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_state::snapshot::RepositoryStatusSnapshotDto>
    for wire::RepositoryStatusSnapshotDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::repository_state::snapshot::RepositoryStatusSnapshotDto,
    ) -> Result<Self, String> {
        Ok(Self {
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            loading: Some(cv(value.loading)?),
            limited: Some(cv(value.limited)?),
            status: Some(cv(value.status)?),
        })
    }
}

impl TryFrom<bool> for wire::ResultBool {
    type Error = String;
    fn try_from(value: bool) -> Result<Self, String> {
        Ok(Self { value: Some(value) })
    }
}

impl TryFrom<wire::ResultBool> for bool {
    type Error = String;
    fn try_from(value: wire::ResultBool) -> Result<Self, String> {
        req(value.value, "value")
    }
}

impl TryFrom<String> for wire::ResultString {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self { value: Some(value) })
    }
}

impl TryFrom<wire::ResultString> for String {
    type Error = String;
    fn try_from(value: wire::ResultString) -> Result<Self, String> {
        req(value.value, "value")
    }
}

impl TryFrom<u32> for wire::ResultUint32 {
    type Error = String;
    fn try_from(value: u32) -> Result<Self, String> {
        Ok(Self { value: Some(value) })
    }
}

impl TryFrom<u64> for wire::ResultUint64 {
    type Error = String;
    fn try_from(value: u64) -> Result<Self, String> {
        Ok(Self { value: Some(value) })
    }
}

impl TryFrom<wire::ResultUint64> for u64 {
    type Error = String;
    fn try_from(value: wire::ResultUint64) -> Result<Self, String> {
        req(value.value, "value")
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::TokenUsageView> for wire::TokenUsageView {
    type Error = String;
    fn try_from(value: crate::adaptor::protocol::workflow::TokenUsageView) -> Result<Self, String> {
        Ok(Self {
            input_tokens: Some(cv(value.input_tokens)?),
            output_tokens: Some(cv(value.output_tokens)?),
        })
    }
}

impl TryFrom<()> for wire::Unit {
    type Error = String;
    fn try_from(_value: ()) -> Result<Self, String> {
        Ok(Self {})
    }
}

impl TryFrom<wire::Unit> for () {
    type Error = String;
    fn try_from(value: wire::Unit) -> Result<Self, String> {
        let _ = value;
        Ok(())
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::WorkflowExecutionChangedPayloadView>
    for wire::WorkflowExecutionChangedPayloadView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::WorkflowExecutionChangedPayloadView,
    ) -> Result<Self, String> {
        Ok(Self {
            worktree_path: Some(cv(value.worktree_path)?),
            workflow_execution: Some(cv(value.workflow_execution)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::WorkflowExecutionView>
    for wire::WorkflowExecutionView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::WorkflowExecutionView,
    ) -> Result<Self, String> {
        Ok(Self {
            id: Some(cv(value.id)?),
            workflow_name: Some(cv(value.workflow_name)?),
            status: Some(cv(value.status)?),
            current_node: value.current_node.map(cv).transpose()?,
            worktree_path: Some(cv(value.worktree_path)?),
            created_from: Some(cv(value.created_from)?),
            started_at: Some(cv(value.started_at)?),
            updated_at: Some(cv(value.updated_at)?),
            completed_at: value.completed_at.map(cv).transpose()?,
            error_reason: value.error_reason.map(cv).transpose()?,
            interruption_reason: value.interruption_reason.map(cv).transpose()?,
            resume_from_node: value.resume_from_node.map(cv).transpose()?,
            total_token_usage: Some(cv(value.total_token_usage)?),
            node_executions: Some(cv(value.node_executions)?),
            artifacts: Some(cv(value.artifacts)?),
            fanouts: Some(cv(value.fanouts)?),
            approval_target: value.approval_target.map(cv).transpose()?,
        })
    }
}

impl TryFrom<i64> for wire::WorkflowInteger {
    type Error = String;
    fn try_from(value: i64) -> Result<Self, String> {
        Ok(Self { value: Some(value) })
    }
}

impl TryFrom<wire::WorkflowInteger> for i64 {
    type Error = String;
    fn try_from(value: wire::WorkflowInteger) -> Result<Self, String> {
        req(value.value, "value")
    }
}

impl TryFrom<f64> for wire::WorkflowNumber {
    type Error = String;
    fn try_from(value: f64) -> Result<Self, String> {
        Ok(Self { value: Some(value) })
    }
}

impl TryFrom<wire::WorkflowNumber> for f64 {
    type Error = String;
    fn try_from(value: wire::WorkflowNumber) -> Result<Self, String> {
        req(value.value, "value")
    }
}

impl<T> TryFrom<Vec<T>> for wire::WorkflowValueList
where
    wire::WorkflowValue: TryFrom<T>,
    <wire::WorkflowValue as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T: TryFrom<wire::WorkflowValue>> TryFrom<wire::WorkflowValueList> for Vec<T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::WorkflowValueList) -> Result<Self, String> {
        value.items.into_iter().map(cv).collect()
    }
}
impl<T> TryFrom<std::collections::BTreeMap<String, T>> for wire::WorkflowValueObject
where
    wire::WorkflowValue: TryFrom<T>,
    <wire::WorkflowValue as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: std::collections::BTreeMap<String, T>) -> Result<Self, String> {
        Ok(Self {
            entries: value
                .into_iter()
                .map(|(key, value)| Ok((key, cv(value)?)))
                .collect::<Result<_, String>>()?,
        })
    }
}
impl<T: TryFrom<wire::WorkflowValue>> TryFrom<wire::WorkflowValueObject>
    for std::collections::BTreeMap<String, T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::WorkflowValueObject) -> Result<Self, String> {
        value
            .entries
            .into_iter()
            .map(|(key, value)| Ok((key, cv(value)?)))
            .collect()
    }
}
impl<T> TryFrom<std::collections::HashMap<String, T>> for wire::WorkflowValueObject
where
    wire::WorkflowValue: TryFrom<T>,
    <wire::WorkflowValue as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: std::collections::HashMap<String, T>) -> Result<Self, String> {
        Ok(Self {
            entries: value
                .into_iter()
                .map(|(key, value)| Ok((key, cv(value)?)))
                .collect::<Result<_, String>>()?,
        })
    }
}
impl<T: TryFrom<wire::WorkflowValue>> TryFrom<wire::WorkflowValueObject>
    for std::collections::HashMap<String, T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::WorkflowValueObject) -> Result<Self, String> {
        value
            .entries
            .into_iter()
            .map(|(key, value)| Ok((key, cv(value)?)))
            .collect()
    }
}
impl TryFrom<crate::usecase::repository_dto::WorktreeDisplayGroupsDto>
    for wire::WorktreeDisplayGroupsDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::repository_dto::WorktreeDisplayGroupsDto,
    ) -> Result<Self, String> {
        Ok(Self {
            working_areas: Some(cv(value.working_areas)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_dto::WorktreeEntryDto> for wire::WorktreeEntryDto {
    type Error = String;
    fn try_from(value: crate::usecase::repository_dto::WorktreeEntryDto) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            path: Some(cv(value.path)?),
            branch: Some(cv(value.branch)?),
            is_main: Some(cv(value.is_main)?),
            is_locked: Some(cv(value.is_locked)?),
            dirty_count: Some(cv(value.dirty_count)?),
            base_branch: value.base_branch.map(cv).transpose()?,
        })
    }
}
