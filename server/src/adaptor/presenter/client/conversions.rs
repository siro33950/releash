use super as wire;

pub(super) fn cv<T, U: TryFrom<T>>(value: T) -> Result<U, String>
where
    U::Error: std::fmt::Display,
{
    U::try_from(value).map_err(|error| error.to_string())
}
fn req<T>(value: Option<T>, name: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("Missing {name}"))
}

impl TryFrom<crate::adaptor::presenter::agent_session::AgentSessionArchiveResponse>
    for wire::AgentSessionArchiveResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::agent_session::AgentSessionArchiveResponse,
    ) -> Result<Self, String> {
        Ok(Self { value: Some(match value { crate::adaptor::presenter::agent_session::AgentSessionArchiveResponse::Archived => wire::agent_session_archive_response::Value::Archived as i32, crate::adaptor::presenter::agent_session::AgentSessionArchiveResponse::AlreadyArchived => wire::agent_session_archive_response::Value::AlreadyArchived as i32 }) })
    }
}

impl TryFrom<crate::usecase::agent_session::AgentSessionHistoryCandidateDto>
    for wire::AgentSessionHistoryCandidateDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::agent_session::AgentSessionHistoryCandidateDto,
    ) -> Result<Self, String> {
        Ok(Self {
            provider: Some(cv(value.provider)?),
            provider_session_id: Some(cv(value.provider_session_id)?),
            label: Some(cv(value.label)?),
            updated_at_ms: Some(cv(value.updated_at_ms)?),
        })
    }
}

impl TryFrom<crate::usecase::agent_session::AgentSessionHistoryPageDto>
    for wire::AgentSessionHistoryPageDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::agent_session::AgentSessionHistoryPageDto,
    ) -> Result<Self, String> {
        Ok(Self {
            items: Some(cv(value.items)?),
            has_more: Some(cv(value.has_more)?),
        })
    }
}

impl TryFrom<crate::usecase::agent_session::AgentSessionItemDto> for wire::AgentSessionItemDto {
    type Error = String;
    fn try_from(value: crate::usecase::agent_session::AgentSessionItemDto) -> Result<Self, String> {
        Ok(Self {
            id: Some(cv(value.id)?),
            workspace_identity: Some(cv(value.workspace_identity)?),
            worktree_path: Some(cv(value.worktree_path)?),
            workspace_worktree_path: Some(cv(value.workspace_worktree_path)?),
            provider: Some(cv(value.provider)?),
            tree_location: Some(cv(value.tree_location)?),
            lifecycle: Some(cv(value.lifecycle)?),
            provider_session_id: value.provider_session_id.map(cv).transpose()?,
            transcript_ref: value.transcript_ref.map(cv).transpose()?,
            operations: Some(cv(value.operations)?),
            last_exit_abnormal: Some(cv(value.last_exit_abnormal)?),
            terminal_presence: value.terminal_presence,
        })
    }
}

impl TryFrom<crate::usecase::agent_session::AgentSessionLifecycleDto>
    for wire::AgentSessionLifecycleDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::agent_session::AgentSessionLifecycleDto,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::agent_session::AgentSessionLifecycleDto::Open => {
                    wire::agent_session_lifecycle_dto::Value::Open as i32
                }
                crate::usecase::agent_session::AgentSessionLifecycleDto::Paused => {
                    wire::agent_session_lifecycle_dto::Value::Paused as i32
                }
                crate::usecase::agent_session::AgentSessionLifecycleDto::Archived => {
                    wire::agent_session_lifecycle_dto::Value::Archived as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::agent_session::AgentSessionOperationsDto>
    for wire::AgentSessionOperationsDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::agent_session::AgentSessionOperationsDto,
    ) -> Result<Self, String> {
        Ok(Self {
            can_archive: Some(cv(value.can_archive)?),
            can_restore: Some(cv(value.can_restore)?),
            can_delete: Some(cv(value.can_delete)?),
        })
    }
}

