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

impl TryFrom<crate::adaptor::protocol::agent_session::AgentSessionArchiveResponse>
    for wire::AgentSessionArchiveResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::agent_session::AgentSessionArchiveResponse,
    ) -> Result<Self, String> {
        Ok(Self { value: Some(match value { crate::adaptor::protocol::agent_session::AgentSessionArchiveResponse::Archived => wire::agent_session_archive_response::Value::Archived as i32, crate::adaptor::protocol::agent_session::AgentSessionArchiveResponse::AlreadyArchived => wire::agent_session_archive_response::Value::AlreadyArchived as i32 }) })
    }
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
            next_after: value.next_after.map(cv).transpose()?,
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

impl TryFrom<crate::adaptor::protocol::agent_session::AgentSessionOpenResponse>
    for wire::AgentSessionOpenResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::agent_session::AgentSessionOpenResponse,
    ) -> Result<Self, String> {
        Ok(Self { value: Some(match value { crate::adaptor::protocol::agent_session::AgentSessionOpenResponse::Attached => wire::agent_session_open_response::Value::Attached as i32, crate::adaptor::protocol::agent_session::AgentSessionOpenResponse::Resumed => wire::agent_session_open_response::Value::Resumed as i32, crate::adaptor::protocol::agent_session::AgentSessionOpenResponse::Restored => wire::agent_session_open_response::Value::Restored as i32, crate::adaptor::protocol::agent_session::AgentSessionOpenResponse::Paused => wire::agent_session_open_response::Value::Paused as i32, crate::adaptor::protocol::agent_session::AgentSessionOpenResponse::Indeterminate => wire::agent_session_open_response::Value::Indeterminate as i32, crate::adaptor::protocol::agent_session::AgentSessionOpenResponse::GarbageCollected => wire::agent_session_open_response::Value::GarbageCollected as i32 }) })
    }
}

impl TryFrom<String> for wire::AgentSessionOpenResponse {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "attached" => wire::agent_session_open_response::Value::Attached as i32,
                "resumed" => wire::agent_session_open_response::Value::Resumed as i32,
                "restored" => wire::agent_session_open_response::Value::Restored as i32,
                "paused" => wire::agent_session_open_response::Value::Paused as i32,
                "indeterminate" => wire::agent_session_open_response::Value::Indeterminate as i32,
                "garbage_collected" => {
                    wire::agent_session_open_response::Value::GarbageCollected as i32
                }
                _ => return Err(format!("Invalid AgentSessionOpenResponse: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::AgentSessionOpenResponse {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
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
            can_resume: Some(cv(value.can_resume)?),
        })
    }
}

impl TryFrom<crate::usecase::agent_session::AgentSessionProviderDto>
    for wire::AgentSessionProviderDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::agent_session::AgentSessionProviderDto,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::agent_session::AgentSessionProviderDto::Claude => {
                    wire::agent_session_provider_dto::Value::Claude as i32
                }
                crate::usecase::agent_session::AgentSessionProviderDto::Codex => {
                    wire::agent_session_provider_dto::Value::Codex as i32
                }
            }),
        })
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

impl TryFrom<crate::adaptor::gateway::app_config::AppSection> for wire::AppSection {
    type Error = String;
    fn try_from(value: crate::adaptor::gateway::app_config::AppSection) -> Result<Self, String> {
        Ok(Self {
            close_to_tray: Some(cv(value.close_to_tray)?),
            auto_launch: Some(cv(value.auto_launch)?),
            start_minimized: Some(cv(value.start_minimized)?),
            last_root_path: Some(cv(value.last_root_path)?),
            last_repo_paths: Some(cv(value.last_repo_paths)?),
            external_editor: Some(cv(value.external_editor)?),
        })
    }
}

