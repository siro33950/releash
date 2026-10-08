use super as wire;
fn cv<T, U: TryFrom<T>>(value: T) -> Result<U, String>
where
    U::Error: std::fmt::Display,
{
    U::try_from(value).map_err(|error| error.to_string())
}
fn req<T>(value: Option<T>, name: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("Missing {name}"))
}
impl TryFrom<String> for wire::AgentSessionArchiveResponse {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "archived" => wire::agent_session_archive_response::Value::Archived as i32,
                "already_archived" => {
                    wire::agent_session_archive_response::Value::AlreadyArchived as i32
                }
                _ => return Err(format!("Invalid AgentSessionArchiveResponse: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::AgentSessionArchiveResponse {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::AgentSessionLifecycleDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "open" => wire::agent_session_lifecycle_dto::Value::Open as i32,
                "paused" => wire::agent_session_lifecycle_dto::Value::Paused as i32,
                "archived" => wire::agent_session_lifecycle_dto::Value::Archived as i32,
                _ => return Err(format!("Invalid AgentSessionLifecycleDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::AgentSessionLifecycleDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::AgentSessionProviderDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "claude" => wire::agent_session_provider_dto::Value::Claude as i32,
                "codex" => wire::agent_session_provider_dto::Value::Codex as i32,
                _ => return Err(format!("Invalid AgentSessionProviderDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::AgentSessionProviderDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::CompletionRequirementDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "approval" => wire::completion_requirement_dto::Value::Approval as i32,
                _ => return Err(format!("Invalid CompletionRequirementDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::CompletionRequirementDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::DiagnosticStage {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "parse_shape" => wire::diagnostic_stage::Value::ParseShape as i32,
                "resolve" => wire::diagnostic_stage::Value::Resolve as i32,
                "typecheck" => wire::diagnostic_stage::Value::Typecheck as i32,
                "control_flow" => wire::diagnostic_stage::Value::ControlFlow as i32,
                _ => return Err(format!("Invalid DiagnosticStage: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::DiagnosticStage {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::DiffBase {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "branch-base" => wire::diff_base::Value::BranchBase as i32,
                "head" => wire::diff_base::Value::Head as i32,
                _ => return Err(format!("Invalid DiffBase: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::DiffBase {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::DiffRangeKindDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "added" => wire::diff_range_kind_dto::Value::Added as i32,
                "modified" => wire::diff_range_kind_dto::Value::Modified as i32,
                "deleted" => wire::diff_range_kind_dto::Value::Deleted as i32,
                _ => return Err(format!("Invalid DiffRangeKindDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::DiffRangeKindDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::DiffTreeNodeType {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "file" => wire::diff_tree_node_type::Value::File as i32,
                "folder" => wire::diff_tree_node_type::Value::Folder as i32,
                _ => return Err(format!("Invalid DiffTreeNodeType: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::DiffTreeNodeType {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<wire::DiffTreeNodeType> for String {
    type Error = String;
    fn try_from(value: wire::DiffTreeNodeType) -> Result<Self, String> {
        Ok(
            match wire::diff_tree_node_type::Value::try_from(req(value.value, "value")?)
                .map_err(|_| "Invalid DiffTreeNodeType")?
            {
                wire::diff_tree_node_type::Value::File => "file".to_owned(),
                wire::diff_tree_node_type::Value::Folder => "folder".to_owned(),
            },
        )
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
impl TryFrom<String> for wire::ExecutionStatusView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "running" => wire::execution_status_view::Value::Running as i32,
                "completed" => wire::execution_status_view::Value::Completed as i32,
                "aborted" => wire::execution_status_view::Value::Aborted as i32,
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
impl TryFrom<String> for wire::InlineChunkKindDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "unchanged" => wire::inline_chunk_kind_dto::Value::Unchanged as i32,
                "added" => wire::inline_chunk_kind_dto::Value::Added as i32,
                "removed" => wire::inline_chunk_kind_dto::Value::Removed as i32,
                _ => return Err(format!("Invalid InlineChunkKindDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::InlineChunkKindDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListAgentSessionHistoryCandidateDto
where
    wire::AgentSessionHistoryCandidateDto: TryFrom<T>,
    <wire::AgentSessionHistoryCandidateDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListAgentSessionItemDto
where
    wire::AgentSessionItemDto: TryFrom<T>,
    <wire::AgentSessionItemDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListAgentSessionProviderDto
where
    wire::AgentSessionProviderDto: TryFrom<T>,
    <wire::AgentSessionProviderDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
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
impl<T> TryFrom<Vec<T>> for wire::ListChangeGroupDto
where
    wire::ChangeGroupDto: TryFrom<T>,
    <wire::ChangeGroupDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListChildEntryDto
where
    wire::ChildEntryDto: TryFrom<T>,
    <wire::ChildEntryDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListChildInputDto
where
    wire::ChildInputDto: TryFrom<T>,
    <wire::ChildInputDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListDiagnosticItem
where
    wire::DiagnosticItem: TryFrom<T>,
    <wire::DiagnosticItem as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T: TryFrom<wire::DiffFileEntryInput>> TryFrom<wire::ListDiffFileEntryInput> for Vec<T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::ListDiffFileEntryInput) -> Result<Self, String> {
        value.items.into_iter().map(cv).collect()
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListDiffRangeDto
where
    wire::DiffRangeDto: TryFrom<T>,
    <wire::DiffRangeDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListDiffTreeNodeDto
where
    wire::DiffTreeNodeDto: TryFrom<T>,
    <wire::DiffTreeNodeDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T: TryFrom<wire::DiffTreeNodeInput>> TryFrom<wire::ListDiffTreeNodeInput> for Vec<T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::ListDiffTreeNodeInput) -> Result<Self, String> {
        value.items.into_iter().map(cv).collect()
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListEditorInfoDto
where
    wire::EditorInfoDto: TryFrom<T>,
    <wire::EditorInfoDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListFacetSummaryDto
where
    wire::FacetSummaryDto: TryFrom<T>,
    <wire::FacetSummaryDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListFacetUsageEntry
where
    wire::FacetUsageEntry: TryFrom<T>,
    <wire::FacetUsageEntry as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<Vec<T>> for wire::ListHiddenRangeDto
where
    wire::HiddenRangeDto: TryFrom<T>,
    <wire::HiddenRangeDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListHunkDto
where
    wire::HunkDto: TryFrom<T>,
    <wire::HunkDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T: TryFrom<wire::HunkInput>> TryFrom<wire::ListHunkInput> for Vec<T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::ListHunkInput) -> Result<Self, String> {
        value.items.into_iter().map(cv).collect()
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListInlineChunkDto
where
    wire::InlineChunkDto: TryFrom<T>,
    <wire::InlineChunkDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListInputParamDto
where
    wire::InputParamDto: TryFrom<T>,
    <wire::InputParamDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListIssueInfoDto
where
    wire::IssueInfoDto: TryFrom<T>,
    <wire::IssueInfoDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListIssueLabelDto
where
    wire::IssueLabelDto: TryFrom<T>,
    <wire::IssueLabelDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListLabelPropertyView
where
    wire::LabelPropertyView: TryFrom<T>,
    <wire::LabelPropertyView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T: TryFrom<wire::LabelPropertyView>> TryFrom<wire::ListLabelPropertyView> for Vec<T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::ListLabelPropertyView) -> Result<Self, String> {
        value.items.into_iter().map(cv).collect()
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListNodeDefinitionDto
where
    wire::NodeDefinitionDto: TryFrom<T>,
    <wire::NodeDefinitionDto as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<Vec<T>> for wire::ListNotionLabelOptionView
where
    wire::NotionLabelOptionView: TryFrom<T>,
    <wire::NotionLabelOptionView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListNotionPropertyInfoView
where
    wire::NotionPropertyInfoView: TryFrom<T>,
    <wire::NotionPropertyInfoView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListNotionTaskView
where
    wire::NotionTaskView: TryFrom<T>,
    <wire::NotionTaskView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListPrAuthorDto
where
    wire::PrAuthorDto: TryFrom<T>,
    <wire::PrAuthorDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListPredicateDto
where
    wire::PredicateDto: TryFrom<T>,
    <wire::PredicateDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListProviderAvailabilityItemResponse
where
    wire::ProviderAvailabilityItemResponse: TryFrom<T>,
    <wire::ProviderAvailabilityItemResponse as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListProviderHookHealthWarningResponse
where
    wire::ProviderHookHealthWarningResponse: TryFrom<T>,
    <wire::ProviderHookHealthWarningResponse as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListReviewCommentDto
where
    wire::ReviewCommentDto: TryFrom<T>,
    <wire::ReviewCommentDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListReviewFileEntryDto
where
    wire::ReviewFileEntryDto: TryFrom<T>,
    <wire::ReviewFileEntryDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListReviewThreadDto
where
    wire::ReviewThreadDto: TryFrom<T>,
    <wire::ReviewThreadDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListRuleDto
where
    wire::RuleDto: TryFrom<T>,
    <wire::RuleDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListSplitRowDto
where
    wire::SplitRowDto: TryFrom<T>,
    <wire::SplitRowDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListVisibleBlockDto
where
    wire::VisibleBlockDto: TryFrom<T>,
    <wire::VisibleBlockDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListWorkflowSummaryDto
where
    wire::WorkflowSummaryDto: TryFrom<T>,
    <wire::WorkflowSummaryDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListWorkspaceTabEntryDto
where
    wire::WorkspaceTabEntryDto: TryFrom<T>,
    <wire::WorkspaceTabEntryDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T: TryFrom<wire::WorkspaceTabEntryDto>> TryFrom<wire::ListWorkspaceTabEntryDto> for Vec<T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::ListWorkspaceTabEntryDto) -> Result<Self, String> {
        value.items.into_iter().map(cv).collect()
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
impl<T> TryFrom<std::collections::BTreeMap<String, T>> for wire::MapDiagnosticSummary
where
    wire::DiagnosticSummary: TryFrom<T>,
    <wire::DiagnosticSummary as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<std::collections::HashMap<String, T>> for wire::MapDiagnosticSummary
where
    wire::DiagnosticSummary: TryFrom<T>,
    <wire::DiagnosticSummary as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<std::collections::BTreeMap<String, T>> for wire::MapListFacetUsageEntry
where
    wire::ListFacetUsageEntry: TryFrom<T>,
    <wire::ListFacetUsageEntry as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<std::collections::HashMap<String, T>> for wire::MapListFacetUsageEntry
where
    wire::ListFacetUsageEntry: TryFrom<T>,
    <wire::ListFacetUsageEntry as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<std::collections::BTreeMap<String, T>> for wire::Mapstring
where
    String: TryFrom<T>,
    <String as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T: TryFrom<String>> TryFrom<wire::Mapstring> for std::collections::BTreeMap<String, T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::Mapstring) -> Result<Self, String> {
        value
            .entries
            .into_iter()
            .map(|(key, value)| Ok((key, cv(value)?)))
            .collect()
    }
}
impl<T> TryFrom<std::collections::HashMap<String, T>> for wire::Mapstring
where
    String: TryFrom<T>,
    <String as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T: TryFrom<String>> TryFrom<wire::Mapstring> for std::collections::HashMap<String, T>
where
    T::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: wire::Mapstring) -> Result<Self, String> {
        value
            .entries
            .into_iter()
            .map(|(key, value)| Ok((key, cv(value)?)))
            .collect()
    }
}
impl TryFrom<wire::MarkdownDiffSideInput> for String {
    type Error = String;
    fn try_from(value: wire::MarkdownDiffSideInput) -> Result<Self, String> {
        Ok(
            match wire::markdown_diff_side_input::Value::try_from(req(value.value, "value")?)
                .map_err(|_| "Invalid MarkdownDiffSideInput")?
            {
                wire::markdown_diff_side_input::Value::Modified => "modified".to_owned(),
                wire::markdown_diff_side_input::Value::Original => "original".to_owned(),
            },
        )
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
impl TryFrom<String> for wire::NodeExecutionStatusView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "running" => wire::node_execution_status_view::Value::Running as i32,
                "waiting_approval" => {
                    wire::node_execution_status_view::Value::WaitingApproval as i32
                }
                "succeeded" => wire::node_execution_status_view::Value::Succeeded as i32,
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
impl TryFrom<String> for wire::NodeKindDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "session" => wire::node_kind_dto::Value::Session as i32,
                "command" => wire::node_kind_dto::Value::Command as i32,
                "fanout" => wire::node_kind_dto::Value::Fanout as i32,
                "sequence" => wire::node_kind_dto::Value::Sequence as i32,
                _ => return Err(format!("Invalid NodeKindDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::NodeKindDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
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
impl TryFrom<String> for wire::NotionConfigStatusView {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "not_configured" => wire::notion_config_status_view::Value::NotConfigured as i32,
                "configured" => wire::notion_config_status_view::Value::Configured as i32,
                "invalid_token" => wire::notion_config_status_view::Value::InvalidToken as i32,
                "invalid_database" => {
                    wire::notion_config_status_view::Value::InvalidDatabase as i32
                }
                "network_error" => wire::notion_config_status_view::Value::NetworkError as i32,
                _ => return Err(format!("Invalid NotionConfigStatusView: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::NotionConfigStatusView {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl<T> TryFrom<Option<T>> for wire::NullableAgentSessionItemDto
where
    wire::AgentSessionItemDto: TryFrom<T>,
    <wire::AgentSessionItemDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Option<T>) -> Result<Self, String> {
        Ok(Self {
            value: value.map(cv).transpose()?,
        })
    }
}
impl<T> TryFrom<Option<T>> for wire::NullableNotionRepoConfigView
where
    wire::NotionRepoConfigView: TryFrom<T>,
    <wire::NotionRepoConfigView as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Option<T>) -> Result<Self, String> {
        Ok(Self {
            value: value.map(cv).transpose()?,
        })
    }
}
impl<T> TryFrom<Option<T>> for wire::NullableWorkspaceNodeDetailDto
where
    wire::WorkspaceNodeDetailDto: TryFrom<T>,
    <wire::WorkspaceNodeDetailDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Option<T>) -> Result<Self, String> {
        Ok(Self {
            value: value.map(cv).transpose()?,
        })
    }
}
impl<T> TryFrom<Option<T>> for wire::NullableWorkflowDto
where
    wire::WorkflowDto: TryFrom<T>,
    <wire::WorkflowDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Option<T>) -> Result<Self, String> {
        Ok(Self {
            value: value.map(cv).transpose()?,
        })
    }
}
impl<T> TryFrom<Option<T>> for wire::NullableWorkspaceStateDto
where
    wire::WorkspaceStateDto: TryFrom<T>,
    <wire::WorkspaceStateDto as TryFrom<T>>::Error: std::fmt::Display,
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
impl TryFrom<String> for wire::ProviderHookHealthProviderResponse {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "claude" => wire::provider_hook_health_provider_response::Value::Claude as i32,
                "codex" => wire::provider_hook_health_provider_response::Value::Codex as i32,
                _ => {
                    return Err(format!(
                        "Invalid ProviderHookHealthProviderResponse: {value}"
                    ))
                }
            }),
        })
    }
}
impl TryFrom<&str> for wire::ProviderHookHealthProviderResponse {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
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
impl From<u64> for wire::ResultUint64 {
    fn from(value: u64) -> Self {
        Self { value: Some(value) }
    }
}
impl TryFrom<wire::ResultUint64> for u64 {
    type Error = String;
    fn try_from(value: wire::ResultUint64) -> Result<Self, String> {
        req(value.value, "value")
    }
}
impl TryFrom<String> for wire::ReviewActorKindWireDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "human" => wire::review_actor_kind_wire_dto::Value::Human as i32,
                "agent" => wire::review_actor_kind_wire_dto::Value::Agent as i32,
                _ => return Err(format!("Invalid ReviewActorKindWireDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::ReviewActorKindWireDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::ReviewLimitReasonDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "fileSize" => wire::review_limit_reason_dto::Value::FileSize as i32,
                "lineCount" => wire::review_limit_reason_dto::Value::LineCount as i32,
                "hunkCount" => wire::review_limit_reason_dto::Value::HunkCount as i32,
                "tokenization" => wire::review_limit_reason_dto::Value::Tokenization as i32,
                _ => return Err(format!("Invalid ReviewLimitReasonDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::ReviewLimitReasonDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::ReviewTextSource {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "diff" => wire::review_text_source::Value::Diff as i32,
                "added" => wire::review_text_source::Value::Added as i32,
                "deleted" => wire::review_text_source::Value::Deleted as i32,
                _ => return Err(format!("Invalid ReviewTextSource: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::ReviewTextSource {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::ReviewThreadStateDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "open" => wire::review_thread_state_dto::Value::Open as i32,
                "resolved" => wire::review_thread_state_dto::Value::Resolved as i32,
                _ => return Err(format!("Invalid ReviewThreadStateDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::ReviewThreadStateDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<wire::ReviewThreadStateDto> for String {
    type Error = String;
    fn try_from(value: wire::ReviewThreadStateDto) -> Result<Self, String> {
        Ok(
            match wire::review_thread_state_dto::Value::try_from(req(value.value, "value")?)
                .map_err(|_| "Invalid ReviewThreadStateDto")?
            {
                wire::review_thread_state_dto::Value::Open => "open".to_owned(),
                wire::review_thread_state_dto::Value::Resolved => "resolved".to_owned(),
            },
        )
    }
}
impl TryFrom<String> for wire::SessionProviderDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "claude" => wire::session_provider_dto::Value::Claude as i32,
                "codex" => wire::session_provider_dto::Value::Codex as i32,
                _ => return Err(format!("Invalid SessionProviderDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::SessionProviderDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::Severity {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "error" => wire::severity::Value::Error as i32,
                "info" => wire::severity::Value::Info as i32,
                _ => return Err(format!("Invalid Severity: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::Severity {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::SplitRowKindDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "unchanged" => wire::split_row_kind_dto::Value::Unchanged as i32,
                "added" => wire::split_row_kind_dto::Value::Added as i32,
                "removed" => wire::split_row_kind_dto::Value::Removed as i32,
                "modified" => wire::split_row_kind_dto::Value::Modified as i32,
                _ => return Err(format!("Invalid SplitRowKindDto: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::SplitRowKindDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
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
impl TryFrom<String> for wire::WorkflowSourceFormat {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "yaml" => wire::workflow_source_format::Value::Yaml as i32,
                "lua" => wire::workflow_source_format::Value::Lua as i32,
                _ => return Err(format!("Invalid WorkflowSourceFormat: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::WorkflowSourceFormat {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
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
impl TryFrom<String> for wire::WorkspaceCenterTab {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "agent" => wire::workspace_center_tab::Value::Agent as i32,
                "editor" => wire::workspace_center_tab::Value::Editor as i32,
                _ => return Err(format!("Invalid WorkspaceCenterTab: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::WorkspaceCenterTab {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<wire::WorkspaceCenterTab> for String {
    type Error = String;
    fn try_from(value: wire::WorkspaceCenterTab) -> Result<Self, String> {
        Ok(
            match wire::workspace_center_tab::Value::try_from(req(value.value, "value")?)
                .map_err(|_| "Invalid WorkspaceCenterTab")?
            {
                wire::workspace_center_tab::Value::Agent => "agent".to_owned(),
                wire::workspace_center_tab::Value::Editor => "editor".to_owned(),
            },
        )
    }
}
impl TryFrom<String> for wire::WorkspaceContentKind {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "session" => wire::workspace_content_kind::Value::Session as i32,
                "command" => wire::workspace_content_kind::Value::Command as i32,
                _ => return Err(format!("Invalid WorkspaceContentKind: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::WorkspaceContentKind {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::WorkspaceHistoryStatus {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "running" => wire::workspace_history_status::Value::Running as i32,
                "waiting" => wire::workspace_history_status::Value::Waiting as i32,
                "aborted" => wire::workspace_history_status::Value::Aborted as i32,
                "completed" => wire::workspace_history_status::Value::Completed as i32,
                _ => return Err(format!("Invalid WorkspaceHistoryStatus: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::WorkspaceHistoryStatus {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::WorkspaceNodeStatus {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "running" => wire::workspace_node_status::Value::Running as i32,
                "waiting" => wire::workspace_node_status::Value::Waiting as i32,
                "aborted" => wire::workspace_node_status::Value::Aborted as i32,
                "completed" => wire::workspace_node_status::Value::Completed as i32,
                _ => return Err(format!("Invalid WorkspaceNodeStatus: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::WorkspaceNodeStatus {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::WorkspaceStatusClassification {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "active" => wire::workspace_status_classification::Value::Active as i32,
                "attention" => wire::workspace_status_classification::Value::Attention as i32,
                "idle" => wire::workspace_status_classification::Value::Idle as i32,
                _ => return Err(format!("Invalid WorkspaceStatusClassification: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::WorkspaceStatusClassification {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::WorkspaceWaitingFor {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "submit" => wire::workspace_waiting_for::Value::Submit as i32,
                "stop" => wire::workspace_waiting_for::Value::Stop as i32,
                _ => return Err(format!("Invalid WorkspaceWaitingFor: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::WorkspaceWaitingFor {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::WorktreeMode {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "shared" => wire::worktree_mode::Value::Shared as i32,
                "isolated" => wire::worktree_mode::Value::Isolated as i32,
                _ => return Err(format!("Invalid WorktreeMode: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::WorktreeMode {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}
impl TryFrom<String> for wire::NodeProcessPresence {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "unknown" => wire::node_process_presence::Value::Unknown as i32,
                "live" => wire::node_process_presence::Value::Live as i32,
                "confirmed_absent" => wire::node_process_presence::Value::ConfirmedAbsent as i32,
                _ => return Err(format!("Invalid NodeProcessPresence: {value}")),
            }),
        })
    }
}
impl TryFrom<&str> for wire::NodeProcessPresence {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

use serde_json::Value;

impl TryFrom<Value> for wire::WorkflowValue {
    type Error = String;
    fn try_from(value: Value) -> Result<Self, String> {
        use wire::workflow_value::Variant;
        let variant = match value {
            Value::Null => Variant::NullValue(wire::Unit {}),
            Value::Bool(value) => Variant::BooleanValue(wire::ResultBool { value: Some(value) }),
            Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    Variant::SignedInteger(wire::WorkflowInteger { value: Some(value) })
                } else if let Some(value) = value.as_u64() {
                    Variant::UnsignedInteger(wire::ResultUint64 { value: Some(value) })
                } else {
                    Variant::NumberValue(wire::WorkflowNumber {
                        value: Some(value.as_f64().ok_or("Invalid workflow number")?),
                    })
                }
            }
            Value::String(value) => Variant::StringValue(wire::ResultString { value: Some(value) }),
            Value::Array(value) => Variant::ListValue(wire::WorkflowValueList {
                items: value
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()?,
            }),
            Value::Object(value) => Variant::ObjectValue(wire::WorkflowValueObject {
                entries: value
                    .into_iter()
                    .map(|(key, value)| Ok((key, value.try_into()?)))
                    .collect::<Result<_, String>>()?,
            }),
        };
        Ok(Self {
            variant: Some(variant),
        })
    }
}

impl TryFrom<wire::WorkflowValue> for Value {
    type Error = String;
    fn try_from(value: wire::WorkflowValue) -> Result<Self, String> {
        use wire::workflow_value::Variant;
        Ok(match value.variant.ok_or("Missing workflow value")? {
            Variant::NullValue(_) => Value::Null,
            Variant::BooleanValue(value) => value.value.ok_or("Missing boolean")?.into(),
            Variant::SignedInteger(value) => value.value.ok_or("Missing signed integer")?.into(),
            Variant::UnsignedInteger(value) => {
                value.value.ok_or("Missing unsigned integer")?.into()
            }
            Variant::NumberValue(value) => Value::Number(
                serde_json::Number::from_f64(value.value.ok_or("Missing number")?)
                    .ok_or("Nonfinite workflow number")?,
            ),
            Variant::StringValue(value) => value.value.ok_or("Missing string")?.into(),
            Variant::ListValue(value) => Value::Array(
                value
                    .items
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()?,
            ),
            Variant::ObjectValue(value) => Value::Object(
                value
                    .entries
                    .into_iter()
                    .map(|(key, value)| Ok((key, value.try_into()?)))
                    .collect::<Result<_, String>>()?,
            ),
        })
    }
}

impl From<()> for wire::StopDaemonResponse {
    fn from(_: ()) -> Self {
        Self {}
    }
}