impl TryFrom<crate::usecase::provider_dto::AgentSessionProviderDto>
    for wire::AgentSessionProviderDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::provider_dto::AgentSessionProviderDto,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::provider_dto::AgentSessionProviderDto::Claude => {
                    wire::agent_session_provider_dto::Value::Claude as i32
                }
                crate::usecase::provider_dto::AgentSessionProviderDto::Codex => {
                    wire::agent_session_provider_dto::Value::Codex as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::agent_session::AgentSessionTreeLocationDto>
    for wire::AgentSessionTreeLocationDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::agent_session::AgentSessionTreeLocationDto,
    ) -> Result<Self, String> {
        Ok(Self {
            tree_id: Some(cv(value.tree_id)?),
            node_execution_id: Some(cv(value.node_execution_id)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::ApprovalTargetView>
    for wire::ApprovalTargetView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::ApprovalTargetView,
    ) -> Result<Self, String> {
        Ok(Self {
            node_execution_id: Some(cv(value.node_execution_id)?),
            node_name: Some(cv(value.node_name)?),
            session_id: value.session_id.map(cv).transpose()?,
        })
    }
}

impl TryFrom<wire::ApproveWorkflowNodeArgs> for crate::usecase::workflow::command::ApprovalCommand {
    type Error = String;
    fn try_from(value: wire::ApproveWorkflowNodeArgs) -> Result<Self, String> {
        Ok(Self {
            execution_id: cv(req(value.execution_id, "executionId")?)?,
            node_name: cv(req(value.node_name, "nodeName")?)?,
            node_execution_id: value.node_execution_id.map(cv).transpose()?,
            comment: value.comment.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::ArtifactView> for wire::ArtifactView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::ArtifactView,
    ) -> Result<Self, String> {
        Ok(Self {
            node_name: Some(cv(value.node_name)?),
            contract: value.contract.map(cv).transpose()?,
            value: Some(cv(value.value)?),
            produced_at: Some(cv(value.produced_at)?),
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

impl TryFrom<crate::usecase::code_dto::ChangeGroupDto> for wire::ChangeGroupDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ChangeGroupDto) -> Result<Self, String> {
        Ok(Self {
            group_index: Some(cv(value.group_index)?),
            group_id: Some(cv(value.group_id)?),
            hunk_index: Some(cv(value.hunk_index)?),
            new_start: Some(cv(value.new_start)?),
            new_end: Some(cv(value.new_end)?),
            line_offset_start: Some(cv(value.line_offset_start)?),
            line_offset_end: Some(cv(value.line_offset_end)?),
            is_staged: value.is_staged.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::ChildEntryDto> for wire::ChildEntryDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::ChildEntryDto) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            inputs: Some(cv(value.inputs)?),
            rules: value.rules.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::ChildInputDto> for wire::ChildInputDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::ChildInputDto) -> Result<Self, String> {
        Ok(Self {
            parameter: Some(cv(value.parameter)?),
            source: Some(cv(value.source)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::CompletionRequirementDto>
    for wire::CompletionRequirementDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::dto::CompletionRequirementDto,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::workflow::dto::CompletionRequirementDto::Approval => {
                    wire::completion_requirement_dto::Value::Approval as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::DiagnosticItem> for wire::DiagnosticItem {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::DiagnosticItem,
    ) -> Result<Self, String> {
        Ok(Self {
            code: Some(cv(value.code)?),
            severity: Some(cv(value.severity)?),
            stage: Some(cv(value.stage)?),
            span: value.span.map(cv).transpose()?,
            message: Some(cv(value.message)?),
            workflow_name: value.workflow_name.map(cv).transpose()?,
            node_name: value.node_name.map(cv).transpose()?,
            facet_key: value.facet_key.map(cv).transpose()?,
            facet_kind: value.facet_kind.map(cv).transpose()?,
            field: value.field.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::DiagnosticReport>
    for wire::DiagnosticReport
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::DiagnosticReport,
    ) -> Result<Self, String> {
        Ok(Self {
            items: Some(cv(value.items)?),
            workflow_summaries: Some(cv(value.workflow_summaries)?),
            facet_summaries: Some(cv(value.facet_summaries)?),
            facet_usage: Some(cv(value.facet_usage)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::DiagnosticSpan> for wire::DiagnosticSpan {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::DiagnosticSpan,
    ) -> Result<Self, String> {
        Ok(Self {
            source: value.source.map(cv).transpose()?,
            start_line: Some(cv(value.start_line)?),
            start_col: Some(cv(value.start_col)?),
            end_line: Some(cv(value.end_line)?),
            end_col: Some(cv(value.end_col)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::DiagnosticStage> for wire::DiagnosticStage {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::DiagnosticStage,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::presenter::workflow_wire::DiagnosticStage::ParseShape => {
                    wire::diagnostic_stage::Value::ParseShape as i32
                }
                crate::adaptor::presenter::workflow_wire::DiagnosticStage::Resolve => {
                    wire::diagnostic_stage::Value::Resolve as i32
                }
                crate::adaptor::presenter::workflow_wire::DiagnosticStage::Typecheck => {
                    wire::diagnostic_stage::Value::Typecheck as i32
                }
                crate::adaptor::presenter::workflow_wire::DiagnosticStage::ControlFlow => {
                    wire::diagnostic_stage::Value::ControlFlow as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::DiagnosticSummary>
    for wire::DiagnosticSummary
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::DiagnosticSummary,
    ) -> Result<Self, String> {
        Ok(Self {
            error_count: Some(cv(value.error_count)?),
            info_count: Some(cv(value.info_count)?),
        })
    }
}

impl TryFrom<wire::DiffFileEntryInput> for crate::adaptor::presenter::code::DiffFileEntryInput {
    type Error = String;
    fn try_from(value: wire::DiffFileEntryInput) -> Result<Self, String> {
        Ok(Self {
            path: cv(req(value.path, "path")?)?,
            status: cv(req(value.status, "status")?)?,
            additions: cv(req(value.additions, "additions")?)?,
            deletions: cv(req(value.deletions, "deletions")?)?,
        })
    }
}

impl TryFrom<crate::usecase::code_dto::DiffRangeDto> for wire::DiffRangeDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::DiffRangeDto) -> Result<Self, String> {
        Ok(Self {
            start_line: Some(cv(value.start_line)?),
            end_line: Some(cv(value.end_line)?),
            kind: Some(cv(value.kind)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::DiffRangeKindDto> for wire::DiffRangeKindDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::DiffRangeKindDto) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::code_dto::DiffRangeKindDto::Added => {
                    wire::diff_range_kind_dto::Value::Added as i32
                }
                crate::usecase::code_dto::DiffRangeKindDto::Modified => {
                    wire::diff_range_kind_dto::Value::Modified as i32
                }
                crate::usecase::code_dto::DiffRangeKindDto::Deleted => {
                    wire::diff_range_kind_dto::Value::Deleted as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::DiffTreeNodeDto> for wire::DiffTreeNodeDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::DiffTreeNodeDto) -> Result<Self, String> {
        Ok(Self {
            id: Some(cv(value.id)?),
            name: Some(cv(value.name)?),
            path: Some(cv(value.path)?),
            node_type: Some(cv(value.node_type)?),
            status: value.status.map(cv).transpose()?,
            additions: value.additions.map(cv).transpose()?,
            deletions: value.deletions.map(cv).transpose()?,
            children: Some(cv(value.children)?),
        })
    }
}

impl TryFrom<wire::DiffTreeNodeInput> for crate::adaptor::presenter::code::DiffTreeNodeInput {
    type Error = String;
    fn try_from(value: wire::DiffTreeNodeInput) -> Result<Self, String> {
        Ok(Self {
            id: cv(req(value.id, "id")?)?,
            name: cv(req(value.name, "name")?)?,
            path: cv(req(value.path, "path")?)?,
            node_type: cv(req(value.node_type, "node_type")?)?,
            status: value.status.map(cv).transpose()?,
            additions: value.additions.map(cv).transpose()?,
            deletions: value.deletions.map(cv).transpose()?,
            children: cv(req(value.children, "children")?)?,
        })
    }
}

impl TryFrom<crate::usecase::external_editor::dto::EditorInfoDto> for wire::EditorInfoDto {
    type Error = String;
    fn try_from(
        value: crate::usecase::external_editor::dto::EditorInfoDto,
    ) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            path: Some(cv(value.path)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::ExecutionOriginView>
    for wire::ExecutionOriginView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::ExecutionOriginView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::presenter::workflow_wire::ExecutionOriginView::DesktopUi => {
                    wire::execution_origin_view::Value::DesktopUi as i32
                }
                crate::adaptor::presenter::workflow_wire::ExecutionOriginView::Cli => {
                    wire::execution_origin_view::Value::Cli as i32
                }
                crate::adaptor::presenter::workflow_wire::ExecutionOriginView::Agent => {
                    wire::execution_origin_view::Value::Agent as i32
                }
                crate::adaptor::presenter::workflow_wire::ExecutionOriginView::Api => {
                    wire::execution_origin_view::Value::Api as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::ExecutionParentRefView>
    for wire::ExecutionParentRefView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::ExecutionParentRefView,
    ) -> Result<Self, String> {
        Ok(Self {
            parent_id: Some(cv(value.parent_id)?),
            item_index: value.item_index.map(cv).transpose()?,
            child_index: value.child_index.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::ExecutionStatusView>
    for wire::ExecutionStatusView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::ExecutionStatusView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::presenter::workflow_wire::ExecutionStatusView::Running => {
                    wire::execution_status_view::Value::Running as i32
                }
                crate::adaptor::presenter::workflow_wire::ExecutionStatusView::Completed => {
                    wire::execution_status_view::Value::Completed as i32
                }
                crate::adaptor::presenter::workflow_wire::ExecutionStatusView::Aborted => {
                    wire::execution_status_view::Value::Aborted as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::FacetRefsDto> for wire::FacetRefsDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::FacetRefsDto) -> Result<Self, String> {
        Ok(Self {
            policy: value.policy.map(cv).transpose()?,
            knowledge: Some(cv(value.knowledge)?),
            instruction: value.instruction.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::FacetSummaryDto> for wire::FacetSummaryDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::FacetSummaryDto) -> Result<Self, String> {
        Ok(Self {
            key: Some(cv(value.key)?),
            kind: Some(cv(value.kind)?),
            description: Some(cv(value.description)?),
            builtin: Some(cv(value.builtin)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::FacetUsageEntry> for wire::FacetUsageEntry {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::FacetUsageEntry,
    ) -> Result<Self, String> {
        Ok(Self {
            workflow_name: Some(cv(value.workflow_name)?),
            node_name: Some(cv(value.node_name)?),
            slot: Some(cv(value.slot)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::FanoutSpecDto> for wire::FanoutSpecDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::FanoutSpecDto) -> Result<Self, String> {
        Ok(Self {
            children: Some(cv(value.children)?),
            items: value.items.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::FanoutView> for wire::FanoutView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::FanoutView,
    ) -> Result<Self, String> {
        Ok(Self {
            parent: Some(cv(value.parent)?),
            children: Some(cv(value.children)?),
            artifact: value.artifact.map(cv).transpose()?,
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

impl TryFrom<crate::usecase::code_dto::FileNavigationResultDto> for wire::FileNavigationResultDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::FileNavigationResultDto) -> Result<Self, String> {
        Ok(Self {
            current_index: Some(cv(value.current_index)?),
            total: Some(cv(value.total)?),
            prev_file: value.prev_file.map(cv).transpose()?,
            next_file: value.next_file.map(cv).transpose()?,
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

impl TryFrom<crate::adaptor::presenter::terminal::GetOrSpawnTerminalV1>
    for wire::GetOrSpawnTerminalV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::terminal::GetOrSpawnTerminalV1,
    ) -> Result<Self, String> {
        Ok(Self {
            session_key: Some(cv(value.session_key)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::HiddenRangeDto> for wire::HiddenRangeDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::HiddenRangeDto) -> Result<Self, String> {
        Ok(Self {
            start_line: Some(cv(value.start_line)?),
            end_line: Some(cv(value.end_line)?),
            hidden_count: Some(cv(value.hidden_count)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::HunkDto> for wire::HunkDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::HunkDto) -> Result<Self, String> {
        Ok(Self {
            index: Some(cv(value.index)?),
            hunk_id: Some(cv(value.hunk_id)?),
            old_start: Some(cv(value.old_start)?),
            old_lines: Some(cv(value.old_lines)?),
            new_start: Some(cv(value.new_start)?),
            new_lines: Some(cv(value.new_lines)?),
            lines: Some(cv(value.lines)?),
        })
    }
}

impl TryFrom<wire::HunkInput> for crate::adaptor::presenter::code::HunkInput {
    type Error = String;
    fn try_from(value: wire::HunkInput) -> Result<Self, String> {
        Ok(Self {
            index: cv(req(value.index, "index")?)?,
            hunk_id: value.hunk_id.map(cv).transpose()?.unwrap_or_default(),
            old_start: cv(req(value.old_start, "oldStart")?)?,
            old_lines: cv(req(value.old_lines, "oldLines")?)?,
            new_start: cv(req(value.new_start, "newStart")?)?,
            new_lines: cv(req(value.new_lines, "newLines")?)?,
            lines: cv(req(value.lines, "lines")?)?,
        })
    }
}

impl TryFrom<crate::usecase::code_dto::InlineChunkDto> for wire::InlineChunkDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::InlineChunkDto) -> Result<Self, String> {
        Ok(Self {
            content: Some(cv(value.content)?),
            kind: Some(cv(value.kind)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::InlineChunkKindDto> for wire::InlineChunkKindDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::InlineChunkKindDto) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::code_dto::InlineChunkKindDto::Unchanged => {
                    wire::inline_chunk_kind_dto::Value::Unchanged as i32
                }
                crate::usecase::code_dto::InlineChunkKindDto::Added => {
                    wire::inline_chunk_kind_dto::Value::Added as i32
                }
                crate::usecase::code_dto::InlineChunkKindDto::Removed => {
                    wire::inline_chunk_kind_dto::Value::Removed as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::InputParamDto> for wire::InputParamDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::InputParamDto) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            contract: value.contract.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::git_host::dto::IssueInfoDto> for wire::IssueInfoDto {
    type Error = String;
    fn try_from(value: crate::usecase::git_host::dto::IssueInfoDto) -> Result<Self, String> {
        Ok(Self {
            number: Some(cv(value.number)?),
            default_branch_name: Some(cv(value.default_branch_name)?),
            title: Some(cv(value.title)?),
            state: Some(cv(value.state)?),
            url: Some(cv(value.url)?),
            author: Some(cv(value.author)?),
            created_at: Some(cv(value.created_at)?),
            updated_at: Some(cv(value.updated_at)?),
            labels: Some(cv(value.labels)?),
            assignees: Some(cv(value.assignees)?),
            body: Some(cv(value.body)?),
            milestone: value.milestone.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::git_host::dto::IssueLabelDto> for wire::IssueLabelDto {
    type Error = String;
    fn try_from(value: crate::usecase::git_host::dto::IssueLabelDto) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            color: Some(cv(value.color)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::ItemsSourceDto> for wire::ItemsSourceDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::ItemsSourceDto) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::usecase::workflow::dto::ItemsSourceDto::Literal(value) => {
                    wire::items_source_dto::Variant::Literal(cv(value)?)
                }
                crate::usecase::workflow::dto::ItemsSourceDto::ArtifactField(value) => {
                    wire::items_source_dto::Variant::ArtifactField(cv(value)?)
                }
            }),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::notion::LabelPropertyView> for wire::LabelPropertyView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::notion::LabelPropertyView,
    ) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            property_type: Some(cv(value.property_type)?),
        })
    }
}

impl TryFrom<wire::LabelPropertyView> for crate::adaptor::presenter::notion::LabelPropertyView {
    type Error = String;
    fn try_from(value: wire::LabelPropertyView) -> Result<Self, String> {
        Ok(Self {
            name: cv(req(value.name, "name")?)?,
            property_type: cv(req(value.property_type, "property_type")?)?,
        })
    }
}

impl TryFrom<wire::MarkdownDiffSideInput>
    for crate::adaptor::presenter::code::MarkdownDiffSideInput
{
    type Error = String;
    fn try_from(value: wire::MarkdownDiffSideInput) -> Result<Self, String> {
        Ok(
            match wire::markdown_diff_side_input::Value::try_from(req(value.value, "value")?)
                .map_err(|_| "Invalid MarkdownDiffSideInput")?
            {
                wire::markdown_diff_side_input::Value::Modified => Self::Modified,
                wire::markdown_diff_side_input::Value::Original => Self::Original,
            },
        )
    }
}

impl TryFrom<crate::usecase::git_host::dto::MilestoneDto> for wire::MilestoneDto {
    type Error = String;
    fn try_from(value: crate::usecase::git_host::dto::MilestoneDto) -> Result<Self, String> {
        Ok(Self {
            title: Some(cv(value.title)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::NodeCompletionDto> for wire::NodeCompletionDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::NodeCompletionDto) -> Result<Self, String> {
        Ok(Self {
            require: value.require.map(cv).transpose()?,
            delegate: value.delegate.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::NodeCompletionSignalView>
    for wire::NodeCompletionSignalView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::NodeCompletionSignalView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::presenter::workflow_wire::NodeCompletionSignalView::Submit => {
                    wire::node_completion_signal_view::Value::Submit as i32
                }
                crate::adaptor::presenter::workflow_wire::NodeCompletionSignalView::Stop => {
                    wire::node_completion_signal_view::Value::Stop as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::NodeDefinitionDto> for wire::NodeDefinitionDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::NodeDefinitionDto) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            kind: Some(cv(value.kind)?),
            command: value.command.map(cv).transpose()?,
            session: value.session.map(cv).transpose()?,
            fanout: value.fanout.map(cv).transpose()?,
            sequence: value.sequence.map(cv).transpose()?,
            artifact: value.artifact.map(cv).transpose()?,
            input: Some(cv(value.input)?),
            completion: value.completion.map(cv).transpose()?,
            worktree: value.worktree.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::NodeExecutionStatusView>
    for wire::NodeExecutionStatusView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::NodeExecutionStatusView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::presenter::workflow_wire::NodeExecutionStatusView::Running => {
                    wire::node_execution_status_view::Value::Running as i32
                }
                crate::adaptor::presenter::workflow_wire::NodeExecutionStatusView::WaitingApproval => {
                    wire::node_execution_status_view::Value::WaitingApproval as i32
                }
                crate::adaptor::presenter::workflow_wire::NodeExecutionStatusView::Succeeded => {
                    wire::node_execution_status_view::Value::Succeeded as i32
                }
                crate::adaptor::presenter::workflow_wire::NodeExecutionStatusView::Aborted => {
                    wire::node_execution_status_view::Value::Aborted as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::NodeExecutionView>
    for wire::NodeExecutionView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::NodeExecutionView,
    ) -> Result<Self, String> {
        Ok(Self {
            process_presence: Some(cv(value.process_presence)?),
            worktree: value.worktree.map(cv).transpose()?,
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
            can_resume_session: Some(value.can_resume_session),
            has_artifact: Some(cv(value.has_artifact)?),
            session_id: value.session_id.map(cv).transpose()?,
            display_command: value.display_command.map(cv).transpose()?,
            result_summary: value.result_summary.map(cv).transpose()?,
            artifact: value.artifact.map(cv).transpose()?,
            token_usage: value.token_usage.map(cv).transpose()?,
            parent: value.parent.map(cv).transpose()?,
            started_at: Some(cv(value.started_at)?),
            completed_at: value.completed_at.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::NodeKindDto> for wire::NodeKindDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::NodeKindDto) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::workflow::dto::NodeKindDto::Session => {
                    wire::node_kind_dto::Value::Session as i32
                }
                crate::usecase::workflow::dto::NodeKindDto::Command => {
                    wire::node_kind_dto::Value::Command as i32
                }
                crate::usecase::workflow::dto::NodeKindDto::Fanout => {
                    wire::node_kind_dto::Value::Fanout as i32
                }
                crate::usecase::workflow::dto::NodeKindDto::Sequence => {
                    wire::node_kind_dto::Value::Sequence as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::NodeKindView> for wire::NodeKindView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::NodeKindView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::presenter::workflow_wire::NodeKindView::Command => {
                    wire::node_kind_view::Value::Command as i32
                }
                crate::adaptor::presenter::workflow_wire::NodeKindView::Session => {
                    wire::node_kind_view::Value::Session as i32
                }
                crate::adaptor::presenter::workflow_wire::NodeKindView::Fanout => {
                    wire::node_kind_view::Value::Fanout as i32
                }
                crate::adaptor::presenter::workflow_wire::NodeKindView::Sequence => {
                    wire::node_kind_view::Value::Sequence as i32
                }
            }),
        })
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

impl TryFrom<crate::adaptor::presenter::notion::NotionConfigStatusView>
    for wire::NotionConfigStatusView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::notion::NotionConfigStatusView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::presenter::notion::NotionConfigStatusView::NotConfigured => {
                    wire::notion_config_status_view::Value::NotConfigured as i32
                }
                crate::adaptor::presenter::notion::NotionConfigStatusView::Configured => {
                    wire::notion_config_status_view::Value::Configured as i32
                }
                crate::adaptor::presenter::notion::NotionConfigStatusView::InvalidToken => {
                    wire::notion_config_status_view::Value::InvalidToken as i32
                }
                crate::adaptor::presenter::notion::NotionConfigStatusView::InvalidDatabase => {
                    wire::notion_config_status_view::Value::InvalidDatabase as i32
                }
                crate::adaptor::presenter::notion::NotionConfigStatusView::NetworkError => {
                    wire::notion_config_status_view::Value::NetworkError as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::notion::NotionLabelOptionView>
    for wire::NotionLabelOptionView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::notion::NotionLabelOptionView,
    ) -> Result<Self, String> {
        Ok(Self {
            property_name: Some(cv(value.property_name)?),
            property_type: Some(cv(value.property_type)?),
            options: Some(cv(value.options)?),
            option_ids: Some(cv(value.option_ids)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::notion::NotionPropertyInfoView>
    for wire::NotionPropertyInfoView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::notion::NotionPropertyInfoView,
    ) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            property_type: Some(cv(value.property_type)?),
            options: Some(cv(value.options)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::notion::NotionRepoConfigView>
    for wire::NotionRepoConfigView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::notion::NotionRepoConfigView,
    ) -> Result<Self, String> {
        Ok(Self {
            api_token: Some(cv(value.api_token)?),
            database_id: Some(cv(value.database_id)?),
            property_mapping: Some(cv(value.property_mapping)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::notion::NotionTaskPageView> for wire::NotionTaskPageView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::notion::NotionTaskPageView,
    ) -> Result<Self, String> {
        Ok(Self {
            tasks: Some(cv(value.tasks)?),
            has_more: Some(cv(value.has_more)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::notion::NotionTaskView> for wire::NotionTaskView {
    type Error = String;
    fn try_from(value: crate::adaptor::presenter::notion::NotionTaskView) -> Result<Self, String> {
        Ok(Self {
            id: Some(cv(value.id)?),
            title: Some(cv(value.title)?),
            url: Some(cv(value.url)?),
            labels: Some(cv(value.labels)?),
            branch_name: Some(cv(value.branch_name)?),
            created_at: Some(cv(value.created_at)?),
            last_edited_at: Some(cv(value.last_edited_at)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::notion::NotionValidationResultView>
    for wire::NotionValidationResultView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::notion::NotionValidationResultView,
    ) -> Result<Self, String> {
        Ok(Self {
            status: Some(cv(value.status)?),
            properties: Some(cv(value.properties)?),
        })
    }
}

impl TryFrom<crate::usecase::git_host::dto::PrAuthorDto> for wire::PrAuthorDto {
    type Error = String;
    fn try_from(value: crate::usecase::git_host::dto::PrAuthorDto) -> Result<Self, String> {
        Ok(Self {
            login: Some(cv(value.login)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::PredicateDto> for wire::PredicateDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::PredicateDto) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::usecase::workflow::dto::PredicateDto::Ref(value) => {
                    wire::predicate_dto::Variant::Ref(cv(value)?)
                }
                crate::usecase::workflow::dto::PredicateDto::And { and } => {
                    wire::predicate_dto::Variant::And(wire::PredicateDtoAnd {
                        and: Some(cv(and)?),
                    })
                }
                crate::usecase::workflow::dto::PredicateDto::Or { or } => {
                    wire::predicate_dto::Variant::Or(wire::PredicateDtoOr { or: Some(cv(or)?) })
                }
            }),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::notion::PropertyMappingView> for wire::PropertyMappingView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::notion::PropertyMappingView,
    ) -> Result<Self, String> {
        Ok(Self {
            title: Some(cv(value.title)?),
            labels: Some(cv(value.labels)?),
            branch_name: Some(cv(value.branch_name)?),
            branch_prefix: Some(cv(value.branch_prefix)?),
        })
    }
}

impl TryFrom<wire::PropertyMappingView> for crate::adaptor::presenter::notion::PropertyMappingView {
    type Error = String;
    fn try_from(value: wire::PropertyMappingView) -> Result<Self, String> {
        Ok(Self {
            title: value.title.map(cv).transpose()?.unwrap_or_default(),
            labels: value.labels.map(cv).transpose()?.unwrap_or_default(),
            branch_name: value.branch_name.map(cv).transpose()?.unwrap_or_default(),
            branch_prefix: value.branch_prefix.map(cv).transpose()?.unwrap_or_default(),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::agent_session::ProviderAvailabilityItemResponse>
    for wire::ProviderAvailabilityItemResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::agent_session::ProviderAvailabilityItemResponse,
    ) -> Result<Self, String> {
        Ok(Self {
            provider: Some(cv(value.provider)?),
            display_name: Some(cv(value.display_name)?),
            default_executable: Some(cv(value.default_executable)?),
            configured_executable: value.configured_executable.map(cv).transpose()?,
            configuration_revision: Some(value.configuration_revision),
            effective_executable: Some(cv(value.effective_executable)?),
            available: Some(cv(value.available)?),
            resolved_executable: value.resolved_executable.map(cv).transpose()?,
            unavailable_reason: value.unavailable_reason.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::presenter::agent_session::ProviderAvailabilitySnapshotResponse>
    for wire::ProviderAvailabilitySnapshotResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::agent_session::ProviderAvailabilitySnapshotResponse,
    ) -> Result<Self, String> {
        Ok(Self {
            providers: Some(cv(value.providers)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::agent_session::ProviderHookHealthProviderResponse>
    for wire::ProviderHookHealthProviderResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::agent_session::ProviderHookHealthProviderResponse,
    ) -> Result<Self, String> {
        Ok(Self { value: Some(match value { crate::adaptor::presenter::agent_session::ProviderHookHealthProviderResponse::Claude => wire::provider_hook_health_provider_response::Value::Claude as i32, crate::adaptor::presenter::agent_session::ProviderHookHealthProviderResponse::Codex => wire::provider_hook_health_provider_response::Value::Codex as i32 }) })
    }
}

impl TryFrom<crate::adaptor::presenter::agent_session::ProviderHookHealthWarningResponse>
    for wire::ProviderHookHealthWarningResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::agent_session::ProviderHookHealthWarningResponse,
    ) -> Result<Self, String> {
        Ok(Self {
            provider: Some(cv(value.provider)?),
            launch_id: Some(cv(value.launch_id)?),
            reason: Some(cv(value.reason)?),
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewActorKind> for wire::ReviewActorKindWireDto {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewActorKind) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::domain::comment::ReviewActorKind::Human => {
                    wire::review_actor_kind_wire_dto::Value::Human as i32
                }
                crate::domain::comment::ReviewActorKind::Agent => {
                    wire::review_actor_kind_wire_dto::Value::Agent as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewActorDto> for wire::ReviewActorWireDto {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewActorDto) -> Result<Self, String> {
        Ok(Self {
            kind: Some(cv(value.kind)?),
            backend_id: value.backend_id.map(cv).transpose()?,
            model: value.model.map(cv).transpose()?,
            display_name: Some(cv(value.display_name)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::ReviewBinaryDto> for wire::ReviewBinaryDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ReviewBinaryDto) -> Result<Self, String> {
        Ok(Self {
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            file_id: Some(cv(value.file_id)?),
            path: Some(cv(value.path)?),
            original_size: value.original_size.map(cv).transpose()?,
            modified_size: value.modified_size.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewComment> for wire::ReviewCommentDto {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewComment) -> Result<Self, String> {
        Ok(Self {
            id: Some(cv(value.id)?),
            thread_id: Some(cv(value.thread_id)?),
            author: Some(cv(value.author)?),
            content: Some(cv(value.content)?),
            created_at: Some(cv(value.created_at)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::ReviewFallbackDto> for wire::ReviewFallbackDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ReviewFallbackDto) -> Result<Self, String> {
        Ok(Self {
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            file_id: Some(cv(value.file_id)?),
            path: Some(cv(value.path)?),
            reason: Some(cv(value.reason)?),
            total_lines: value.total_lines.map(cv).transpose()?,
            size_bytes: value.size_bytes.map(cv).transpose()?,
            hunk_count: value.hunk_count.map(cv).transpose()?,
            limited: Some(cv(value.limited)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::ReviewFileEntryDto> for wire::ReviewFileEntryDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ReviewFileEntryDto) -> Result<Self, String> {
        Ok(Self {
            file_id: Some(cv(value.file_id)?),
            path: Some(cv(value.path)?),
            index_status: Some(cv(value.index_status)?),
            worktree_status: Some(cv(value.worktree_status)?),
            additions: Some(cv(value.additions)?),
            deletions: Some(cv(value.deletions)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::ReviewFileViewDto> for wire::ReviewFileViewDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ReviewFileViewDto) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::usecase::code_dto::ReviewFileViewDto::TextDiff(value) => {
                    wire::review_file_view_dto::Variant::TextDiff(cv(value)?)
                }
                crate::usecase::code_dto::ReviewFileViewDto::Image(value) => {
                    wire::review_file_view_dto::Variant::Image(cv(value)?)
                }
                crate::usecase::code_dto::ReviewFileViewDto::Binary(value) => {
                    wire::review_file_view_dto::Variant::Binary(cv(value)?)
                }
                crate::usecase::code_dto::ReviewFileViewDto::Fallback(value) => {
                    wire::review_file_view_dto::Variant::Fallback(cv(value)?)
                }
            }),
        })
    }
}

impl TryFrom<wire::ReviewGroupActionInput>
    for crate::adaptor::presenter::code::ReviewGroupActionInput
{
    type Error = String;
    fn try_from(value: wire::ReviewGroupActionInput) -> Result<Self, String> {
        Ok(Self {
            worktree_path: cv(req(value.worktree_path, "worktreePath")?)?,
            path: cv(req(value.path, "path")?)?,
            section: cv(req(value.section, "section")?)?,
            base: cv(req(value.base, "base")?)?,
            group_id: cv(req(value.group_id, "groupId")?)?,
        })
    }
}

impl TryFrom<crate::usecase::code_dto::ReviewImageDto> for wire::ReviewImageDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ReviewImageDto) -> Result<Self, String> {
        Ok(Self {
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            file_id: Some(cv(value.file_id)?),
            path: Some(cv(value.path)?),
            original_url: value.original_url.map(cv).transpose()?,
            modified_url: value.modified_url.map(cv).transpose()?,
            mime: Some(cv(value.mime)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::ReviewLimitReasonDto> for wire::ReviewLimitReasonDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ReviewLimitReasonDto) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::code_dto::ReviewLimitReasonDto::FileSize => {
                    wire::review_limit_reason_dto::Value::FileSize as i32
                }
                crate::usecase::code_dto::ReviewLimitReasonDto::LineCount => {
                    wire::review_limit_reason_dto::Value::LineCount as i32
                }
                crate::usecase::code_dto::ReviewLimitReasonDto::HunkCount => {
                    wire::review_limit_reason_dto::Value::HunkCount as i32
                }
                crate::usecase::code_dto::ReviewLimitReasonDto::Tokenization => {
                    wire::review_limit_reason_dto::Value::Tokenization as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewResolveInfo> for wire::ReviewResolveInfoDto {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewResolveInfo) -> Result<Self, String> {
        Ok(Self {
            actor: Some(cv(value.actor)?),
            outcome: Some(cv(value.outcome)?),
            summary: Some(cv(value.summary)?),
            resolved_at: Some(cv(value.resolved_at)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::ReviewSnapshotDto> for wire::ReviewSnapshotDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ReviewSnapshotDto) -> Result<Self, String> {
        Ok(Self {
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            loading: Some(cv(value.loading)?),
            base: Some(cv(value.base)?),
            files: Some(cv(value.files)?),
            staged_files: Some(cv(value.staged_files)?),
            changed_files: Some(cv(value.changed_files)?),
            diff_stats: Some(cv(value.diff_stats)?),
            tree: Some(cv(value.tree)?),
            staged_tree: Some(cv(value.staged_tree)?),
            changes_tree: Some(cv(value.changes_tree)?),
            staged_file_count: Some(cv(value.staged_file_count)?),
            changes_file_count: Some(cv(value.changes_file_count)?),
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewTarget> for wire::ReviewTargetWireDto {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewTarget) -> Result<Self, String> {
        Ok(Self {
            file_path: value.file_path.map(cv).transpose()?,
            line_number: value.line_number.map(cv).transpose()?,
            end_line: value.end_line.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::code_dto::ReviewTextDiffDto> for wire::ReviewTextDiffDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ReviewTextDiffDto) -> Result<Self, String> {
        Ok(Self {
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            file_id: Some(cv(value.file_id)?),
            path: Some(cv(value.path)?),
            original: Some(cv(value.original)?),
            modified: Some(cv(value.modified)?),
            source: Some(cv(value.source)?),
            hunks: Some(cv(value.hunks)?),
            change_groups: Some(cv(value.change_groups)?),
            limited: Some(cv(value.limited)?),
            total_lines: Some(cv(value.total_lines)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::ReviewTextSource> for wire::ReviewTextSource {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ReviewTextSource) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::code_dto::ReviewTextSource::Diff => {
                    wire::review_text_source::Value::Diff as i32
                }
                crate::usecase::code_dto::ReviewTextSource::Added => {
                    wire::review_text_source::Value::Added as i32
                }
                crate::usecase::code_dto::ReviewTextSource::Deleted => {
                    wire::review_text_source::Value::Deleted as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewThread> for wire::ReviewThreadDto {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewThread) -> Result<Self, String> {
        Ok(Self {
            id: Some(cv(value.id)?),
            worktree_name: Some(cv(value.worktree_name)?),
            author: Some(cv(value.author)?),
            target: Some(cv(value.target)?),
            state: Some(cv(value.state)?),
            comments: Some(cv(value.comments)?),
            resolve: value.resolve.map(cv).transpose()?,
            created_at: Some(cv(value.created_at)?),
            updated_at: Some(cv(value.updated_at)?),
            version: Some(cv(value.version)?),
            can_resolve: Some(cv(value.can_resolve)?),
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewThreadState> for wire::ReviewThreadStateDto {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewThreadState) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::domain::comment::ReviewThreadState::Open => {
                    wire::review_thread_state_dto::Value::Open as i32
                }
                crate::domain::comment::ReviewThreadState::Resolved => {
                    wire::review_thread_state_dto::Value::Resolved as i32
                }
            }),
        })
    }
}

impl TryFrom<wire::ReviewThreadStateDto> for crate::domain::comment::ReviewThreadState {
    type Error = String;
    fn try_from(value: wire::ReviewThreadStateDto) -> Result<Self, String> {
        Ok(
            match wire::review_thread_state_dto::Value::try_from(req(value.value, "value")?)
                .map_err(|_| "Invalid ReviewThreadStateDto")?
            {
                wire::review_thread_state_dto::Value::Open => Self::Open,
                wire::review_thread_state_dto::Value::Resolved => Self::Resolved,
            },
        )
    }
}

impl TryFrom<crate::usecase::workflow::dto::RuleDto> for wire::RuleDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::RuleDto) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::usecase::workflow::dto::RuleDto::When { on, then, next } => {
                    wire::rule_dto::Variant::When(wire::RuleDtoWhen {
                        on: Some(cv(on)?),
                        then: Some(cv(then)?),
                        next: Some(cv(next)?),
                    })
                }
                crate::usecase::workflow::dto::RuleDto::Switch { on, cases, next } => {
                    wire::rule_dto::Variant::Switch(wire::RuleDtoSwitch {
                        on: Some(cv(on)?),
                        cases: Some(cv(cases)?),
                        next: next.map(cv).transpose()?,
                    })
                }
                crate::usecase::workflow::dto::RuleDto::LoopGuard {
                    max_iterations,
                    on_exhausted,
                } => wire::rule_dto::Variant::LoopGuard(wire::RuleDtoLoopGuard {
                    max_iterations: Some(cv(max_iterations)?),
                    on_exhausted: Some(cv(on_exhausted)?),
                }),
                crate::usecase::workflow::dto::RuleDto::Next { next } => {
                    wire::rule_dto::Variant::Next(wire::RuleDtoNext {
                        next: Some(cv(next)?),
                    })
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::SequenceSpecDto> for wire::SequenceSpecDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::SequenceSpecDto) -> Result<Self, String> {
        Ok(Self {
            entry: value.entry.map(cv).transpose()?,
            children: Some(cv(value.children)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::SessionDelegateDto> for wire::SessionDelegateDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::SessionDelegateDto) -> Result<Self, String> {
        Ok(Self {
            child: Some(cv(value.child)?),
            inputs: Some(cv(value.inputs)?),
            when: Some(cv(value.when)?),
            max_iterations: Some(cv(value.max_iterations)?),
        })
    }
}

impl TryFrom<crate::usecase::provider_dto::AgentSessionProviderDto> for wire::SessionProviderDto {
    type Error = String;
    fn try_from(
        value: crate::usecase::provider_dto::AgentSessionProviderDto,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::provider_dto::AgentSessionProviderDto::Claude => {
                    wire::session_provider_dto::Value::Claude as i32
                }
                crate::usecase::provider_dto::AgentSessionProviderDto::Codex => {
                    wire::session_provider_dto::Value::Codex as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::SessionSpecDto> for wire::SessionSpecDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::SessionSpecDto) -> Result<Self, String> {
        Ok(Self {
            provider: Some(cv(value.provider)?),
            model: value.model.map(cv).transpose()?,
            permission: value.permission.map(cv).transpose()?,
            facets: Some(cv(value.facets)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::Severity> for wire::Severity {
    type Error = String;
    fn try_from(value: crate::adaptor::presenter::workflow_wire::Severity) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::presenter::workflow_wire::Severity::Error => {
                    wire::severity::Value::Error as i32
                }
                crate::adaptor::presenter::workflow_wire::Severity::Info => {
                    wire::severity::Value::Info as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::SplitRowDto> for wire::SplitRowDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::SplitRowDto) -> Result<Self, String> {
        Ok(Self {
            left: value.left.map(cv).transpose()?,
            right: value.right.map(cv).transpose()?,
            kind: Some(cv(value.kind)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::SplitRowKindDto> for wire::SplitRowKindDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::SplitRowKindDto) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::code_dto::SplitRowKindDto::Unchanged => {
                    wire::split_row_kind_dto::Value::Unchanged as i32
                }
                crate::usecase::code_dto::SplitRowKindDto::Added => {
                    wire::split_row_kind_dto::Value::Added as i32
                }
                crate::usecase::code_dto::SplitRowKindDto::Removed => {
                    wire::split_row_kind_dto::Value::Removed as i32
                }
                crate::usecase::code_dto::SplitRowKindDto::Modified => {
                    wire::split_row_kind_dto::Value::Modified as i32
                }
            }),
        })
    }
}

impl TryFrom<wire::TerminalSurfaceOwnerV1>
    for crate::adaptor::presenter::terminal::TerminalSurfaceOwnerV1
{
    type Error = String;
    fn try_from(value: wire::TerminalSurfaceOwnerV1) -> Result<Self, String> {
        Ok(match req(value.variant, "variant")? {
            wire::terminal_surface_owner_v1::Variant::Workspace(value) => {
                crate::adaptor::presenter::terminal::TerminalSurfaceOwnerV1::Workspace {
                    workspace_path: cv(req(value.workspace_path, "workspacePath")?)?,
                }
            }
            wire::terminal_surface_owner_v1::Variant::Session(value) => {
                crate::adaptor::presenter::terminal::TerminalSurfaceOwnerV1::Session {
                    workspace_path: cv(req(value.workspace_path, "workspacePath")?)?,
                    session_id: cv(req(value.session_id, "sessionId")?)?,
                }
            }
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::TokenUsageView> for wire::TokenUsageView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::TokenUsageView,
    ) -> Result<Self, String> {
        Ok(Self {
            input_tokens: Some(cv(value.input_tokens)?),
            output_tokens: Some(cv(value.output_tokens)?),
        })
    }
}

impl TryFrom<crate::usecase::code_dto::VisibleBlockDto> for wire::VisibleBlockDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::VisibleBlockDto) -> Result<Self, String> {
        Ok(Self {
            start_line: Some(cv(value.start_line)?),
            end_line: Some(cv(value.end_line)?),
            content: Some(cv(value.content)?),
            deleted_content: value.deleted_content.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::WorkflowDto> for wire::WorkflowDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::WorkflowDto) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            description: Some(cv(value.description)?),
            builtin: Some(cv(value.builtin)?),
            source_format: Some(cv(value.source_format)?),
            schemas: Some(cv(value.schemas)?),
            nodes: Some(cv(value.nodes)?),
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::WorkflowExecutionView>
    for wire::WorkflowExecutionView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::WorkflowExecutionView,
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
            total_token_usage: Some(cv(value.total_token_usage)?),
            node_executions: Some(cv(value.node_executions)?),
            artifacts: Some(cv(value.artifacts)?),
            fanouts: Some(cv(value.fanouts)?),
            approval_target: value.approval_target.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::presenter::workflow_wire::WorkflowGetOutputResponse>
    for wire::WorkflowOutputView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::presenter::workflow_wire::WorkflowGetOutputResponse,
    ) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::adaptor::presenter::workflow_wire::WorkflowGetOutputResponse::Submitted {
                    contract,
                    structured_output,
                    submitted_at,
                    request_id,
                    timestamp,
                } => wire::workflow_output_view::Variant::Submitted(
                    wire::WorkflowOutputViewSubmitted {
                        contract: contract.map(cv).transpose()?,
                        structured_output: Some(cv(structured_output)?),
                        submitted_at: submitted_at.map(cv).transpose()?,
                        request_id: request_id.map(cv).transpose()?,
                        timestamp: Some(cv(timestamp)?),
                    },
                ),
                crate::adaptor::presenter::workflow_wire::WorkflowGetOutputResponse::NotSubmitted => {
                    wire::workflow_output_view::Variant::NotSubmitted(wire::Unit {})
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::app_config::query_service::WorkflowConfigDto>
    for wire::WorkflowSection
{
    type Error = String;
    fn try_from(
        value: crate::usecase::app_config::query_service::WorkflowConfigDto,
    ) -> Result<Self, String> {
        Ok(Self {
            approval_auto_approve: Some(cv(value.approval_auto_approve)?),
        })
    }
}

impl TryFrom<wire::WorkflowSection> for crate::usecase::app_config::WorkflowConfigInput {
    type Error = String;
    fn try_from(value: wire::WorkflowSection) -> Result<Self, String> {
        Ok(Self {
            approval_auto_approve: value
                .approval_auto_approve
                .map(cv)
                .transpose()?
                .unwrap_or_default(),
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::WorkflowSourceFormatDto>
    for wire::WorkflowSourceFormat
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::dto::WorkflowSourceFormatDto,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::workflow::dto::WorkflowSourceFormatDto::Yaml => {
                    wire::workflow_source_format::Value::Yaml as i32
                }
                crate::usecase::workflow::dto::WorkflowSourceFormatDto::Lua => {
                    wire::workflow_source_format::Value::Lua as i32
                }
            }),
        })
    }
}

impl TryFrom<wire::WorkflowSubmitArtifactInput>
    for crate::adaptor::presenter::workflow_wire::WorkflowSubmitArtifactInput
{
    type Error = String;
    fn try_from(value: wire::WorkflowSubmitArtifactInput) -> Result<Self, String> {
        Ok(Self {
            contract: cv(req(value.contract, "contract")?)?,
            value: cv(req(value.value, "value")?)?,
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::WorkflowSummaryDto> for wire::WorkflowSummaryDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::WorkflowSummaryDto) -> Result<Self, String> {
        Ok(Self {
            read_error: value
                .failure
                .as_ref()
                .map(|failure| format!("Workflow read failed: {failure}")),
            name: Some(cv(value.name)?),
            description: Some(cv(value.description)?),
            builtin: Some(cv(value.builtin)?),
            is_running: Some(cv(value.is_running)?),
            source_format: Some(cv(value.source_format)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceCommandNodeContentDto>
    for wire::WorkspaceCommandNodeContentDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::WorkspaceCommandNodeContentDto,
    ) -> Result<Self, String> {
        Ok(Self {
            display_command: value.display_command.map(cv).transpose()?,
            result: value.result.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceCommandResultDto>
    for wire::WorkspaceCommandResultDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::WorkspaceCommandResultDto,
    ) -> Result<Self, String> {
        Ok(Self {
            exit_code: Some(cv(value.exit_code)?),
            duration: Some(cv(value.duration)?),
            stdout: Some(cv(value.stdout)?),
            stderr: Some(cv(value.stderr)?),
        })
    }
}

impl TryFrom<crate::usecase::workspace_state::dto::WorkspaceLayoutStateDto>
    for wire::WorkspaceLayoutStateDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workspace_state::dto::WorkspaceLayoutStateDto,
    ) -> Result<Self, String> {
        Ok(Self {
            center_tab: Some(cv(value.center_tab)?),
            active_view: Some(cv(value.active_view)?),
            left_nav_collapsed: Some(cv(value.left_nav_collapsed)?),
            right_collapsed: Some(cv(value.right_collapsed)?),
            right_bottom_collapsed: Some(cv(value.right_bottom_collapsed)?),
            right_bottom_active_tab: value.right_bottom_active_tab.map(cv).transpose()?,
            selected_diff_file: value.selected_diff_file.map(cv).transpose()?,
            review_collapsed: None,
            diff_only_mode: None,
        })
    }
}

impl TryFrom<wire::WorkspaceLayoutStateDto>
    for crate::usecase::workspace_state::dto::WorkspaceLayoutStateDto
{
    type Error = String;
    fn try_from(value: wire::WorkspaceLayoutStateDto) -> Result<Self, String> {
        Ok(Self {
            center_tab: cv(req(value.center_tab, "centerTab")?)?,
            active_view: cv(req(value.active_view, "activeView")?)?,
            left_nav_collapsed: cv(req(value.left_nav_collapsed, "leftNavCollapsed")?)?,
            right_collapsed: cv(req(value.right_collapsed, "rightCollapsed")?)?,
            right_bottom_collapsed: cv(req(value.right_bottom_collapsed, "rightBottomCollapsed")?)?,
            right_bottom_active_tab: value.right_bottom_active_tab.map(cv).transpose()?,
            selected_diff_file: value.selected_diff_file.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceNodeCapabilitiesDto>
    for wire::WorkspaceNodeCapabilitiesDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::WorkspaceNodeCapabilitiesDto,
    ) -> Result<Self, String> {
        Ok(Self {
            can_rename: Some(cv(value.can_rename)?),
            can_approve: Some(cv(value.can_approve)?),
            can_retry: Some(cv(value.can_retry)?),
            can_resume_session: Some(value.can_resume_session),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceNodeContentDto> for wire::WorkspaceNodeContentDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::WorkspaceNodeContentDto) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::usecase::workflow::WorkspaceNodeContentDto::Session(value) => {
                    wire::workspace_node_content_dto::Variant::Session(cv(value)?)
                }
                crate::usecase::workflow::WorkspaceNodeContentDto::Command(value) => {
                    wire::workspace_node_content_dto::Variant::Command(cv(value)?)
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceNodeDetailDto> for wire::WorkspaceNodeDetailDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::WorkspaceNodeDetailDto) -> Result<Self, String> {
        Ok(Self {
            process_presence: Some(cv(value.process_presence)?),
            worktree: value.worktree.map(cv).transpose()?,
            id: Some(cv(value.id)?),
            title: Some(cv(value.title)?),
            status: Some(cv(value.status)?),
            submit_received: Some(cv(value.submit_received)?),
            stop_received: Some(cv(value.stop_received)?),
            waiting_for: value.waiting_for.map(cv).transpose()?,
            has_artifact: Some(cv(value.has_artifact)?),
            error_reason: value.error_reason.map(cv).transpose()?,
            capabilities: Some(cv(value.capabilities)?),
            updated_at: Some(cv(value.updated_at)?),
            content: Some(cv(value.content)?),
        })
    }
}

impl TryFrom<crate::domain::workspace_tree::WorkspaceNodeStatus> for wire::WorkspaceNodeStatus {
    type Error = String;
    fn try_from(value: crate::domain::workspace_tree::WorkspaceNodeStatus) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::domain::workspace_tree::WorkspaceNodeStatus::Running => {
                    wire::workspace_node_status::Value::Running as i32
                }
                crate::domain::workspace_tree::WorkspaceNodeStatus::Waiting => {
                    wire::workspace_node_status::Value::Waiting as i32
                }
                crate::domain::workspace_tree::WorkspaceNodeStatus::Aborted => {
                    wire::workspace_node_status::Value::Aborted as i32
                }
                crate::domain::workspace_tree::WorkspaceNodeStatus::Completed => {
                    wire::workspace_node_status::Value::Completed as i32
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceSessionNodeContentDto>
    for wire::WorkspaceSessionNodeContentDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::WorkspaceSessionNodeContentDto,
    ) -> Result<Self, String> {
        Ok(Self {
            session_id: value.session_id.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::workspace_state::dto::WorkspaceStateDto> for wire::WorkspaceStateDto {
    type Error = String;
    fn try_from(
        value: crate::usecase::workspace_state::dto::WorkspaceStateDto,
    ) -> Result<Self, String> {
        Ok(Self {
            pane_layout: None,
            version: Some(cv(value.version)?),
            tabs: Some(cv(value.tabs)?),
            layout: Some(cv(value.layout)?),
        })
    }
}

impl TryFrom<wire::WorkspaceStateDto> for crate::usecase::workspace_state::dto::WorkspaceStateDto {
    type Error = String;
    fn try_from(value: wire::WorkspaceStateDto) -> Result<Self, String> {
        if value.version != Some(1) {
            return Err("Expected workspace state version 1".into());
        }
        Ok(Self {
            version: cv(req(value.version, "version")?)?,
            tabs: cv(req(value.tabs, "tabs")?)?,
            layout: cv(req(value.layout, "layout")?)?,
        })
    }
}

impl TryFrom<crate::usecase::workspace_state::dto::WorkspaceTabEntryDto>
    for wire::WorkspaceTabEntryDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workspace_state::dto::WorkspaceTabEntryDto,
    ) -> Result<Self, String> {
        Ok(Self {
            path: Some(cv(value.path)?),
            name: Some(cv(value.name)?),
        })
    }
}

impl TryFrom<wire::WorkspaceTabEntryDto>
    for crate::usecase::workspace_state::dto::WorkspaceTabEntryDto
{
    type Error = String;
    fn try_from(value: wire::WorkspaceTabEntryDto) -> Result<Self, String> {
        Ok(Self {
            path: cv(req(value.path, "path")?)?,
            name: cv(req(value.name, "name")?)?,
        })
    }
}

impl TryFrom<crate::usecase::workspace_state::dto::WorkspaceTabsStateDto>
    for wire::WorkspaceTabsStateDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workspace_state::dto::WorkspaceTabsStateDto,
    ) -> Result<Self, String> {
        Ok(Self {
            editors: Some(cv(value.editors)?),
            active_editor_path: value.active_editor_path.map(cv).transpose()?,
        })
    }
}

impl TryFrom<wire::WorkspaceTabsStateDto>
    for crate::usecase::workspace_state::dto::WorkspaceTabsStateDto
{
    type Error = String;
    fn try_from(value: wire::WorkspaceTabsStateDto) -> Result<Self, String> {
        Ok(Self {
            editors: cv(req(value.editors, "editors")?)?,
            active_editor_path: value.active_editor_path.map(cv).transpose()?,
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
        })
    }
}

impl TryFrom<crate::domain::workflow::WorktreeMode> for wire::WorktreeMode {
    type Error = String;
    fn try_from(value: crate::domain::workflow::WorktreeMode) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::domain::workflow::WorktreeMode::Shared => {
                    wire::worktree_mode::Value::Shared as i32
                }
                crate::domain::workflow::WorktreeMode::Isolated => {
                    wire::worktree_mode::Value::Isolated as i32
                }
            }),
        })
    }
}

pub(crate) fn session_selection(
    (agent_session_id, node): (String, crate::domain::workspace_tree::WorkspaceTreeNode),
) -> wire::SessionSelection {
    wire::SessionSelection {
        agent_session_id: Some(agent_session_id),
        node_id: Some(node.id),
    }
}

#[cfg(test)]
#[path = "conversions_test.rs"]
mod conversions_tests;

impl TryFrom<crate::usecase::workflow::diagnostic_dto::DiagnosticReport>
    for wire::DiagnoseWorkflowDirectoryResponse
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::diagnostic_dto::DiagnosticReport,
    ) -> Result<Self, String> {
        Ok(Self {
            report: Some(cv(value)?),
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewThread> for wire::CreateSessionReviewThreadResponse {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewThread) -> Result<Self, String> {
        Ok(Self {
            thread: Some(cv(value)?),
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewThread> for wire::AppendSessionReviewCommentResponse {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewThread) -> Result<Self, String> {
        Ok(Self {
            thread: Some(cv(value)?),
        })
    }
}

impl TryFrom<crate::domain::comment::ReviewThread> for wire::ResolveSessionReviewThreadResponse {
    type Error = String;
    fn try_from(value: crate::domain::comment::ReviewThread) -> Result<Self, String> {
        Ok(Self {
            thread: Some(cv(value)?),
        })
    }
}

impl TryFrom<crate::domain::workspace_state::value_objects::pane_layout::PaneLayout>
    for wire::PaneLayout
{
    type Error = String;
    fn try_from(
        value: crate::domain::workspace_state::value_objects::pane_layout::PaneLayout,
    ) -> Result<Self, String> {
        use crate::domain::workspace_state::value_objects::pane_layout::{
            PaneLayout, PaneTabKind, SplitAxis,
        };
        Ok(Self {
            node: Some(match value {
                PaneLayout::Pane {
                    id,
                    tabs,
                    active_tab,
                } => wire::pane_layout::Node::Pane(wire::Pane {
                    id,
                    active_tab,
                    tabs: tabs
                        .into_iter()
                        .map(|tab| wire::PaneTab {
                            id: tab.id,
                            kind: match tab.kind {
                                PaneTabKind::Terminal => wire::PaneTabKind::Terminal,
                                PaneTabKind::Workflow => wire::PaneTabKind::Workflow,
                                PaneTabKind::File => wire::PaneTabKind::File,
                            } as i32,
                        })
                        .collect(),
                }),
                PaneLayout::Split {
                    id,
                    axis,
                    ratio,
                    first,
                    second,
                } => wire::pane_layout::Node::Split(Box::new(wire::PaneSplit {
                    id,
                    ratio,
                    axis: match axis {
                        SplitAxis::Horizontal => wire::SplitAxis::Horizontal,
                        SplitAxis::Vertical => wire::SplitAxis::Vertical,
                    } as i32,
                    first: Some(Box::new(cv(*first)?)),
                    second: Some(Box::new(cv(*second)?)),
                })),
            }),
        })
    }
}
impl TryFrom<wire::PaneLayout>
    for crate::domain::workspace_state::value_objects::pane_layout::PaneLayout
{
    type Error = String;
    fn try_from(value: wire::PaneLayout) -> Result<Self, String> {
        use crate::domain::workspace_state::value_objects::pane_layout::{
            PaneTab, PaneTabKind, SplitAxis,
        };
        Ok(match req(value.node, "pane node")? {
            wire::pane_layout::Node::Pane(pane) => Self::Pane {
                id: pane.id,
                active_tab: pane.active_tab,
                tabs: pane
                    .tabs
                    .into_iter()
                    .map(|tab| {
                        Ok(PaneTab {
                            id: tab.id,
                            kind: match wire::PaneTabKind::try_from(tab.kind) {
                                Ok(wire::PaneTabKind::Terminal) => PaneTabKind::Terminal,
                                Ok(wire::PaneTabKind::Workflow) => PaneTabKind::Workflow,
                                Ok(wire::PaneTabKind::File) => PaneTabKind::File,
                                _ => return Err("Unknown pane tab kind".to_owned()),
                            },
                        })
                    })
                    .collect::<Result<_, String>>()?,
            },
            wire::pane_layout::Node::Split(split) => Self::Split {
                id: split.id,
                ratio: split.ratio,
                axis: match wire::SplitAxis::try_from(split.axis) {
                    Ok(wire::SplitAxis::Horizontal) => SplitAxis::Horizontal,
                    Ok(wire::SplitAxis::Vertical) => SplitAxis::Vertical,
                    _ => return Err("Unknown split axis".into()),
                },
                first: Box::new(cv(*req(split.first, "first pane")?)?),
                second: Box::new(cv(*req(split.second, "second pane")?)?),
            },
        })
    }
}

impl TryFrom<crate::domain::workspace_state::WorkspaceState> for wire::WorkspaceStateDto {
    type Error = String;
    fn try_from(mut value: crate::domain::workspace_state::WorkspaceState) -> Result<Self, String> {
        let panes = value.panes.take();
        let mut message: Self =
            cv(crate::usecase::workspace_state::dto::WorkspaceStateDto::from(value))?;
        message.pane_layout = panes.map(cv).transpose()?;
        Ok(message)
    }
}
impl TryFrom<wire::WorkspaceStateDto> for crate::domain::workspace_state::WorkspaceState {
    type Error = String;
    fn try_from(mut value: wire::WorkspaceStateDto) -> Result<Self, String> {
        let panes = value.pane_layout.take();
        let legacy: crate::usecase::workspace_state::dto::WorkspaceStateDto = cv(value)?;
        let mut state: Self = legacy.into();
        state.panes = panes.map(cv).transpose()?;
        Ok(state)
    }
}