impl TryFrom<wire::AppSection> for crate::adaptor::gateway::app_config::AppSection {
    type Error = String;
    fn try_from(value: wire::AppSection) -> Result<Self, String> {
        Ok(Self {
            close_to_tray: value.close_to_tray.map(cv).transpose()?.unwrap_or(true),
            auto_launch: value.auto_launch.map(cv).transpose()?.unwrap_or_default(),
            start_minimized: value
                .start_minimized
                .map(cv)
                .transpose()?
                .unwrap_or_default(),
            last_root_path: value
                .last_root_path
                .map(cv)
                .transpose()?
                .unwrap_or_default(),
            last_repo_paths: value
                .last_repo_paths
                .map(cv)
                .transpose()?
                .unwrap_or_default(),
            external_editor: value
                .external_editor
                .map(cv)
                .transpose()?
                .unwrap_or_default(),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1>
    for wire::ApplicationQuitIntentDtoV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1,
    ) -> Result<Self, String> {
        Ok(Self { variant: Some(match value { crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Exit { code } => wire::application_quit_intent_dto_v1::Variant::Exit(wire::ApplicationQuitIntentDtoV1Exit { code: Some(cv(code)?) }), crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Restart { code } => wire::application_quit_intent_dto_v1::Variant::Restart(wire::ApplicationQuitIntentDtoV1Restart { code: Some(cv(code)?) }) }) })
    }
}

impl TryFrom<wire::ApplicationQuitIntentDtoV1>
    for crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1
{
    type Error = String;
    fn try_from(value: wire::ApplicationQuitIntentDtoV1) -> Result<Self, String> {
        Ok(match req(value.variant,"variant")? { wire::application_quit_intent_dto_v1::Variant::Exit(value) => crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Exit { code: cv(req(value.code, "code")?)? }, wire::application_quit_intent_dto_v1::Variant::Restart(value) => crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitIntentDtoV1::Restart { code: cv(req(value.code, "code")?)? } })
    }
}

impl TryFrom<wire::ApplicationQuitRequestDtoV1>
    for crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitRequestDtoV1
{
    type Error = String;
    fn try_from(value: wire::ApplicationQuitRequestDtoV1) -> Result<Self, String> {
        Ok(Self {
            intent: cv(req(value.intent, "intent")?)?,
        })
    }
}

impl TryFrom<crate::adaptor::protocol::application_lifecycle_v1::ApplicationStartupOutcomeDtoV1>
    for wire::ApplicationStartupOutcomeDtoV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::application_lifecycle_v1::ApplicationStartupOutcomeDtoV1,
    ) -> Result<Self, String> {
        Ok(Self { variant: Some(match value { crate::adaptor::protocol::application_lifecycle_v1::ApplicationStartupOutcomeDtoV1::Ready => wire::application_startup_outcome_dto_v1::Variant::Ready(wire::Unit {}), crate::adaptor::protocol::application_lifecycle_v1::ApplicationStartupOutcomeDtoV1::Failed { kind, safe_description, correlation_id, retry_on_next_launch, actions } => wire::application_startup_outcome_dto_v1::Variant::Failed(wire::ApplicationStartupOutcomeDtoV1Failed { kind: Some(cv(kind)?), safe_description: Some(cv(safe_description)?), correlation_id: Some(cv(correlation_id)?), retry_on_next_launch: Some(cv(retry_on_next_launch)?), actions: Some(cv(actions.to_vec())?) }) }) })
    }
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

impl TryFrom<wire::AuthorScopeDto> for crate::usecase::comment::dto::AuthorScopeDto {
    type Error = String;
    fn try_from(value: wire::AuthorScopeDto) -> Result<Self, String> {
        Ok(
            match wire::author_scope_dto::Value::try_from(req(value.value, "value")?)
                .map_err(|_| "Invalid AuthorScopeDto")?
            {
                wire::author_scope_dto::Value::Mine => Self::Mine,
                wire::author_scope_dto::Value::Other => Self::Other,
            },
        )
    }
}

impl TryFrom<wire::AuthorScopeDto> for String {
    type Error = String;
    fn try_from(value: wire::AuthorScopeDto) -> Result<Self, String> {
        Ok(
            match wire::author_scope_dto::Value::try_from(req(value.value, "value")?)
                .map_err(|_| "Invalid AuthorScopeDto")?
            {
                wire::author_scope_dto::Value::Mine => "mine".to_owned(),
                wire::author_scope_dto::Value::Other => "other".to_owned(),
            },
        )
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

impl TryFrom<crate::usecase::code_dto::BranchDiffSummaryDto> for wire::BranchDiffSummaryDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::BranchDiffSummaryDto) -> Result<Self, String> {
        Ok(Self {
            base_branch: Some(cv(value.base_branch)?),
            changed_files: Some(cv(value.changed_files)?),
            stats: Some(cv(value.stats)?),
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

impl TryFrom<crate::usecase::code_dto::ChangedFileDto> for wire::ChangedFileDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ChangedFileDto) -> Result<Self, String> {
        Ok(Self {
            path: Some(cv(value.path)?),
            old_path: value.old_path.map(cv).transpose()?,
            status: Some(cv(value.status)?),
            binary: Some(cv(value.binary)?),
            stats: Some(cv(value.stats)?),
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

impl TryFrom<crate::adaptor::protocol::workflow::DiagnosticItem> for wire::DiagnosticItem {
    type Error = String;
    fn try_from(value: crate::adaptor::protocol::workflow::DiagnosticItem) -> Result<Self, String> {
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

impl TryFrom<crate::adaptor::protocol::workflow::DiagnosticReport> for wire::DiagnosticReport {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::DiagnosticReport,
    ) -> Result<Self, String> {
        Ok(Self {
            items: Some(cv(value.items)?),
            workflow_summaries: Some(cv(value.workflow_summaries)?),
            facet_summaries: Some(cv(value.facet_summaries)?),
            facet_usage: Some(cv(value.facet_usage)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::DiagnosticSpan> for wire::DiagnosticSpan {
    type Error = String;
    fn try_from(value: crate::adaptor::protocol::workflow::DiagnosticSpan) -> Result<Self, String> {
        Ok(Self {
            source: value.source.map(cv).transpose()?,
            start_line: Some(cv(value.start_line)?),
            start_col: Some(cv(value.start_col)?),
            end_line: Some(cv(value.end_line)?),
            end_col: Some(cv(value.end_col)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::DiagnosticStage> for wire::DiagnosticStage {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::DiagnosticStage,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::workflow::DiagnosticStage::ParseShape => {
                    wire::diagnostic_stage::Value::ParseShape as i32
                }
                crate::adaptor::protocol::workflow::DiagnosticStage::Resolve => {
                    wire::diagnostic_stage::Value::Resolve as i32
                }
                crate::adaptor::protocol::workflow::DiagnosticStage::Typecheck => {
                    wire::diagnostic_stage::Value::Typecheck as i32
                }
                crate::adaptor::protocol::workflow::DiagnosticStage::ControlFlow => {
                    wire::diagnostic_stage::Value::ControlFlow as i32
                }
            }),
        })
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

impl TryFrom<crate::adaptor::protocol::workflow::DiagnosticSummary> for wire::DiagnosticSummary {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::DiagnosticSummary,
    ) -> Result<Self, String> {
        Ok(Self {
            error_count: Some(cv(value.error_count)?),
            info_count: Some(cv(value.info_count)?),
        })
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

impl TryFrom<wire::DiffFileEntryInput> for crate::adaptor::protocol::code::DiffFileEntryInput {
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

impl TryFrom<crate::usecase::code_dto::DiffStatsDto> for wire::DiffStatsDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::DiffStatsDto) -> Result<Self, String> {
        Ok(Self {
            additions: Some(cv(value.additions)?),
            deletions: Some(cv(value.deletions)?),
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

impl TryFrom<wire::DiffTreeNodeInput> for crate::adaptor::protocol::code::DiffTreeNodeInput {
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

impl TryFrom<crate::usecase::workflow::dto::ExecutionOriginDto> for wire::ExecutionOriginDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::ExecutionOriginDto) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::workflow::dto::ExecutionOriginDto::DesktopUi => {
                    wire::execution_origin_dto::Value::DesktopUi as i32
                }
                crate::usecase::workflow::dto::ExecutionOriginDto::Cli => {
                    wire::execution_origin_dto::Value::Cli as i32
                }
                crate::usecase::workflow::dto::ExecutionOriginDto::Agent => {
                    wire::execution_origin_dto::Value::Agent as i32
                }
                crate::usecase::workflow::dto::ExecutionOriginDto::Api => {
                    wire::execution_origin_dto::Value::Api as i32
                }
            }),
        })
    }
}

impl TryFrom<String> for wire::ExecutionOriginDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "desktop_ui" => wire::execution_origin_dto::Value::DesktopUi as i32,
                "cli" => wire::execution_origin_dto::Value::Cli as i32,
                "agent" => wire::execution_origin_dto::Value::Agent as i32,
                "api" => wire::execution_origin_dto::Value::Api as i32,
                _ => return Err(format!("Invalid ExecutionOriginDto: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::ExecutionOriginDto {
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

impl TryFrom<crate::usecase::workflow::dto::ExecutionStatusDto> for wire::ExecutionStatusDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::ExecutionStatusDto) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::workflow::dto::ExecutionStatusDto::Running => {
                    wire::execution_status_dto::Value::Running as i32
                }
                crate::usecase::workflow::dto::ExecutionStatusDto::Completed => {
                    wire::execution_status_dto::Value::Completed as i32
                }
                crate::usecase::workflow::dto::ExecutionStatusDto::Aborted => {
                    wire::execution_status_dto::Value::Aborted as i32
                }
            }),
        })
    }
}

impl TryFrom<String> for wire::ExecutionStatusDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "running" => wire::execution_status_dto::Value::Running as i32,
                "completed" => wire::execution_status_dto::Value::Completed as i32,
                "aborted" => wire::execution_status_dto::Value::Aborted as i32,
                _ => return Err(format!("Invalid ExecutionStatusDto: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::ExecutionStatusDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
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
                crate::adaptor::protocol::workflow::ExecutionStatusView::Completed => {
                    wire::execution_status_view::Value::Completed as i32
                }
                crate::adaptor::protocol::workflow::ExecutionStatusView::Aborted => {
                    wire::execution_status_view::Value::Aborted as i32
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

impl TryFrom<crate::adaptor::protocol::workflow::FacetUsageEntry> for wire::FacetUsageEntry {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::FacetUsageEntry,
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

impl TryFrom<crate::adaptor::protocol::terminal::GetOrSpawnTerminalV1>
    for wire::GetOrSpawnTerminalV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::terminal::GetOrSpawnTerminalV1,
    ) -> Result<Self, String> {
        Ok(Self {
            session_key: Some(cv(value.session_key)?),
            restored_from_checkpoint: Some(cv(value.restored_from_checkpoint)?),
            is_new: Some(cv(value.is_new)?),
            is_exited: Some(cv(value.is_exited)?),
            exit_code: value.exit_code.map(cv).transpose()?,
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

impl TryFrom<wire::HunkInput> for crate::adaptor::protocol::code::HunkInput {
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

impl TryFrom<crate::adaptor::protocol::notion::LabelPropertyView> for wire::LabelPropertyView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::notion::LabelPropertyView,
    ) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            property_type: Some(cv(value.property_type)?),
        })
    }
}

impl TryFrom<wire::LabelPropertyView> for crate::adaptor::protocol::notion::LabelPropertyView {
    type Error = String;
    fn try_from(value: wire::LabelPropertyView) -> Result<Self, String> {
        Ok(Self {
            name: cv(req(value.name, "name")?)?,
            property_type: cv(req(value.property_type, "property_type")?)?,
        })
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
impl<T> TryFrom<Vec<T>> for wire::ListChangedFileDto
where
    wire::ChangedFileDto: TryFrom<T>,
    <wire::ChangedFileDto as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<Vec<T>> for wire::ListDurableWorkflowFactLogEntry
where
    wire::DurableWorkflowFactLogEntry: TryFrom<T>,
    <wire::DurableWorkflowFactLogEntry as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
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
impl<T> TryFrom<Vec<T>> for wire::ListReviewHistoryEntryDto
where
    wire::ReviewHistoryEntryDto: TryFrom<T>,
    <wire::ReviewHistoryEntryDto as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<Vec<T>> for wire::ListStartupFailureActionDtoV1
where
    wire::StartupFailureActionDtoV1: TryFrom<T>,
    <wire::StartupFailureActionDtoV1 as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListTerminalInputPerformanceSampleV1
where
    wire::TerminalInputPerformanceSampleV1: TryFrom<T>,
    <wire::TerminalInputPerformanceSampleV1 as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListTerminalLaunchPerformanceSampleV1
where
    wire::TerminalLaunchPerformanceSampleV1: TryFrom<T>,
    <wire::TerminalLaunchPerformanceSampleV1 as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<Vec<T>> for wire::ListWorkflowExecutionSummaryDto
where
    wire::WorkflowExecutionSummaryDto: TryFrom<T>,
    <wire::WorkflowExecutionSummaryDto as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<Vec<T>> for wire::ListWorkspaceNodeDto
where
    wire::WorkspaceNodeDto: TryFrom<T>,
    <wire::WorkspaceNodeDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value
                .into_iter()
                .map(|item| {
                    Ok(wire::WorkspacePastAttemptDto {
                        variant: Some(wire::workspace_past_attempt_dto::Variant::Node(cv(item)?)),
                    })
                })
                .collect::<Result<_, String>>()?,
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
impl<T> TryFrom<Vec<T>> for wire::ListWorkspaceTreeItemDto
where
    wire::WorkspaceTreeItemDto: TryFrom<T>,
    <wire::WorkspaceTreeItemDto as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Vec<T>) -> Result<Self, String> {
        Ok(Self {
            items: value.into_iter().map(cv).collect::<Result<_, _>>()?,
        })
    }
}
impl<T> TryFrom<Vec<T>> for wire::ListWorkspaceWorkflowHistoryItemDto
where
    wire::WorkspaceWorkflowHistoryItemDto: TryFrom<T>,
    <wire::WorkspaceWorkflowHistoryItemDto as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<std::collections::BTreeMap<String, T>> for wire::MapPrInfoDto
where
    wire::PrInfoDto: TryFrom<T>,
    <wire::PrInfoDto as TryFrom<T>>::Error: std::fmt::Display,
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
impl<T> TryFrom<std::collections::HashMap<String, T>> for wire::MapPrInfoDto
where
    wire::PrInfoDto: TryFrom<T>,
    <wire::PrInfoDto as TryFrom<T>>::Error: std::fmt::Display,
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
impl TryFrom<wire::MarkdownDiffSideInput>
    for crate::adaptor::protocol::code::MarkdownDiffSideInput
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

impl TryFrom<crate::adaptor::protocol::workflow::NodeExecutionStatusView>
    for wire::NodeExecutionStatusView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::NodeExecutionStatusView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::Running => {
                    wire::node_execution_status_view::Value::Running as i32
                }
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::WaitingApproval => {
                    wire::node_execution_status_view::Value::WaitingApproval as i32
                }
                crate::adaptor::protocol::workflow::NodeExecutionStatusView::Succeeded => {
                    wire::node_execution_status_view::Value::Succeeded as i32
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

impl TryFrom<crate::adaptor::protocol::workflow::NodeExecutionView> for wire::NodeExecutionView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::NodeExecutionView,
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

impl TryFrom<crate::adaptor::protocol::notion::NotionConfigStatusView>
    for wire::NotionConfigStatusView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::notion::NotionConfigStatusView,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::notion::NotionConfigStatusView::NotConfigured => {
                    wire::notion_config_status_view::Value::NotConfigured as i32
                }
                crate::adaptor::protocol::notion::NotionConfigStatusView::Configured => {
                    wire::notion_config_status_view::Value::Configured as i32
                }
                crate::adaptor::protocol::notion::NotionConfigStatusView::InvalidToken => {
                    wire::notion_config_status_view::Value::InvalidToken as i32
                }
                crate::adaptor::protocol::notion::NotionConfigStatusView::InvalidDatabase => {
                    wire::notion_config_status_view::Value::InvalidDatabase as i32
                }
                crate::adaptor::protocol::notion::NotionConfigStatusView::NetworkError => {
                    wire::notion_config_status_view::Value::NetworkError as i32
                }
            }),
        })
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

impl TryFrom<crate::adaptor::protocol::notion::NotionLabelOptionView>
    for wire::NotionLabelOptionView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::notion::NotionLabelOptionView,
    ) -> Result<Self, String> {
        Ok(Self {
            property_name: Some(cv(value.property_name)?),
            property_type: Some(cv(value.property_type)?),
            options: Some(cv(value.options)?),
            option_ids: Some(cv(value.option_ids)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::notion::NotionPropertyInfoView>
    for wire::NotionPropertyInfoView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::notion::NotionPropertyInfoView,
    ) -> Result<Self, String> {
        Ok(Self {
            name: Some(cv(value.name)?),
            property_type: Some(cv(value.property_type)?),
            options: Some(cv(value.options)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::notion::NotionRepoConfigView>
    for wire::NotionRepoConfigView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::notion::NotionRepoConfigView,
    ) -> Result<Self, String> {
        Ok(Self {
            api_token: Some(cv(value.api_token)?),
            database_id: Some(cv(value.database_id)?),
            property_mapping: Some(cv(value.property_mapping)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::notion::NotionTaskPageView> for wire::NotionTaskPageView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::notion::NotionTaskPageView,
    ) -> Result<Self, String> {
        Ok(Self {
            tasks: Some(cv(value.tasks)?),
            has_more: Some(cv(value.has_more)?),
            next_cursor: value.next_cursor.map(cv).transpose()?,
        })
    }
}

impl TryFrom<wire::NotionTaskQueryInput>
    for crate::adaptor::protocol::notion::NotionTaskQueryInput
{
    type Error = String;
    fn try_from(value: wire::NotionTaskQueryInput) -> Result<Self, String> {
        Ok(Self {
            title_filter: cv(req(value.title_filter, "title_filter")?)?,
            label_filters: cv(req(value.label_filters, "label_filters")?)?,
            cursor: value.cursor.map(cv).transpose()?,
            page_size: value.page_size.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::protocol::notion::NotionTaskView> for wire::NotionTaskView {
    type Error = String;
    fn try_from(value: crate::adaptor::protocol::notion::NotionTaskView) -> Result<Self, String> {
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

impl TryFrom<crate::adaptor::protocol::notion::NotionValidationResultView>
    for wire::NotionValidationResultView
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::notion::NotionValidationResultView,
    ) -> Result<Self, String> {
        Ok(Self {
            status: Some(cv(value.status)?),
            properties: Some(cv(value.properties)?),
        })
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
impl<T> TryFrom<Option<T>> for wire::NullableListDurableWorkflowFactLogEntry
where
    wire::ListDurableWorkflowFactLogEntry: TryFrom<T>,
    <wire::ListDurableWorkflowFactLogEntry as TryFrom<T>>::Error: std::fmt::Display,
{
    type Error = String;
    fn try_from(value: Option<T>) -> Result<Self, String> {
        Ok(Self {
            value: value.map(cv).transpose()?,
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
impl<T> TryFrom<Option<T>> for wire::NullableWorkflowExecutionSummaryDto
where
    wire::WorkflowExecutionSummaryDto: TryFrom<T>,
    <wire::WorkflowExecutionSummaryDto as TryFrom<T>>::Error: std::fmt::Display,
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

impl TryFrom<crate::usecase::git_host::dto::PrAuthorDto> for wire::PrAuthorDto {
    type Error = String;
    fn try_from(value: crate::usecase::git_host::dto::PrAuthorDto) -> Result<Self, String> {
        Ok(Self {
            login: Some(cv(value.login)?),
        })
    }
}

impl TryFrom<crate::usecase::git_host::dto::PrInfoDto> for wire::PrInfoDto {
    type Error = String;
    fn try_from(value: crate::usecase::git_host::dto::PrInfoDto) -> Result<Self, String> {
        Ok(Self {
            number: Some(cv(value.number)?),
            url: Some(cv(value.url)?),
        })
    }
}

impl TryFrom<crate::usecase::git_host::dto::PrStatusDto> for wire::PrStatusDto {
    type Error = String;
    fn try_from(value: crate::usecase::git_host::dto::PrStatusDto) -> Result<Self, String> {
        Ok(Self {
            open_prs: Some(cv(value.open_prs)?),
            merged_branches: Some(cv(value.merged_branches)?),
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

impl TryFrom<crate::adaptor::protocol::notion::PropertyMappingView> for wire::PropertyMappingView {
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::notion::PropertyMappingView,
    ) -> Result<Self, String> {
        Ok(Self {
            title: Some(cv(value.title)?),
            labels: Some(cv(value.labels)?),
            branch_name: Some(cv(value.branch_name)?),
            branch_prefix: Some(cv(value.branch_prefix)?),
        })
    }
}

impl TryFrom<wire::PropertyMappingView> for crate::adaptor::protocol::notion::PropertyMappingView {
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

impl TryFrom<crate::adaptor::protocol::agent_session::ProviderAvailabilityItemResponse>
    for wire::ProviderAvailabilityItemResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::agent_session::ProviderAvailabilityItemResponse,
    ) -> Result<Self, String> {
        Ok(Self {
            provider: Some(cv(value.provider)?),
            display_name: Some(cv(value.display_name)?),
            default_executable: Some(cv(value.default_executable)?),
            configured_executable: value.configured_executable.map(cv).transpose()?,
            effective_executable: Some(cv(value.effective_executable)?),
            available: Some(cv(value.available)?),
            resolved_executable: value.resolved_executable.map(cv).transpose()?,
            unavailable_reason: value.unavailable_reason.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::protocol::agent_session::ProviderAvailabilitySnapshotResponse>
    for wire::ProviderAvailabilitySnapshotResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::agent_session::ProviderAvailabilitySnapshotResponse,
    ) -> Result<Self, String> {
        Ok(Self {
            providers: Some(cv(value.providers)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::agent_session::ProviderHookHealthProviderResponse>
    for wire::ProviderHookHealthProviderResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::agent_session::ProviderHookHealthProviderResponse,
    ) -> Result<Self, String> {
        Ok(Self { value: Some(match value { crate::adaptor::protocol::agent_session::ProviderHookHealthProviderResponse::Claude => wire::provider_hook_health_provider_response::Value::Claude as i32, crate::adaptor::protocol::agent_session::ProviderHookHealthProviderResponse::Codex => wire::provider_hook_health_provider_response::Value::Codex as i32 }) })
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

impl TryFrom<crate::adaptor::protocol::agent_session::ProviderHookHealthWarningResponse>
    for wire::ProviderHookHealthWarningResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::agent_session::ProviderHookHealthWarningResponse,
    ) -> Result<Self, String> {
        Ok(Self {
            provider: Some(cv(value.provider)?),
            launch_id: Some(cv(value.launch_id)?),
            reason: Some(cv(value.reason)?),
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
            diff_stats: Some(cv(value.diff_stats)?),
        })
    }
}

impl TryFrom<crate::usecase::repository_state::snapshot::RepositoryHeadDiffFileTreeSnapshotDto>
    for wire::RepositoryHeadDiffFileTreeSnapshotDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::repository_state::snapshot::RepositoryHeadDiffFileTreeSnapshotDto,
    ) -> Result<Self, String> {
        Ok(Self {
            version: Some(cv(value.version)?),
            stale: Some(cv(value.stale)?),
            loading: Some(cv(value.loading)?),
            combined_tree: Some(cv(value.combined_tree)?),
            staged_tree: Some(cv(value.staged_tree)?),
            changes_tree: Some(cv(value.changes_tree)?),
            staged_file_count: Some(cv(value.staged_file_count)?),
            changes_file_count: Some(cv(value.changes_file_count)?),
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

impl TryFrom<crate::usecase::comment::dto::ReviewActorKindWireDto>
    for wire::ReviewActorKindWireDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::comment::dto::ReviewActorKindWireDto,
    ) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::comment::dto::ReviewActorKindWireDto::Human => {
                    wire::review_actor_kind_wire_dto::Value::Human as i32
                }
                crate::usecase::comment::dto::ReviewActorKindWireDto::Agent => {
                    wire::review_actor_kind_wire_dto::Value::Agent as i32
                }
            }),
        })
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

impl TryFrom<crate::usecase::comment::dto::ReviewActorWireDto> for wire::ReviewActorWireDto {
    type Error = String;
    fn try_from(value: crate::usecase::comment::dto::ReviewActorWireDto) -> Result<Self, String> {
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
            original_url: value.original_url.map(cv).transpose()?,
            modified_url: value.modified_url.map(cv).transpose()?,
            original_size: value.original_size.map(cv).transpose()?,
            modified_size: value.modified_size.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::comment::dto::ReviewCommentDto> for wire::ReviewCommentDto {
    type Error = String;
    fn try_from(value: crate::usecase::comment::dto::ReviewCommentDto) -> Result<Self, String> {
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

impl TryFrom<wire::ReviewFileViewInput> for crate::adaptor::protocol::code::ReviewFileViewInput {
    type Error = String;
    fn try_from(value: wire::ReviewFileViewInput) -> Result<Self, String> {
        Ok(Self {
            worktree_path: cv(req(value.worktree_path, "worktreePath")?)?,
            target: cv(req(value.target, "target")?)?,
            section: cv(req(value.section, "section")?)?,
            base: cv(req(value.base, "base")?)?,
            snapshot_version: value.snapshot_version.map(cv).transpose()?,
            viewport: value.viewport.map(cv).transpose()?,
        })
    }
}

impl TryFrom<wire::ReviewGroupActionInput>
    for crate::adaptor::protocol::code::ReviewGroupActionInput
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

impl TryFrom<crate::usecase::comment::dto::ReviewHistoryEntryDto> for wire::ReviewHistoryEntryDto {
    type Error = String;
    fn try_from(
        value: crate::usecase::comment::dto::ReviewHistoryEntryDto,
    ) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::usecase::comment::dto::ReviewHistoryEntryDto::ThreadCreated {
                    id,
                    thread_id,
                    comment_id,
                    actor,
                    target,
                    content,
                    at,
                } => wire::review_history_entry_dto::Variant::ThreadCreated(
                    wire::ReviewHistoryEntryDtoThreadCreated {
                        id: Some(cv(id)?),
                        thread_id: Some(cv(thread_id)?),
                        comment_id: Some(cv(comment_id)?),
                        actor: Some(cv(actor)?),
                        target: Some(cv(target)?),
                        content: Some(cv(content)?),
                        at: Some(cv(at)?),
                    },
                ),
                crate::usecase::comment::dto::ReviewHistoryEntryDto::CommentAppended {
                    id,
                    thread_id,
                    comment_id,
                    actor,
                    content,
                    at,
                } => wire::review_history_entry_dto::Variant::CommentAppended(
                    wire::ReviewHistoryEntryDtoCommentAppended {
                        id: Some(cv(id)?),
                        thread_id: Some(cv(thread_id)?),
                        comment_id: Some(cv(comment_id)?),
                        actor: Some(cv(actor)?),
                        content: Some(cv(content)?),
                        at: Some(cv(at)?),
                    },
                ),
                crate::usecase::comment::dto::ReviewHistoryEntryDto::ThreadResolved {
                    id,
                    thread_id,
                    actor,
                    outcome,
                    summary,
                    at,
                } => wire::review_history_entry_dto::Variant::ThreadResolved(
                    wire::ReviewHistoryEntryDtoThreadResolved {
                        id: Some(cv(id)?),
                        thread_id: Some(cv(thread_id)?),
                        actor: Some(cv(actor)?),
                        outcome: Some(cv(outcome)?),
                        summary: Some(cv(summary)?),
                        at: Some(cv(at)?),
                    },
                ),
                crate::usecase::comment::dto::ReviewHistoryEntryDto::ThreadDeleted {
                    id,
                    thread_id,
                    actor,
                    at,
                } => wire::review_history_entry_dto::Variant::ThreadDeleted(
                    wire::ReviewHistoryEntryDtoThreadDeleted {
                        id: Some(cv(id)?),
                        thread_id: Some(cv(thread_id)?),
                        actor: Some(cv(actor)?),
                        at: Some(cv(at)?),
                    },
                ),
            }),
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

impl TryFrom<crate::usecase::comment::dto::ReviewResolveInfoDto> for wire::ReviewResolveInfoDto {
    type Error = String;
    fn try_from(value: crate::usecase::comment::dto::ReviewResolveInfoDto) -> Result<Self, String> {
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

impl TryFrom<wire::ReviewSnapshotInput> for crate::adaptor::protocol::code::ReviewSnapshotInput {
    type Error = String;
    fn try_from(value: wire::ReviewSnapshotInput) -> Result<Self, String> {
        Ok(Self {
            worktree_path: cv(req(value.worktree_path, "worktreePath")?)?,
            base: cv(req(value.base, "base")?)?,
        })
    }
}

impl TryFrom<wire::ReviewTargetInput> for crate::adaptor::protocol::code::ReviewTargetInput {
    type Error = String;
    fn try_from(value: wire::ReviewTargetInput) -> Result<Self, String> {
        Ok(match req(value.variant, "variant")? {
            wire::review_target_input::Variant::FileId(value) => {
                crate::adaptor::protocol::code::ReviewTargetInput::FileId(cv(value)?)
            }
            wire::review_target_input::Variant::Path(value) => {
                crate::adaptor::protocol::code::ReviewTargetInput::Path(cv(value)?)
            }
        })
    }
}

impl TryFrom<crate::usecase::comment::dto::ReviewTargetWireDto> for wire::ReviewTargetWireDto {
    type Error = String;
    fn try_from(value: crate::usecase::comment::dto::ReviewTargetWireDto) -> Result<Self, String> {
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
            viewport: value.viewport.map(cv).transpose()?,
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

impl TryFrom<crate::usecase::comment::dto::ReviewThreadDto> for wire::ReviewThreadDto {
    type Error = String;
    fn try_from(value: crate::usecase::comment::dto::ReviewThreadDto) -> Result<Self, String> {
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

impl TryFrom<wire::ReviewThreadFilterDto> for crate::usecase::comment::dto::ReviewThreadFilterDto {
    type Error = String;
    fn try_from(value: wire::ReviewThreadFilterDto) -> Result<Self, String> {
        Ok(Self {
            file: value.file.map(cv).transpose()?,
            state: value.state.map(cv).transpose()?,
            author: value.author.map(cv).transpose()?,
            unread: value.unread.map(cv).transpose()?,
            thread_id: value.thread_id.map(cv).transpose()?.unwrap_or_default(),
        })
    }
}

impl TryFrom<crate::usecase::comment::dto::ReviewThreadStateDto> for wire::ReviewThreadStateDto {
    type Error = String;
    fn try_from(value: crate::usecase::comment::dto::ReviewThreadStateDto) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::comment::dto::ReviewThreadStateDto::Open => {
                    wire::review_thread_state_dto::Value::Open as i32
                }
                crate::usecase::comment::dto::ReviewThreadStateDto::Resolved => {
                    wire::review_thread_state_dto::Value::Resolved as i32
                }
            }),
        })
    }
}

impl TryFrom<wire::ReviewThreadStateDto> for crate::usecase::comment::dto::ReviewThreadStateDto {
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

impl TryFrom<crate::usecase::workflow::dto::SessionProviderDto> for wire::SessionProviderDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::SessionProviderDto) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::usecase::workflow::dto::SessionProviderDto::Claude => {
                    wire::session_provider_dto::Value::Claude as i32
                }
                crate::usecase::workflow::dto::SessionProviderDto::Codex => {
                    wire::session_provider_dto::Value::Codex as i32
                }
            }),
        })
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

impl TryFrom<crate::adaptor::protocol::workflow::Severity> for wire::Severity {
    type Error = String;
    fn try_from(value: crate::adaptor::protocol::workflow::Severity) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::adaptor::protocol::workflow::Severity::Error => {
                    wire::severity::Value::Error as i32
                }
                crate::adaptor::protocol::workflow::Severity::Info => {
                    wire::severity::Value::Info as i32
                }
            }),
        })
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

impl TryFrom<crate::adaptor::protocol::application_lifecycle_v1::StartupFailureActionDtoV1>
    for wire::StartupFailureActionDtoV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::application_lifecycle_v1::StartupFailureActionDtoV1,
    ) -> Result<Self, String> {
        Ok(Self { value: Some(match value { crate::adaptor::protocol::application_lifecycle_v1::StartupFailureActionDtoV1::Quit => wire::startup_failure_action_dto_v1::Value::Quit as i32 }) })
    }
}

impl TryFrom<String> for wire::StartupFailureActionDtoV1 {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "quit" => wire::startup_failure_action_dto_v1::Value::Quit as i32,
                _ => return Err(format!("Invalid StartupFailureActionDtoV1: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::StartupFailureActionDtoV1 {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::adaptor::protocol::application_lifecycle_v1::StartupFailureKindDtoV1>
    for wire::StartupFailureKindDtoV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::application_lifecycle_v1::StartupFailureKindDtoV1,
    ) -> Result<Self, String> {
        Ok(Self { value: Some(match value { crate::adaptor::protocol::application_lifecycle_v1::StartupFailureKindDtoV1::StoreInUse => wire::startup_failure_kind_dto_v1::Value::StoreInUse as i32, crate::adaptor::protocol::application_lifecycle_v1::StartupFailureKindDtoV1::StorageUnavailable => wire::startup_failure_kind_dto_v1::Value::StorageUnavailable as i32, crate::adaptor::protocol::application_lifecycle_v1::StartupFailureKindDtoV1::UnsupportedRuntime => wire::startup_failure_kind_dto_v1::Value::UnsupportedRuntime as i32, crate::adaptor::protocol::application_lifecycle_v1::StartupFailureKindDtoV1::UnsupportedStoreVersion => wire::startup_failure_kind_dto_v1::Value::UnsupportedStoreVersion as i32, crate::adaptor::protocol::application_lifecycle_v1::StartupFailureKindDtoV1::InitializationStateInvalid => wire::startup_failure_kind_dto_v1::Value::InitializationStateInvalid as i32, crate::adaptor::protocol::application_lifecycle_v1::StartupFailureKindDtoV1::StoreValidationFailed => wire::startup_failure_kind_dto_v1::Value::StoreValidationFailed as i32, crate::adaptor::protocol::application_lifecycle_v1::StartupFailureKindDtoV1::SchemaEvolutionFailed => wire::startup_failure_kind_dto_v1::Value::SchemaEvolutionFailed as i32 }) })
    }
}

impl TryFrom<String> for wire::StartupFailureKindDtoV1 {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "store_in_use" => wire::startup_failure_kind_dto_v1::Value::StoreInUse as i32,
                "storage_unavailable" => {
                    wire::startup_failure_kind_dto_v1::Value::StorageUnavailable as i32
                }
                "unsupported_runtime" => {
                    wire::startup_failure_kind_dto_v1::Value::UnsupportedRuntime as i32
                }
                "unsupported_store_version" => {
                    wire::startup_failure_kind_dto_v1::Value::UnsupportedStoreVersion as i32
                }
                "initialization_state_invalid" => {
                    wire::startup_failure_kind_dto_v1::Value::InitializationStateInvalid as i32
                }
                "store_validation_failed" => {
                    wire::startup_failure_kind_dto_v1::Value::StoreValidationFailed as i32
                }
                "schema_evolution_failed" => {
                    wire::startup_failure_kind_dto_v1::Value::SchemaEvolutionFailed as i32
                }
                _ => return Err(format!("Invalid StartupFailureKindDtoV1: {value}")),
            }),
        })
    }
}

impl TryFrom<&str> for wire::StartupFailureKindDtoV1 {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        cv(value.to_owned())
    }
}

impl TryFrom<crate::adaptor::protocol::application_lifecycle_v1::StartupFailureQuitOutcomeDtoV1>
    for wire::StartupFailureQuitOutcomeDtoV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::application_lifecycle_v1::StartupFailureQuitOutcomeDtoV1,
    ) -> Result<Self, String> {
        Ok(Self { variant: Some(match value { crate::adaptor::protocol::application_lifecycle_v1::StartupFailureQuitOutcomeDtoV1::Accepted { correlation_id } => wire::startup_failure_quit_outcome_dto_v1::Variant::Accepted(wire::StartupFailureQuitOutcomeDtoV1Accepted { correlation_id: Some(cv(correlation_id)?) }) }) })
    }
}

impl TryFrom<crate::adaptor::protocol::terminal::TerminalInputPerformanceSampleV1>
    for wire::TerminalInputPerformanceSampleV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::terminal::TerminalInputPerformanceSampleV1,
    ) -> Result<Self, String> {
        Ok(Self {
            sequence: Some(cv(value.sequence)?),
            on_data_to_command_ingress_ms: Some(cv(value.on_data_to_command_ingress_ms)?),
            command_ingress_to_admission_ms: Some(cv(value.command_ingress_to_admission_ms)?),
            admission_to_writer_enqueue_ms: Some(cv(value.admission_to_writer_enqueue_ms)?),
            writer_enqueue_to_output_read_ms: Some(cv(value.writer_enqueue_to_output_read_ms)?),
            output_read_to_model_apply_ms: Some(cv(value.output_read_to_model_apply_ms)?),
            model_apply_to_event_publish_ms: Some(cv(value.model_apply_to_event_publish_ms)?),
            event_published_at_unix_ms: Some(cv(value.event_published_at_unix_ms)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::terminal::TerminalLaunchPerformanceSampleV1>
    for wire::TerminalLaunchPerformanceSampleV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::terminal::TerminalLaunchPerformanceSampleV1,
    ) -> Result<Self, String> {
        Ok(Self {
            phase: Some(cv(value.phase)?),
            duration_ms: Some(cv(value.duration_ms)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::terminal::TerminalPerformanceSwitchesV1>
    for wire::TerminalPerformanceSwitchesV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::terminal::TerminalPerformanceSwitchesV1,
    ) -> Result<Self, String> {
        Ok(Self {
            disable_output_flow_control: Some(cv(value.disable_output_flow_control)?),
            disable_terminal_journal: Some(cv(value.disable_terminal_journal)?),
            disable_renderer_write_serialization: Some(cv(
                value.disable_renderer_write_serialization
            )?),
            disable_webgl_renderer: Some(cv(value.disable_webgl_renderer)?),
        })
    }
}

impl TryFrom<wire::TerminalSurfaceOwnerV1>
    for crate::adaptor::protocol::terminal::TerminalSurfaceOwnerV1
{
    type Error = String;
    fn try_from(value: wire::TerminalSurfaceOwnerV1) -> Result<Self, String> {
        Ok(match req(value.variant, "variant")? {
            wire::terminal_surface_owner_v1::Variant::Workspace(value) => {
                crate::adaptor::protocol::terminal::TerminalSurfaceOwnerV1::Workspace {
                    workspace_path: cv(req(value.workspace_path, "workspacePath")?)?,
                }
            }
            wire::terminal_surface_owner_v1::Variant::Session(value) => {
                crate::adaptor::protocol::terminal::TerminalSurfaceOwnerV1::Session {
                    workspace_path: cv(req(value.workspace_path, "workspacePath")?)?,
                    session_id: cv(req(value.session_id, "sessionId")?)?,
                }
            }
        })
    }
}

impl TryFrom<crate::adaptor::protocol::terminal::TerminalSurfaceSummaryV1>
    for wire::TerminalSurfaceSummaryV1
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::terminal::TerminalSurfaceSummaryV1,
    ) -> Result<Self, String> {
        Ok(Self {
            session_key: Some(cv(value.session_key)?),
            is_exited: Some(cv(value.is_exited)?),
            exit_code: value.exit_code.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::usecase::workflow::dto::TokenUsageDto> for wire::TokenUsageDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::dto::TokenUsageDto) -> Result<Self, String> {
        Ok(Self {
            input_tokens: Some(cv(value.input_tokens)?),
            output_tokens: Some(cv(value.output_tokens)?),
        })
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

impl TryFrom<crate::usecase::code_dto::ViewportDto> for wire::ViewportDto {
    type Error = String;
    fn try_from(value: crate::usecase::code_dto::ViewportDto) -> Result<Self, String> {
        Ok(Self {
            start_line: Some(cv(value.start_line)?),
            end_line: Some(cv(value.end_line)?),
        })
    }
}

impl TryFrom<wire::ViewportInput> for crate::adaptor::protocol::code::ViewportInput {
    type Error = String;
    fn try_from(value: wire::ViewportInput) -> Result<Self, String> {
        Ok(Self {
            start_line: cv(req(value.start_line, "startLine")?)?,
            end_line: cv(req(value.end_line, "endLine")?)?,
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

impl TryFrom<crate::usecase::workflow::dto::WorkflowExecutionSummaryDto>
    for wire::WorkflowExecutionSummaryDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::dto::WorkflowExecutionSummaryDto,
    ) -> Result<Self, String> {
        Ok(Self {
            execution_id: Some(cv(value.execution_id)?),
            workflow_name: Some(cv(value.workflow_name)?),
            status: Some(cv(value.status)?),
            worktree_path: Some(cv(value.worktree_path)?),
            current_node: value.current_node.map(cv).transpose()?,
            created_from: Some(cv(value.created_from)?),
            started_at: Some(cv(value.started_at)?),
            updated_at: Some(cv(value.updated_at)?),
            completed_at: value.completed_at.map(cv).transpose()?,
            error_reason: value.error_reason.map(cv).transpose()?,
            total_token_usage: Some(cv(value.total_token_usage)?),
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
            total_token_usage: Some(cv(value.total_token_usage)?),
            node_executions: Some(cv(value.node_executions)?),
            artifacts: Some(cv(value.artifacts)?),
            fanouts: Some(cv(value.fanouts)?),
            approval_target: value.approval_target.map(cv).transpose()?,
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::WorkflowGetOutputResponse>
    for wire::WorkflowGetOutputResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::WorkflowGetOutputResponse,
    ) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::adaptor::protocol::workflow::WorkflowGetOutputResponse::Submitted {
                    contract,
                    structured_output,
                    submitted_at,
                    request_id,
                    timestamp,
                } => wire::workflow_get_output_response::Variant::Submitted(
                    wire::WorkflowGetOutputResponseSubmitted {
                        contract: contract.map(cv).transpose()?,
                        structured_output: Some(cv(structured_output)?),
                        submitted_at: submitted_at.map(cv).transpose()?,
                        request_id: request_id.map(cv).transpose()?,
                        timestamp: Some(cv(timestamp)?),
                    },
                ),
                crate::adaptor::protocol::workflow::WorkflowGetOutputResponse::NotSubmitted => {
                    wire::workflow_get_output_response::Variant::NotSubmitted(wire::Unit {})
                }
            }),
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

impl TryFrom<crate::adaptor::gateway::app_config::WorkflowSection> for wire::WorkflowSection {
    type Error = String;
    fn try_from(
        value: crate::adaptor::gateway::app_config::WorkflowSection,
    ) -> Result<Self, String> {
        Ok(Self {
            approval_auto_approve: Some(cv(value.approval_auto_approve)?),
        })
    }
}

impl TryFrom<wire::WorkflowSection> for crate::adaptor::gateway::app_config::WorkflowSection {
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

impl TryFrom<crate::domain::workflow::WorkflowSourceFormat> for wire::WorkflowSourceFormat {
    type Error = String;
    fn try_from(value: crate::domain::workflow::WorkflowSourceFormat) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value {
                crate::domain::workflow::WorkflowSourceFormat::Yaml => {
                    wire::workflow_source_format::Value::Yaml as i32
                }
                crate::domain::workflow::WorkflowSourceFormat::Lua => {
                    wire::workflow_source_format::Value::Lua as i32
                }
            }),
        })
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

impl TryFrom<wire::WorkflowSubmitArtifactInput>
    for crate::adaptor::protocol::workflow::WorkflowSubmitArtifactInput
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
            name: Some(cv(value.name)?),
            description: Some(cv(value.description)?),
            builtin: Some(cv(value.builtin)?),
            is_running: Some(cv(value.is_running)?),
            source_format: Some(cv(value.source_format)?),
        })
    }
}

impl TryFrom<crate::adaptor::protocol::workflow::WorkflowValidateOutputResponse>
    for wire::WorkflowValidateOutputResponse
{
    type Error = String;
    fn try_from(
        value: crate::adaptor::protocol::workflow::WorkflowValidateOutputResponse,
    ) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::adaptor::protocol::workflow::WorkflowValidateOutputResponse::Valid => {
                    wire::workflow_validate_output_response::Variant::Valid(wire::Unit {})
                }
                crate::adaptor::protocol::workflow::WorkflowValidateOutputResponse::Invalid {
                    reason,
                    details,
                } => wire::workflow_validate_output_response::Variant::Invalid(
                    wire::WorkflowValidateOutputResponseInvalid {
                        reason: Some(cv(reason)?),
                        details: Some(cv(details)?),
                    },
                ),
            }),
        })
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

impl TryFrom<crate::usecase::workflow::WorkspaceFanoutDto> for wire::WorkspaceFanoutDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::WorkspaceFanoutDto) -> Result<Self, String> {
        Ok(Self {
            worktree: value.worktree.map(cv).transpose()?,
            id: Some(cv(value.id)?),
            title: Some(cv(value.title)?),
            status: Some(cv(value.status)?),
            workflow_capabilities: value.workflow_capabilities.map(cv).transpose()?,
            children: Some(cv(value.children)?),
            updated_at: Some(cv(value.updated_at)?),
        })
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
            status_classification: Some(cv(value.status_classification)?),
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

impl TryFrom<crate::usecase::workflow::WorkspaceNodeDto> for wire::WorkspaceNodeDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::WorkspaceNodeDto) -> Result<Self, String> {
        Ok(Self {
            process_presence: Some(cv(value.process_presence)?),
            id: Some(cv(value.id)?),
            title: Some(cv(value.title)?),
            status: Some(cv(value.status)?),
            error_reason: value.error_reason.map(cv).transpose()?,
            content_kind: Some(cv(value.content_kind)?),
            capabilities: Some(cv(value.capabilities)?),
            workflow_capabilities: value.workflow_capabilities.map(cv).transpose()?,
            session_capabilities: value.session_capabilities.map(cv).transpose()?,
            children: Some(cv(value.children)?),
            past_attempts: Some(cv(value.past_attempts)?),
            past_attempts_collapsed: Some(cv(value.past_attempts_collapsed)?),
            updated_at: Some(cv(value.updated_at)?),
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

impl TryFrom<crate::usecase::workflow::WorkspaceSelectionReconciliationDto>
    for wire::WorkspaceSelectionReconciliationDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::WorkspaceSelectionReconciliationDto,
    ) -> Result<Self, String> {
        Ok(Self {
            selection_in_snapshot: Some(cv(value.selection_in_snapshot)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceSequenceDto> for wire::WorkspaceSequenceDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::WorkspaceSequenceDto) -> Result<Self, String> {
        Ok(Self {
            worktree: value.worktree.map(cv).transpose()?,
            id: Some(cv(value.id)?),
            title: Some(cv(value.title)?),
            status: Some(cv(value.status)?),
            workflow_capabilities: value.workflow_capabilities.map(cv).transpose()?,
            children: Some(cv(value.children)?),
            updated_at: Some(cv(value.updated_at)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceSessionCapabilitiesDto>
    for wire::WorkspaceSessionCapabilitiesDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::WorkspaceSessionCapabilitiesDto,
    ) -> Result<Self, String> {
        Ok(Self {
            session_ref: Some(cv(value.session_ref)?),
            can_archive: Some(cv(value.can_archive)?),
            can_delete: Some(cv(value.can_delete)?),
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

impl TryFrom<String> for wire::WorkspaceStatusClassification {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "active" => wire::workspace_status_classification::Value::Active as i32,
                "attention" => wire::workspace_status_classification::Value::Attention as i32,
                "idle" => wire::workspace_status_classification::Value::Idle as i32,
                "unbound" => wire::workspace_status_classification::Value::Unbound as i32,
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

impl TryFrom<crate::usecase::workflow::WorkspaceTreeItemDto> for wire::WorkspaceTreeItemDto {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::WorkspaceTreeItemDto) -> Result<Self, String> {
        Ok(Self {
            variant: Some(match value {
                crate::usecase::workflow::WorkspaceTreeItemDto::Node(value) => {
                    wire::workspace_tree_item_dto::Variant::Node(cv(value)?)
                }
                crate::usecase::workflow::WorkspaceTreeItemDto::Sequence(value) => {
                    wire::workspace_tree_item_dto::Variant::Sequence(cv(value)?)
                }
                crate::usecase::workflow::WorkspaceTreeItemDto::Fanout(value) => {
                    wire::workspace_tree_item_dto::Variant::Fanout(cv(value)?)
                }
            }),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceTreeSelectionSnapshotDto>
    for wire::WorkspaceTreeSelectionSnapshotDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::WorkspaceTreeSelectionSnapshotDto,
    ) -> Result<Self, String> {
        Ok(Self {
            snapshot: Some(cv(value.snapshot)?),
            reconciliation: Some(cv(value.reconciliation)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceTreeSnapshotDto>
    for wire::WorkspaceTreeSnapshotDto
{
    type Error = String;
    fn try_from(value: crate::usecase::workflow::WorkspaceTreeSnapshotDto) -> Result<Self, String> {
        Ok(Self {
            nodes: Some(cv(value.nodes)?),
            archived_sessions: Some(cv(value.archived_sessions)?),
            preferred_node_id: value.preferred_node_id.map(cv).transpose()?,
        })
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

impl TryFrom<crate::usecase::workflow::WorkspaceWorkflowCapabilitiesDto>
    for wire::WorkspaceWorkflowCapabilitiesDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::WorkspaceWorkflowCapabilitiesDto,
    ) -> Result<Self, String> {
        Ok(Self {
            can_abort: Some(cv(value.can_abort)?),
            can_archive: Some(cv(value.can_archive)?),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkspaceWorkflowHistoryItemDto>
    for wire::WorkspaceWorkflowHistoryItemDto
{
    type Error = String;
    fn try_from(
        value: crate::usecase::workflow::WorkspaceWorkflowHistoryItemDto,
    ) -> Result<Self, String> {
        Ok(Self {
            execution_id: Some(cv(value.execution_id)?),
            worktree_path: Some(cv(value.worktree_path)?),
            title: Some(cv(value.title)?),
            status: Some(cv(value.status)?),
            updated_at: Some(cv(value.updated_at)?),
            archived_at: Some(cv(value.archived_at)?),
            archive_reason: Some(cv(value.archive_reason)?),
        })
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

impl TryFrom<crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitOutcomeDtoV1>
    for wire::ApplicationQuitOutcomeDtoV1
{
    type Error = String;
    fn try_from(
        _: crate::adaptor::protocol::application_lifecycle_v1::ApplicationQuitOutcomeDtoV1,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            variant: Some(wire::application_quit_outcome_dto_v1::Variant::Accepted(
                wire::Unit {},
            )),
        })
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
