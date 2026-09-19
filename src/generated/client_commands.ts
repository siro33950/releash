// Generated from proto/client.proto. Run pnpm generate:protocol.
import {
	type DescMessage,
	fromJson,
	type Message,
	toJson,
} from "@bufbuild/protobuf";
import { type Client, ConnectError } from "@connectrpc/connect";
import { getClient, refreshClientOnDisconnect } from "@/lib/client";
import { clientJson } from "@/lib/clientJson";
import {
	AbortWorkflowRequestSchema,
	AcknowledgeApplicationAttemptRequestSchema,
	AckTerminalSurfaceOutputRequestSchema,
	AddRepoPathRequestSchema,
	AgentSessionArchiveResponseSchema,
	AgentSessionHistoryPageDtoSchema,
	AgentSessionOpenResponseSchema,
	AppendReviewCommentRequestSchema,
	ApplicationQuitLookupDtoV1Schema,
	ApplicationQuitOutcomeDtoV1Schema,
	ApplicationStartupOutcomeDtoV1Schema,
	ApproveWorkflowNodeRequestSchema,
	ApproveWorkspaceNodeRequestSchema,
	AppSectionSchema,
	ArchiveAgentSessionRequestSchema,
	ArchiveWorkspaceWorkflowExecutionRequestSchema,
	BranchDiffSummaryDtoSchema,
	BuildDiffFileTreeRequestSchema,
	BuildReviewThreadHandoffRequestSchema,
	type ClientService,
	CommandErrorSchema,
	CompactApplicationShutdownDetailsRequestSchema,
	ComputeHiddenRangesFromContentRequestSchema,
	ComputeHiddenRangesRequestSchema,
	ComputeMarkdownDiffRangesRequestSchema,
	ComputeMarkdownInlineChunksRequestSchema,
	ComputeMarkdownSplitRowsRequestSchema,
	ComputeVisibleMarkdownBlocksRequestSchema,
	ConfirmAgentSessionArchiveDeleteRequestSchema,
	CreateAgentSessionRequestSchema,
	CreateReviewThreadRequestSchema,
	CreateWorktreeRequestSchema,
	CurrentShutdownResultDtoV1Schema,
	DeleteAgentSessionRequestSchema,
	DeleteBranchRequestSchema,
	DeleteFacetRequestSchema,
	DeleteNotionConfigRequestSchema,
	DeleteReviewThreadRequestSchema,
	DeleteWorkflowRequestSchema,
	DetachTerminalSurfaceRequestSchema,
	DetectEditorsRequestSchema,
	DiagnoseAllCmdRequestSchema,
	DiagnosticReportSchema,
	DuplicateFacetRequestSchema,
	DuplicateWorkflowRequestSchema,
	FetchIssuesRequestSchema,
	FetchNotionLabelOptionsRequestSchema,
	FetchPrStatusRequestSchema,
	FileNavigationResultDtoSchema,
	GetAgentSessionRequestSchema,
	GetApplicationQuitOperationRequestSchema,
	GetApplicationShutdownRequestSchema,
	GetApplicationStartupOutcomeRequestSchema,
	GetAppSettingsRequestSchema,
	GetAutomationConfigDirRequestSchema,
	GetBinaryFileAtBranchBaseRequestSchema,
	GetBinaryFileAtRefRequestSchema,
	GetBinaryStagedContentRequestSchema,
	GetBranchBaseRequestSchema,
	GetBranchDiffSummaryRequestSchema,
	GetCachedIssuesRequestSchema,
	GetCachedPrStatusRequestSchema,
	GetCrashReportingEnabledRequestSchema,
	GetCurrentBranchRequestSchema,
	GetCwdRequestSchema,
	GetDefaultBranchRequestSchema,
	GetExternalEditorRequestSchema,
	GetFacetRequestSchema,
	GetFileAtBranchBaseRequestSchema,
	GetFileAtRefRequestSchema,
	GetFileNavigationRequestSchema,
	GetGitLogRequestSchema,
	GetGitStatusRequestSchema,
	GetGitStatusSnapshotRequestSchema,
	GetHeadDiffFileTreeSnapshotRequestSchema,
	GetLanguageFromPathRequestSchema,
	GetMainRepoPathRequestSchema,
	GetNotionConfigRequestSchema,
	GetOrSpawnTerminalSurfaceRequestSchema,
	GetOrSpawnTerminalV1Schema,
	GetPerformanceRealAppModeRequestSchema,
	GetPerformanceTelemetryEnabledRequestSchema,
	GetProviderAvailabilityRequestSchema,
	GetRelativePathRequestSchema,
	GetReleashBaseRequestSchema,
	GetRepoGitDirRequestSchema,
	GetRepoPathsRequestSchema,
	GetReviewBlobRequestSchema,
	GetReviewFileViewRequestSchema,
	GetReviewSnapshotRequestSchema,
	GetReviewThreadHistoryRequestSchema,
	GetReviewThreadRequestSchema,
	GetShutdownPlanRequestSchema,
	GetStagedContentRequestSchema,
	GetStatusDiffStatsRequestSchema,
	GetStatusDiffStatsSnapshotRequestSchema,
	GetTerminalPerformanceSwitchesRequestSchema,
	GetTerminalSurfaceRequestSchema,
	GetWorkflowConfigRequestSchema,
	GetWorkflowExecutionLogRequestSchema,
	GetWorkflowExecutionRequestSchema,
	GetWorkflowExecutionStateRequestSchema,
	GetWorkflowNodeDetailRequestSchema,
	GetWorkflowRequestSchema,
	GetWorkflowSourceRequestSchema,
	GetWorkspaceNodeDetailRequestSchema,
	GetWorkspaceSessionNodeIdRequestSchema,
	GetWorkspaceTreeSelectionReconciliationRequestSchema,
	GetWorktreeDirtyCountRequestSchema,
	GitCreateBranchRequestSchema,
	GitStageRequestSchema,
	GitStageReviewGroupRequestSchema,
	GitUnstageRequestSchema,
	GitUnstageReviewGroupRequestSchema,
	KillTerminalSurfaceRequestSchema,
	ListAgentSessionHistoryRequestSchema,
	ListAgentSessionProviderDtoSchema,
	ListAvailableAgentSessionProvidersRequestSchema,
	ListBranchCardDtoSchema,
	ListBranchDtoSchema,
	ListBranchesRequestSchema,
	ListBranchesWithStatusRequestSchema,
	ListBranchesWithStatusSnapshotRequestSchema,
	ListCommitDtoSchema,
	ListDiffRangeDtoSchema,
	ListDiffTreeNodeDtoSchema,
	ListEditorInfoDtoSchema,
	ListFacetSummariesRequestSchema,
	ListFacetSummaryDtoSchema,
	ListFacetsRequestSchema,
	ListFileDiffStatDtoSchema,
	ListFileStatusDtoSchema,
	ListHiddenRangeDtoSchema,
	ListInlineChunkDtoSchema,
	ListIssueInfoDtoSchema,
	ListNotionLabelOptionViewSchema,
	ListPendingApplicationAttemptsRequestSchema,
	ListProviderHookHealthWarningResponseSchema,
	ListProviderHookHealthWarningsRequestSchema,
	ListReviewHistoryEntryDtoSchema,
	ListReviewThreadDtoSchema,
	ListReviewThreadsRequestSchema,
	ListSplitRowDtoSchema,
	ListstringSchema,
	ListTerminalInputPerformanceSampleV1Schema,
	ListTerminalLaunchPerformanceSampleV1Schema,
	ListVisibleBlockDtoSchema,
	ListWorkflowExecutionSummaryDtoSchema,
	ListWorkflowExecutionsRequestSchema,
	ListWorkflowSummaryDtoSchema,
	ListWorkflowsRequestSchema,
	ListWorkspaceWorkflowHistoryItemDtoSchema,
	ListWorkspaceWorkflowHistoryRequestSchema,
	ListWorkspaceWorktreeNodesRequestSchema,
	ListWorktreeEntryDtoSchema,
	ListWorktreesRequestSchema,
	LoadWorkspaceStateRequestSchema,
	NotionTaskPageViewSchema,
	NotionValidationResultViewSchema,
	NullableAgentSessionItemDtoSchema,
	NullableListDurableWorkflowFactLogEntrySchema,
	NullableNodeExecutionViewSchema,
	NullableNotionRepoConfigViewSchema,
	NullablestringSchema,
	NullableWorkflowExecutionSummaryDtoSchema,
	NullableWorkflowExecutionViewSchema,
	NullableWorkspaceNodeDetailDtoSchema,
	NullableWorkspaceStateDtoSchema,
	OpenAgentSessionRequestSchema,
	OpenFacetInEditorRequestSchema,
	OpenFolderInEditorRequestSchema,
	OpenInEditorRequestSchema,
	OpenWorkflowInEditorRequestSchema,
	PendingCallerAttemptPageDtoV1Schema,
	ProviderAvailabilitySnapshotResponseSchema,
	PrStatusDtoSchema,
	QueryNotionTasksRequestSchema,
	QuitAfterStartupFailureRequestSchema,
	RecordTerminalLaunchRendererPhaseRequestSchema,
	RecoveryActionOutcomeDtoV1Schema,
	RefreshProviderAvailabilityRequestSchema,
	RemoveRepoPathRequestSchema,
	RemoveWorktreeRequestSchema,
	RenameWorkspaceSessionNodeRequestSchema,
	RenderFacetPreviewRequestSchema,
	ReportFrontendErrorRequestSchema,
	ReportMountedXtermCountRequestSchema,
	ReportUsageEventRequestSchema,
	RepositoryBranchCardsSnapshotDtoSchema,
	RepositoryDiffStatsSnapshotDtoSchema,
	RepositoryHeadDiffFileTreeSnapshotDtoSchema,
	RepositoryStatusSnapshotDtoSchema,
	RequestApplicationQuitRequestSchema,
	ResetProviderExecutableRequestSchema,
	ResizeTerminalSurfaceRequestSchema,
	ResolveActiveExecutionByWorktreeRequestSchema,
	ResolveReviewThreadRequestSchema,
	ResolveShutdownTargetActionRequestSchema,
	ResolveWorktreeByExecutionRequestSchema,
	RestoreAgentSessionRequestSchema,
	RestoreWorkspaceWorkflowExecutionRequestSchema,
	ResultBoolSchema,
	ResultStringSchema,
	ResultUint32Schema,
	ResumeAgentSessionHistoryCandidateRequestSchema,
	ResumeAgentSessionRequestSchema,
	ResumeWorkflowRequestSchema,
	RetryWorkspaceNodeRequestSchema,
	ReviewFileViewDtoSchema,
	ReviewSnapshotDtoSchema,
	ReviewThreadDtoSchema,
	SaveFacetRequestSchema,
	SaveNotionConfigRequestSchema,
	SaveWorkflowSourceRequestSchema,
	SaveWorkflowSourceResultDtoSchema,
	SaveWorkspaceStateRequestSchema,
	SetBranchBaseRequestSchema,
	SetReleashBaseRequestSchema,
	ShutdownPlanDtoV1Schema,
	ShutdownPlanPageDtoV1Schema,
	StartTerminalInputPerformanceCollectionRequestSchema,
	StartTerminalLaunchPerformanceCollectionRequestSchema,
	StartupFailureQuitOutcomeDtoV1Schema,
	StartWorkflowRequestSchema,
	StopWatchingRequestSchema,
	StopWorkflowRequestSchema,
	TakeTerminalInputPerformanceSamplesRequestSchema,
	TakeTerminalLaunchPerformanceSamplesRequestSchema,
	TerminalPerformanceSwitchesV1Schema,
	TerminalSurfaceSummaryV1Schema,
	UnitSchema,
	UpdateAppSettingsRequestSchema,
	UpdateCrashReportingRequestSchema,
	UpdateExternalEditorRequestSchema,
	UpdateLoginItemPreferenceRequestSchema,
	UpdatePerformanceTelemetryRequestSchema,
	UpdateProviderExecutableRequestSchema,
	UpdateWorkflowConfigRequestSchema,
	ValidateNotionConfigRequestSchema,
	WorkflowDtoSchema,
	WorkflowGetOutputRequestSchema,
	WorkflowGetOutputResponseSchema,
	WorkflowSectionSchema,
	WorkflowSubmitOutputRequestSchema,
	WorkflowValidateOutputRequestSchema,
	WorkflowValidateOutputResponseSchema,
	WorkspaceTreeSelectionSnapshotDtoSchema,
	WorkspaceTreeSnapshotDtoSchema,
	WorktreeEntryDtoSchema,
	WritePathsToTerminalSurfaceRequestSchema,
	WriteTerminalSurfaceRequestSchema,
} from "./client_pb";
import type { ClientCommandArgs, ClientCommandResults } from "./client_types";

const decode = (schema: DescMessage, message: Message) =>
	clientJson(schema, toJson(schema, message), false);
const commands = {
	get_review_blob: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_review_blob"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getReviewBlob(
				fromJson(
					GetReviewBlobRequestSchema,
					clientJson(
						GetReviewBlobRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	abort_workflow: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["abort_workflow"],
	) => {
		const result = decode(
			UnitSchema,
			await client.abortWorkflow(
				fromJson(
					AbortWorkflowRequestSchema,
					clientJson(
						AbortWorkflowRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	ack_terminal_surface_output: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["ack_terminal_surface_output"],
	) => {
		const result = decode(
			UnitSchema,
			await client.ackTerminalSurfaceOutput(
				fromJson(
					AckTerminalSurfaceOutputRequestSchema,
					clientJson(
						AckTerminalSurfaceOutputRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	acknowledge_application_attempt: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["acknowledge_application_attempt"],
	) => {
		const result = decode(
			UnitSchema,
			await client.acknowledgeApplicationAttempt(
				fromJson(
					AcknowledgeApplicationAttemptRequestSchema,
					clientJson(
						AcknowledgeApplicationAttemptRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	add_repo_path: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["add_repo_path"],
	) => {
		const result = decode(
			ResultBoolSchema,
			await client.addRepoPath(
				fromJson(
					AddRepoPathRequestSchema,
					clientJson(
						AddRepoPathRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	append_review_comment: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["append_review_comment"],
	) => {
		const result = decode(
			ReviewThreadDtoSchema,
			await client.appendReviewComment(
				fromJson(
					AppendReviewCommentRequestSchema,
					clientJson(
						AppendReviewCommentRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	approve_workspace_node: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["approve_workspace_node"],
	) => {
		const result = decode(
			UnitSchema,
			await client.approveWorkspaceNode(
				fromJson(
					ApproveWorkspaceNodeRequestSchema,
					clientJson(
						ApproveWorkspaceNodeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	archive_agent_session: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["archive_agent_session"],
	) => {
		const result = decode(
			AgentSessionArchiveResponseSchema,
			await client.archiveAgentSession(
				fromJson(
					ArchiveAgentSessionRequestSchema,
					clientJson(
						ArchiveAgentSessionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	archive_workspace_workflow_execution: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["archive_workspace_workflow_execution"],
	) => {
		const result = decode(
			UnitSchema,
			await client.archiveWorkspaceWorkflowExecution(
				fromJson(
					ArchiveWorkspaceWorkflowExecutionRequestSchema,
					clientJson(
						ArchiveWorkspaceWorkflowExecutionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	build_review_thread_handoff: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["build_review_thread_handoff"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.buildReviewThreadHandoff(
				fromJson(
					BuildReviewThreadHandoffRequestSchema,
					clientJson(
						BuildReviewThreadHandoffRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	compute_hidden_ranges_from_content: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["compute_hidden_ranges_from_content"],
	) => {
		const result = decode(
			ListHiddenRangeDtoSchema,
			await client.computeHiddenRangesFromContent(
				fromJson(
					ComputeHiddenRangesFromContentRequestSchema,
					clientJson(
						ComputeHiddenRangesFromContentRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	compute_markdown_diff_ranges: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["compute_markdown_diff_ranges"],
	) => {
		const result = decode(
			ListDiffRangeDtoSchema,
			await client.computeMarkdownDiffRanges(
				fromJson(
					ComputeMarkdownDiffRangesRequestSchema,
					clientJson(
						ComputeMarkdownDiffRangesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	compute_markdown_inline_chunks: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["compute_markdown_inline_chunks"],
	) => {
		const result = decode(
			ListInlineChunkDtoSchema,
			await client.computeMarkdownInlineChunks(
				fromJson(
					ComputeMarkdownInlineChunksRequestSchema,
					clientJson(
						ComputeMarkdownInlineChunksRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	compute_markdown_split_rows: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["compute_markdown_split_rows"],
	) => {
		const result = decode(
			ListSplitRowDtoSchema,
			await client.computeMarkdownSplitRows(
				fromJson(
					ComputeMarkdownSplitRowsRequestSchema,
					clientJson(
						ComputeMarkdownSplitRowsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	compute_visible_markdown_blocks: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["compute_visible_markdown_blocks"],
	) => {
		const result = decode(
			ListVisibleBlockDtoSchema,
			await client.computeVisibleMarkdownBlocks(
				fromJson(
					ComputeVisibleMarkdownBlocksRequestSchema,
					clientJson(
						ComputeVisibleMarkdownBlocksRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	confirm_agent_session_archive_delete: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["confirm_agent_session_archive_delete"],
	) => {
		const result = decode(
			UnitSchema,
			await client.confirmAgentSessionArchiveDelete(
				fromJson(
					ConfirmAgentSessionArchiveDeleteRequestSchema,
					clientJson(
						ConfirmAgentSessionArchiveDeleteRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	create_agent_session: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["create_agent_session"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.createAgentSession(
				fromJson(
					CreateAgentSessionRequestSchema,
					clientJson(
						CreateAgentSessionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	create_review_thread: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["create_review_thread"],
	) => {
		const result = decode(
			ReviewThreadDtoSchema,
			await client.createReviewThread(
				fromJson(
					CreateReviewThreadRequestSchema,
					clientJson(
						CreateReviewThreadRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	create_worktree: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["create_worktree"],
	) => {
		const result = decode(
			WorktreeEntryDtoSchema,
			await client.createWorktree(
				fromJson(
					CreateWorktreeRequestSchema,
					clientJson(
						CreateWorktreeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	delete_agent_session: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["delete_agent_session"],
	) => {
		const result = decode(
			UnitSchema,
			await client.deleteAgentSession(
				fromJson(
					DeleteAgentSessionRequestSchema,
					clientJson(
						DeleteAgentSessionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	delete_branch: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["delete_branch"],
	) => {
		const result = decode(
			UnitSchema,
			await client.deleteBranch(
				fromJson(
					DeleteBranchRequestSchema,
					clientJson(
						DeleteBranchRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	delete_facet: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["delete_facet"],
	) => {
		const result = decode(
			UnitSchema,
			await client.deleteFacet(
				fromJson(
					DeleteFacetRequestSchema,
					clientJson(
						DeleteFacetRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	delete_notion_config: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["delete_notion_config"],
	) => {
		const result = decode(
			UnitSchema,
			await client.deleteNotionConfig(
				fromJson(
					DeleteNotionConfigRequestSchema,
					clientJson(
						DeleteNotionConfigRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	delete_review_thread: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["delete_review_thread"],
	) => {
		const result = decode(
			UnitSchema,
			await client.deleteReviewThread(
				fromJson(
					DeleteReviewThreadRequestSchema,
					clientJson(
						DeleteReviewThreadRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	delete_workflow: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["delete_workflow"],
	) => {
		const result = decode(
			UnitSchema,
			await client.deleteWorkflow(
				fromJson(
					DeleteWorkflowRequestSchema,
					clientJson(
						DeleteWorkflowRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	detach_terminal_surface: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["detach_terminal_surface"],
	) => {
		const result = decode(
			UnitSchema,
			await client.detachTerminalSurface(
				fromJson(
					DetachTerminalSurfaceRequestSchema,
					clientJson(
						DetachTerminalSurfaceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	detect_editors: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["detect_editors"],
	) => {
		const result = decode(
			ListEditorInfoDtoSchema,
			await client.detectEditors(
				fromJson(
					DetectEditorsRequestSchema,
					clientJson(
						DetectEditorsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	diagnose_all_cmd: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["diagnose_all_cmd"],
	) => {
		const result = decode(
			DiagnosticReportSchema,
			await client.diagnoseAllCmd(
				fromJson(
					DiagnoseAllCmdRequestSchema,
					clientJson(
						DiagnoseAllCmdRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	duplicate_facet: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["duplicate_facet"],
	) => {
		const result = decode(
			UnitSchema,
			await client.duplicateFacet(
				fromJson(
					DuplicateFacetRequestSchema,
					clientJson(
						DuplicateFacetRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	duplicate_workflow: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["duplicate_workflow"],
	) => {
		const result = decode(
			UnitSchema,
			await client.duplicateWorkflow(
				fromJson(
					DuplicateWorkflowRequestSchema,
					clientJson(
						DuplicateWorkflowRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	fetch_issues: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["fetch_issues"],
	) => {
		const result = decode(
			ListIssueInfoDtoSchema,
			await client.fetchIssues(
				fromJson(
					FetchIssuesRequestSchema,
					clientJson(
						FetchIssuesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	fetch_notion_label_options: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["fetch_notion_label_options"],
	) => {
		const result = decode(
			ListNotionLabelOptionViewSchema,
			await client.fetchNotionLabelOptions(
				fromJson(
					FetchNotionLabelOptionsRequestSchema,
					clientJson(
						FetchNotionLabelOptionsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_agent_session: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_agent_session"],
	) => {
		const result = decode(
			NullableAgentSessionItemDtoSchema,
			await client.getAgentSession(
				fromJson(
					GetAgentSessionRequestSchema,
					clientJson(
						GetAgentSessionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_app_settings: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_app_settings"],
	) => {
		const result = decode(
			AppSectionSchema,
			await client.getAppSettings(
				fromJson(
					GetAppSettingsRequestSchema,
					clientJson(
						GetAppSettingsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_application_quit_operation: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_application_quit_operation"],
	) => {
		const result = decode(
			ApplicationQuitLookupDtoV1Schema,
			await client.getApplicationQuitOperation(
				fromJson(
					GetApplicationQuitOperationRequestSchema,
					clientJson(
						GetApplicationQuitOperationRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_application_shutdown: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_application_shutdown"],
	) => {
		const result = decode(
			CurrentShutdownResultDtoV1Schema,
			await client.getApplicationShutdown(
				fromJson(
					GetApplicationShutdownRequestSchema,
					clientJson(
						GetApplicationShutdownRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_application_startup_outcome: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_application_startup_outcome"],
	) => {
		const result = decode(
			ApplicationStartupOutcomeDtoV1Schema,
			await client.getApplicationStartupOutcome(
				fromJson(
					GetApplicationStartupOutcomeRequestSchema,
					clientJson(
						GetApplicationStartupOutcomeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_automation_config_dir: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_automation_config_dir"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getAutomationConfigDir(
				fromJson(
					GetAutomationConfigDirRequestSchema,
					clientJson(
						GetAutomationConfigDirRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_branch_base: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_branch_base"],
	) => {
		const result = decode(
			NullablestringSchema,
			await client.getBranchBase(
				fromJson(
					GetBranchBaseRequestSchema,
					clientJson(
						GetBranchBaseRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_cached_issues: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_cached_issues"],
	) => {
		const result = decode(
			ListIssueInfoDtoSchema,
			await client.getCachedIssues(
				fromJson(
					GetCachedIssuesRequestSchema,
					clientJson(
						GetCachedIssuesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_cached_pr_status: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_cached_pr_status"],
	) => {
		const result = decode(
			PrStatusDtoSchema,
			await client.getCachedPrStatus(
				fromJson(
					GetCachedPrStatusRequestSchema,
					clientJson(
						GetCachedPrStatusRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_current_branch: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_current_branch"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getCurrentBranch(
				fromJson(
					GetCurrentBranchRequestSchema,
					clientJson(
						GetCurrentBranchRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_cwd: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_cwd"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getCwd(
				fromJson(
					GetCwdRequestSchema,
					clientJson(
						GetCwdRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_external_editor: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_external_editor"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getExternalEditor(
				fromJson(
					GetExternalEditorRequestSchema,
					clientJson(
						GetExternalEditorRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_facet: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_facet"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getFacet(
				fromJson(
					GetFacetRequestSchema,
					clientJson(
						GetFacetRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_file_navigation: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_file_navigation"],
	) => {
		const result = decode(
			FileNavigationResultDtoSchema,
			await client.getFileNavigation(
				fromJson(
					GetFileNavigationRequestSchema,
					clientJson(
						GetFileNavigationRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_language_from_path: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_language_from_path"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getLanguageFromPath(
				fromJson(
					GetLanguageFromPathRequestSchema,
					clientJson(
						GetLanguageFromPathRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_main_repo_path: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_main_repo_path"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getMainRepoPath(
				fromJson(
					GetMainRepoPathRequestSchema,
					clientJson(
						GetMainRepoPathRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_notion_config: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_notion_config"],
	) => {
		const result = decode(
			NullableNotionRepoConfigViewSchema,
			await client.getNotionConfig(
				fromJson(
					GetNotionConfigRequestSchema,
					clientJson(
						GetNotionConfigRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_or_spawn_terminal_surface: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_or_spawn_terminal_surface"],
	) => {
		const result = decode(
			GetOrSpawnTerminalV1Schema,
			await client.getOrSpawnTerminalSurface(
				fromJson(
					GetOrSpawnTerminalSurfaceRequestSchema,
					clientJson(
						GetOrSpawnTerminalSurfaceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_performance_real_app_mode: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_performance_real_app_mode"],
	) => {
		const result = decode(
			ResultBoolSchema,
			await client.getPerformanceRealAppMode(
				fromJson(
					GetPerformanceRealAppModeRequestSchema,
					clientJson(
						GetPerformanceRealAppModeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_performance_telemetry_enabled: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_performance_telemetry_enabled"],
	) => {
		const result = decode(
			ResultBoolSchema,
			await client.getPerformanceTelemetryEnabled(
				fromJson(
					GetPerformanceTelemetryEnabledRequestSchema,
					clientJson(
						GetPerformanceTelemetryEnabledRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_provider_availability: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_provider_availability"],
	) => {
		const result = decode(
			ProviderAvailabilitySnapshotResponseSchema,
			await client.getProviderAvailability(
				fromJson(
					GetProviderAvailabilityRequestSchema,
					clientJson(
						GetProviderAvailabilityRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_releash_base: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_releash_base"],
	) => {
		const result = decode(
			NullablestringSchema,
			await client.getReleashBase(
				fromJson(
					GetReleashBaseRequestSchema,
					clientJson(
						GetReleashBaseRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_repo_paths: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_repo_paths"],
	) => {
		const result = decode(
			ListstringSchema,
			await client.getRepoPaths(
				fromJson(
					GetRepoPathsRequestSchema,
					clientJson(
						GetRepoPathsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_review_file_view: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_review_file_view"],
	) => {
		const result = decode(
			ReviewFileViewDtoSchema,
			await client.getReviewFileView(
				fromJson(
					GetReviewFileViewRequestSchema,
					clientJson(
						GetReviewFileViewRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_review_snapshot: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_review_snapshot"],
	) => {
		const result = decode(
			ReviewSnapshotDtoSchema,
			await client.getReviewSnapshot(
				fromJson(
					GetReviewSnapshotRequestSchema,
					clientJson(
						GetReviewSnapshotRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_shutdown_plan: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_shutdown_plan"],
	) => {
		const result = decode(
			ShutdownPlanPageDtoV1Schema,
			await client.getShutdownPlan(
				fromJson(
					GetShutdownPlanRequestSchema,
					clientJson(
						GetShutdownPlanRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_terminal_performance_switches: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_terminal_performance_switches"],
	) => {
		const result = decode(
			TerminalPerformanceSwitchesV1Schema,
			await client.getTerminalPerformanceSwitches(
				fromJson(
					GetTerminalPerformanceSwitchesRequestSchema,
					clientJson(
						GetTerminalPerformanceSwitchesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_terminal_surface: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_terminal_surface"],
	) => {
		const result = decode(
			TerminalSurfaceSummaryV1Schema,
			await client.getTerminalSurface(
				fromJson(
					GetTerminalSurfaceRequestSchema,
					clientJson(
						GetTerminalSurfaceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workflow: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workflow"],
	) => {
		const result = decode(
			WorkflowDtoSchema,
			await client.getWorkflow(
				fromJson(
					GetWorkflowRequestSchema,
					clientJson(
						GetWorkflowRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workflow_config: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workflow_config"],
	) => {
		const result = decode(
			WorkflowSectionSchema,
			await client.getWorkflowConfig(
				fromJson(
					GetWorkflowConfigRequestSchema,
					clientJson(
						GetWorkflowConfigRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workflow_execution_state: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workflow_execution_state"],
	) => {
		const result = decode(
			NullableWorkflowExecutionViewSchema,
			await client.getWorkflowExecutionState(
				fromJson(
					GetWorkflowExecutionStateRequestSchema,
					clientJson(
						GetWorkflowExecutionStateRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workflow_source: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workflow_source"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getWorkflowSource(
				fromJson(
					GetWorkflowSourceRequestSchema,
					clientJson(
						GetWorkflowSourceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workspace_node_detail: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workspace_node_detail"],
	) => {
		const result = decode(
			NullableWorkspaceNodeDetailDtoSchema,
			await client.getWorkspaceNodeDetail(
				fromJson(
					GetWorkspaceNodeDetailRequestSchema,
					clientJson(
						GetWorkspaceNodeDetailRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workspace_session_node_id: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workspace_session_node_id"],
	) => {
		const result = decode(
			NullablestringSchema,
			await client.getWorkspaceSessionNodeId(
				fromJson(
					GetWorkspaceSessionNodeIdRequestSchema,
					clientJson(
						GetWorkspaceSessionNodeIdRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workspace_tree_selection_reconciliation: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workspace_tree_selection_reconciliation"],
	) => {
		const result = decode(
			WorkspaceTreeSelectionSnapshotDtoSchema,
			await client.getWorkspaceTreeSelectionReconciliation(
				fromJson(
					GetWorkspaceTreeSelectionReconciliationRequestSchema,
					clientJson(
						GetWorkspaceTreeSelectionReconciliationRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	git_create_branch: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["git_create_branch"],
	) => {
		const result = decode(
			UnitSchema,
			await client.gitCreateBranch(
				fromJson(
					GitCreateBranchRequestSchema,
					clientJson(
						GitCreateBranchRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	git_stage: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["git_stage"],
	) => {
		const result = decode(
			UnitSchema,
			await client.gitStage(
				fromJson(
					GitStageRequestSchema,
					clientJson(
						GitStageRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	git_stage_review_group: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["git_stage_review_group"],
	) => {
		const result = decode(
			UnitSchema,
			await client.gitStageReviewGroup(
				fromJson(
					GitStageReviewGroupRequestSchema,
					clientJson(
						GitStageReviewGroupRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	git_unstage: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["git_unstage"],
	) => {
		const result = decode(
			UnitSchema,
			await client.gitUnstage(
				fromJson(
					GitUnstageRequestSchema,
					clientJson(
						GitUnstageRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	git_unstage_review_group: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["git_unstage_review_group"],
	) => {
		const result = decode(
			UnitSchema,
			await client.gitUnstageReviewGroup(
				fromJson(
					GitUnstageReviewGroupRequestSchema,
					clientJson(
						GitUnstageReviewGroupRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	kill_terminal_surface: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["kill_terminal_surface"],
	) => {
		const result = decode(
			UnitSchema,
			await client.killTerminalSurface(
				fromJson(
					KillTerminalSurfaceRequestSchema,
					clientJson(
						KillTerminalSurfaceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_agent_session_history: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_agent_session_history"],
	) => {
		const result = decode(
			AgentSessionHistoryPageDtoSchema,
			await client.listAgentSessionHistory(
				fromJson(
					ListAgentSessionHistoryRequestSchema,
					clientJson(
						ListAgentSessionHistoryRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_available_agent_session_providers: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_available_agent_session_providers"],
	) => {
		const result = decode(
			ListAgentSessionProviderDtoSchema,
			await client.listAvailableAgentSessionProviders(
				fromJson(
					ListAvailableAgentSessionProvidersRequestSchema,
					clientJson(
						ListAvailableAgentSessionProvidersRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_branches: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_branches"],
	) => {
		const result = decode(
			ListBranchDtoSchema,
			await client.listBranches(
				fromJson(
					ListBranchesRequestSchema,
					clientJson(
						ListBranchesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_branches_with_status: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_branches_with_status"],
	) => {
		const result = decode(
			ListBranchCardDtoSchema,
			await client.listBranchesWithStatus(
				fromJson(
					ListBranchesWithStatusRequestSchema,
					clientJson(
						ListBranchesWithStatusRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_branches_with_status_snapshot: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_branches_with_status_snapshot"],
	) => {
		const result = decode(
			RepositoryBranchCardsSnapshotDtoSchema,
			await client.listBranchesWithStatusSnapshot(
				fromJson(
					ListBranchesWithStatusSnapshotRequestSchema,
					clientJson(
						ListBranchesWithStatusSnapshotRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_facet_summaries: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_facet_summaries"],
	) => {
		const result = decode(
			ListFacetSummaryDtoSchema,
			await client.listFacetSummaries(
				fromJson(
					ListFacetSummariesRequestSchema,
					clientJson(
						ListFacetSummariesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_pending_application_attempts: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_pending_application_attempts"],
	) => {
		const result = decode(
			PendingCallerAttemptPageDtoV1Schema,
			await client.listPendingApplicationAttempts(
				fromJson(
					ListPendingApplicationAttemptsRequestSchema,
					clientJson(
						ListPendingApplicationAttemptsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_provider_hook_health_warnings: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_provider_hook_health_warnings"],
	) => {
		const result = decode(
			ListProviderHookHealthWarningResponseSchema,
			await client.listProviderHookHealthWarnings(
				fromJson(
					ListProviderHookHealthWarningsRequestSchema,
					clientJson(
						ListProviderHookHealthWarningsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_review_threads: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_review_threads"],
	) => {
		const result = decode(
			ListReviewThreadDtoSchema,
			await client.listReviewThreads(
				fromJson(
					ListReviewThreadsRequestSchema,
					clientJson(
						ListReviewThreadsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_workflows: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_workflows"],
	) => {
		const result = decode(
			ListWorkflowSummaryDtoSchema,
			await client.listWorkflows(
				fromJson(
					ListWorkflowsRequestSchema,
					clientJson(
						ListWorkflowsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_workspace_workflow_history: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_workspace_workflow_history"],
	) => {
		const result = decode(
			ListWorkspaceWorkflowHistoryItemDtoSchema,
			await client.listWorkspaceWorkflowHistory(
				fromJson(
					ListWorkspaceWorkflowHistoryRequestSchema,
					clientJson(
						ListWorkspaceWorkflowHistoryRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_workspace_worktree_nodes: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_workspace_worktree_nodes"],
	) => {
		const result = decode(
			WorkspaceTreeSnapshotDtoSchema,
			await client.listWorkspaceWorktreeNodes(
				fromJson(
					ListWorkspaceWorktreeNodesRequestSchema,
					clientJson(
						ListWorkspaceWorktreeNodesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_worktrees: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_worktrees"],
	) => {
		const result = decode(
			ListWorktreeEntryDtoSchema,
			await client.listWorktrees(
				fromJson(
					ListWorktreesRequestSchema,
					clientJson(
						ListWorktreesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	load_workspace_state: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["load_workspace_state"],
	) => {
		const result = decode(
			NullableWorkspaceStateDtoSchema,
			await client.loadWorkspaceState(
				fromJson(
					LoadWorkspaceStateRequestSchema,
					clientJson(
						LoadWorkspaceStateRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	open_agent_session: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["open_agent_session"],
	) => {
		const result = decode(
			AgentSessionOpenResponseSchema,
			await client.openAgentSession(
				fromJson(
					OpenAgentSessionRequestSchema,
					clientJson(
						OpenAgentSessionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	open_facet_in_editor: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["open_facet_in_editor"],
	) => {
		const result = decode(
			UnitSchema,
			await client.openFacetInEditor(
				fromJson(
					OpenFacetInEditorRequestSchema,
					clientJson(
						OpenFacetInEditorRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	open_folder_in_editor: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["open_folder_in_editor"],
	) => {
		const result = decode(
			UnitSchema,
			await client.openFolderInEditor(
				fromJson(
					OpenFolderInEditorRequestSchema,
					clientJson(
						OpenFolderInEditorRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	open_in_editor: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["open_in_editor"],
	) => {
		const result = decode(
			UnitSchema,
			await client.openInEditor(
				fromJson(
					OpenInEditorRequestSchema,
					clientJson(
						OpenInEditorRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	open_workflow_in_editor: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["open_workflow_in_editor"],
	) => {
		const result = decode(
			UnitSchema,
			await client.openWorkflowInEditor(
				fromJson(
					OpenWorkflowInEditorRequestSchema,
					clientJson(
						OpenWorkflowInEditorRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	query_notion_tasks: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["query_notion_tasks"],
	) => {
		const result = decode(
			NotionTaskPageViewSchema,
			await client.queryNotionTasks(
				fromJson(
					QueryNotionTasksRequestSchema,
					clientJson(
						QueryNotionTasksRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	quit_after_startup_failure: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["quit_after_startup_failure"],
	) => {
		const result = decode(
			StartupFailureQuitOutcomeDtoV1Schema,
			await client.quitAfterStartupFailure(
				fromJson(
					QuitAfterStartupFailureRequestSchema,
					clientJson(
						QuitAfterStartupFailureRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	record_terminal_launch_renderer_phase: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["record_terminal_launch_renderer_phase"],
	) => {
		const result = decode(
			UnitSchema,
			await client.recordTerminalLaunchRendererPhase(
				fromJson(
					RecordTerminalLaunchRendererPhaseRequestSchema,
					clientJson(
						RecordTerminalLaunchRendererPhaseRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	refresh_provider_availability: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["refresh_provider_availability"],
	) => {
		const result = decode(
			ProviderAvailabilitySnapshotResponseSchema,
			await client.refreshProviderAvailability(
				fromJson(
					RefreshProviderAvailabilityRequestSchema,
					clientJson(
						RefreshProviderAvailabilityRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	remove_repo_path: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["remove_repo_path"],
	) => {
		const result = decode(
			ResultBoolSchema,
			await client.removeRepoPath(
				fromJson(
					RemoveRepoPathRequestSchema,
					clientJson(
						RemoveRepoPathRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	remove_worktree: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["remove_worktree"],
	) => {
		const result = decode(
			UnitSchema,
			await client.removeWorktree(
				fromJson(
					RemoveWorktreeRequestSchema,
					clientJson(
						RemoveWorktreeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	rename_workspace_session_node: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["rename_workspace_session_node"],
	) => {
		const result = decode(
			UnitSchema,
			await client.renameWorkspaceSessionNode(
				fromJson(
					RenameWorkspaceSessionNodeRequestSchema,
					clientJson(
						RenameWorkspaceSessionNodeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	render_facet_preview: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["render_facet_preview"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.renderFacetPreview(
				fromJson(
					RenderFacetPreviewRequestSchema,
					clientJson(
						RenderFacetPreviewRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	report_frontend_error: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["report_frontend_error"],
	) => {
		const result = decode(
			UnitSchema,
			await client.reportFrontendError(
				fromJson(
					ReportFrontendErrorRequestSchema,
					clientJson(
						ReportFrontendErrorRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	report_mounted_xterm_count: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["report_mounted_xterm_count"],
	) => {
		const result = decode(
			UnitSchema,
			await client.reportMountedXtermCount(
				fromJson(
					ReportMountedXtermCountRequestSchema,
					clientJson(
						ReportMountedXtermCountRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	report_usage_event: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["report_usage_event"],
	) => {
		const result = decode(
			UnitSchema,
			await client.reportUsageEvent(
				fromJson(
					ReportUsageEventRequestSchema,
					clientJson(
						ReportUsageEventRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	request_application_quit: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["request_application_quit"],
	) => {
		const result = decode(
			ApplicationQuitOutcomeDtoV1Schema,
			await client.requestApplicationQuit(
				fromJson(
					RequestApplicationQuitRequestSchema,
					clientJson(
						RequestApplicationQuitRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	reset_provider_executable: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["reset_provider_executable"],
	) => {
		const result = decode(
			ProviderAvailabilitySnapshotResponseSchema,
			await client.resetProviderExecutable(
				fromJson(
					ResetProviderExecutableRequestSchema,
					clientJson(
						ResetProviderExecutableRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	resize_terminal_surface: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["resize_terminal_surface"],
	) => {
		const result = decode(
			UnitSchema,
			await client.resizeTerminalSurface(
				fromJson(
					ResizeTerminalSurfaceRequestSchema,
					clientJson(
						ResizeTerminalSurfaceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	resolve_active_execution_by_worktree: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["resolve_active_execution_by_worktree"],
	) => {
		const result = decode(
			NullablestringSchema,
			await client.resolveActiveExecutionByWorktree(
				fromJson(
					ResolveActiveExecutionByWorktreeRequestSchema,
					clientJson(
						ResolveActiveExecutionByWorktreeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	resolve_review_thread: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["resolve_review_thread"],
	) => {
		const result = decode(
			ReviewThreadDtoSchema,
			await client.resolveReviewThread(
				fromJson(
					ResolveReviewThreadRequestSchema,
					clientJson(
						ResolveReviewThreadRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	resolve_shutdown_target_action: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["resolve_shutdown_target_action"],
	) => {
		const result = decode(
			RecoveryActionOutcomeDtoV1Schema,
			await client.resolveShutdownTargetAction(
				fromJson(
					ResolveShutdownTargetActionRequestSchema,
					clientJson(
						ResolveShutdownTargetActionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	restore_agent_session: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["restore_agent_session"],
	) => {
		const result = decode(
			AgentSessionOpenResponseSchema,
			await client.restoreAgentSession(
				fromJson(
					RestoreAgentSessionRequestSchema,
					clientJson(
						RestoreAgentSessionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	restore_workspace_workflow_execution: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["restore_workspace_workflow_execution"],
	) => {
		const result = decode(
			UnitSchema,
			await client.restoreWorkspaceWorkflowExecution(
				fromJson(
					RestoreWorkspaceWorkflowExecutionRequestSchema,
					clientJson(
						RestoreWorkspaceWorkflowExecutionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	resume_agent_session: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["resume_agent_session"],
	) => {
		const result = decode(
			AgentSessionOpenResponseSchema,
			await client.resumeAgentSession(
				fromJson(
					ResumeAgentSessionRequestSchema,
					clientJson(
						ResumeAgentSessionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	resume_agent_session_history_candidate: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["resume_agent_session_history_candidate"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.resumeAgentSessionHistoryCandidate(
				fromJson(
					ResumeAgentSessionHistoryCandidateRequestSchema,
					clientJson(
						ResumeAgentSessionHistoryCandidateRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	resume_workflow: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["resume_workflow"],
	) => {
		const result = decode(
			UnitSchema,
			await client.resumeWorkflow(
				fromJson(
					ResumeWorkflowRequestSchema,
					clientJson(
						ResumeWorkflowRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	retry_workspace_node: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["retry_workspace_node"],
	) => {
		const result = decode(
			UnitSchema,
			await client.retryWorkspaceNode(
				fromJson(
					RetryWorkspaceNodeRequestSchema,
					clientJson(
						RetryWorkspaceNodeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	save_facet: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["save_facet"],
	) => {
		const result = decode(
			UnitSchema,
			await client.saveFacet(
				fromJson(
					SaveFacetRequestSchema,
					clientJson(
						SaveFacetRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	save_notion_config: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["save_notion_config"],
	) => {
		const result = decode(
			UnitSchema,
			await client.saveNotionConfig(
				fromJson(
					SaveNotionConfigRequestSchema,
					clientJson(
						SaveNotionConfigRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	save_workflow_source: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["save_workflow_source"],
	) => {
		const result = decode(
			SaveWorkflowSourceResultDtoSchema,
			await client.saveWorkflowSource(
				fromJson(
					SaveWorkflowSourceRequestSchema,
					clientJson(
						SaveWorkflowSourceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	save_workspace_state: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["save_workspace_state"],
	) => {
		const result = decode(
			UnitSchema,
			await client.saveWorkspaceState(
				fromJson(
					SaveWorkspaceStateRequestSchema,
					clientJson(
						SaveWorkspaceStateRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	set_branch_base: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["set_branch_base"],
	) => {
		const result = decode(
			UnitSchema,
			await client.setBranchBase(
				fromJson(
					SetBranchBaseRequestSchema,
					clientJson(
						SetBranchBaseRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	set_releash_base: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["set_releash_base"],
	) => {
		const result = decode(
			UnitSchema,
			await client.setReleashBase(
				fromJson(
					SetReleashBaseRequestSchema,
					clientJson(
						SetReleashBaseRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	start_terminal_input_performance_collection: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["start_terminal_input_performance_collection"],
	) => {
		const result = decode(
			UnitSchema,
			await client.startTerminalInputPerformanceCollection(
				fromJson(
					StartTerminalInputPerformanceCollectionRequestSchema,
					clientJson(
						StartTerminalInputPerformanceCollectionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	start_terminal_launch_performance_collection: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["start_terminal_launch_performance_collection"],
	) => {
		const result = decode(
			UnitSchema,
			await client.startTerminalLaunchPerformanceCollection(
				fromJson(
					StartTerminalLaunchPerformanceCollectionRequestSchema,
					clientJson(
						StartTerminalLaunchPerformanceCollectionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	start_workflow: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["start_workflow"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.startWorkflow(
				fromJson(
					StartWorkflowRequestSchema,
					clientJson(
						StartWorkflowRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	stop_watching: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["stop_watching"],
	) => {
		const result = decode(
			UnitSchema,
			await client.stopWatching(
				fromJson(
					StopWatchingRequestSchema,
					clientJson(
						StopWatchingRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	stop_workflow: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["stop_workflow"],
	) => {
		const result = decode(
			UnitSchema,
			await client.stopWorkflow(
				fromJson(
					StopWorkflowRequestSchema,
					clientJson(
						StopWorkflowRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	take_terminal_input_performance_samples: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["take_terminal_input_performance_samples"],
	) => {
		const result = decode(
			ListTerminalInputPerformanceSampleV1Schema,
			await client.takeTerminalInputPerformanceSamples(
				fromJson(
					TakeTerminalInputPerformanceSamplesRequestSchema,
					clientJson(
						TakeTerminalInputPerformanceSamplesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	take_terminal_launch_performance_samples: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["take_terminal_launch_performance_samples"],
	) => {
		const result = decode(
			ListTerminalLaunchPerformanceSampleV1Schema,
			await client.takeTerminalLaunchPerformanceSamples(
				fromJson(
					TakeTerminalLaunchPerformanceSamplesRequestSchema,
					clientJson(
						TakeTerminalLaunchPerformanceSamplesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	update_app_settings: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["update_app_settings"],
	) => {
		const result = decode(
			UnitSchema,
			await client.updateAppSettings(
				fromJson(
					UpdateAppSettingsRequestSchema,
					clientJson(
						UpdateAppSettingsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	update_login_item_preference: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["update_login_item_preference"],
	) => {
		const result = decode(
			UnitSchema,
			await client.updateLoginItemPreference(
				fromJson(
					UpdateLoginItemPreferenceRequestSchema,
					clientJson(
						UpdateLoginItemPreferenceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	update_crash_reporting: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["update_crash_reporting"],
	) => {
		const result = decode(
			UnitSchema,
			await client.updateCrashReporting(
				fromJson(
					UpdateCrashReportingRequestSchema,
					clientJson(
						UpdateCrashReportingRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	update_external_editor: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["update_external_editor"],
	) => {
		const result = decode(
			UnitSchema,
			await client.updateExternalEditor(
				fromJson(
					UpdateExternalEditorRequestSchema,
					clientJson(
						UpdateExternalEditorRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	update_performance_telemetry: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["update_performance_telemetry"],
	) => {
		const result = decode(
			UnitSchema,
			await client.updatePerformanceTelemetry(
				fromJson(
					UpdatePerformanceTelemetryRequestSchema,
					clientJson(
						UpdatePerformanceTelemetryRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	update_provider_executable: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["update_provider_executable"],
	) => {
		const result = decode(
			ProviderAvailabilitySnapshotResponseSchema,
			await client.updateProviderExecutable(
				fromJson(
					UpdateProviderExecutableRequestSchema,
					clientJson(
						UpdateProviderExecutableRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	update_workflow_config: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["update_workflow_config"],
	) => {
		const result = decode(
			UnitSchema,
			await client.updateWorkflowConfig(
				fromJson(
					UpdateWorkflowConfigRequestSchema,
					clientJson(
						UpdateWorkflowConfigRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	validate_notion_config: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["validate_notion_config"],
	) => {
		const result = decode(
			NotionValidationResultViewSchema,
			await client.validateNotionConfig(
				fromJson(
					ValidateNotionConfigRequestSchema,
					clientJson(
						ValidateNotionConfigRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	write_paths_to_terminal_surface: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["write_paths_to_terminal_surface"],
	) => {
		const result = decode(
			UnitSchema,
			await client.writePathsToTerminalSurface(
				fromJson(
					WritePathsToTerminalSurfaceRequestSchema,
					clientJson(
						WritePathsToTerminalSurfaceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	write_terminal_surface: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["write_terminal_surface"],
	) => {
		const result = decode(
			UnitSchema,
			await client.writeTerminalSurface(
				fromJson(
					WriteTerminalSurfaceRequestSchema,
					clientJson(
						WriteTerminalSurfaceRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_crash_reporting_enabled: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_crash_reporting_enabled"],
	) => {
		const result = decode(
			ResultBoolSchema,
			await client.getCrashReportingEnabled(
				fromJson(
					GetCrashReportingEnabledRequestSchema,
					clientJson(
						GetCrashReportingEnabledRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_file_at_ref: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_file_at_ref"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getFileAtRef(
				fromJson(
					GetFileAtRefRequestSchema,
					clientJson(
						GetFileAtRefRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_staged_content: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_staged_content"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getStagedContent(
				fromJson(
					GetStagedContentRequestSchema,
					clientJson(
						GetStagedContentRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_binary_staged_content: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_binary_staged_content"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getBinaryStagedContent(
				fromJson(
					GetBinaryStagedContentRequestSchema,
					clientJson(
						GetBinaryStagedContentRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_file_at_branch_base: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_file_at_branch_base"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getFileAtBranchBase(
				fromJson(
					GetFileAtBranchBaseRequestSchema,
					clientJson(
						GetFileAtBranchBaseRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_binary_file_at_branch_base: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_binary_file_at_branch_base"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getBinaryFileAtBranchBase(
				fromJson(
					GetBinaryFileAtBranchBaseRequestSchema,
					clientJson(
						GetBinaryFileAtBranchBaseRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_binary_file_at_ref: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_binary_file_at_ref"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getBinaryFileAtRef(
				fromJson(
					GetBinaryFileAtRefRequestSchema,
					clientJson(
						GetBinaryFileAtRefRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_branch_diff_summary: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_branch_diff_summary"],
	) => {
		const result = decode(
			BranchDiffSummaryDtoSchema,
			await client.getBranchDiffSummary(
				fromJson(
					GetBranchDiffSummaryRequestSchema,
					clientJson(
						GetBranchDiffSummaryRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	build_diff_file_tree: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["build_diff_file_tree"],
	) => {
		const result = decode(
			ListDiffTreeNodeDtoSchema,
			await client.buildDiffFileTree(
				fromJson(
					BuildDiffFileTreeRequestSchema,
					clientJson(
						BuildDiffFileTreeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_head_diff_file_tree_snapshot: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_head_diff_file_tree_snapshot"],
	) => {
		const result = decode(
			RepositoryHeadDiffFileTreeSnapshotDtoSchema,
			await client.getHeadDiffFileTreeSnapshot(
				fromJson(
					GetHeadDiffFileTreeSnapshotRequestSchema,
					clientJson(
						GetHeadDiffFileTreeSnapshotRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	compute_hidden_ranges: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["compute_hidden_ranges"],
	) => {
		const result = decode(
			ListHiddenRangeDtoSchema,
			await client.computeHiddenRanges(
				fromJson(
					ComputeHiddenRangesRequestSchema,
					clientJson(
						ComputeHiddenRangesRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_relative_path: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_relative_path"],
	) => {
		const result = decode(
			NullablestringSchema,
			await client.getRelativePath(
				fromJson(
					GetRelativePathRequestSchema,
					clientJson(
						GetRelativePathRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_review_thread: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_review_thread"],
	) => {
		const result = decode(
			ReviewThreadDtoSchema,
			await client.getReviewThread(
				fromJson(
					GetReviewThreadRequestSchema,
					clientJson(
						GetReviewThreadRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_review_thread_history: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_review_thread_history"],
	) => {
		const result = decode(
			ListReviewHistoryEntryDtoSchema,
			await client.getReviewThreadHistory(
				fromJson(
					GetReviewThreadHistoryRequestSchema,
					clientJson(
						GetReviewThreadHistoryRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	fetch_pr_status: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["fetch_pr_status"],
	) => {
		const result = decode(
			PrStatusDtoSchema,
			await client.fetchPrStatus(
				fromJson(
					FetchPrStatusRequestSchema,
					clientJson(
						FetchPrStatusRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_default_branch: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_default_branch"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getDefaultBranch(
				fromJson(
					GetDefaultBranchRequestSchema,
					clientJson(
						GetDefaultBranchRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_git_status: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_git_status"],
	) => {
		const result = decode(
			ListFileStatusDtoSchema,
			await client.getGitStatus(
				fromJson(
					GetGitStatusRequestSchema,
					clientJson(
						GetGitStatusRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_git_status_snapshot: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_git_status_snapshot"],
	) => {
		const result = decode(
			RepositoryStatusSnapshotDtoSchema,
			await client.getGitStatusSnapshot(
				fromJson(
					GetGitStatusSnapshotRequestSchema,
					clientJson(
						GetGitStatusSnapshotRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_status_diff_stats: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_status_diff_stats"],
	) => {
		const result = decode(
			ListFileDiffStatDtoSchema,
			await client.getStatusDiffStats(
				fromJson(
					GetStatusDiffStatsRequestSchema,
					clientJson(
						GetStatusDiffStatsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_status_diff_stats_snapshot: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_status_diff_stats_snapshot"],
	) => {
		const result = decode(
			RepositoryDiffStatsSnapshotDtoSchema,
			await client.getStatusDiffStatsSnapshot(
				fromJson(
					GetStatusDiffStatsSnapshotRequestSchema,
					clientJson(
						GetStatusDiffStatsSnapshotRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_git_log: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_git_log"],
	) => {
		const result = decode(
			ListCommitDtoSchema,
			await client.getGitLog(
				fromJson(
					GetGitLogRequestSchema,
					clientJson(
						GetGitLogRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_worktree_dirty_count: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_worktree_dirty_count"],
	) => {
		const result = decode(
			ResultUint32Schema,
			await client.getWorktreeDirtyCount(
				fromJson(
					GetWorktreeDirtyCountRequestSchema,
					clientJson(
						GetWorktreeDirtyCountRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_repo_git_dir: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_repo_git_dir"],
	) => {
		const result = decode(
			ResultStringSchema,
			await client.getRepoGitDir(
				fromJson(
					GetRepoGitDirRequestSchema,
					clientJson(
						GetRepoGitDirRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	approve_workflow_node: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["approve_workflow_node"],
	) => {
		const result = decode(
			UnitSchema,
			await client.approveWorkflowNode(
				fromJson(
					ApproveWorkflowNodeRequestSchema,
					clientJson(
						ApproveWorkflowNodeRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_workflow_executions: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_workflow_executions"],
	) => {
		const result = decode(
			ListWorkflowExecutionSummaryDtoSchema,
			await client.listWorkflowExecutions(
				fromJson(
					ListWorkflowExecutionsRequestSchema,
					clientJson(
						ListWorkflowExecutionsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workflow_execution: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workflow_execution"],
	) => {
		const result = decode(
			NullableWorkflowExecutionSummaryDtoSchema,
			await client.getWorkflowExecution(
				fromJson(
					GetWorkflowExecutionRequestSchema,
					clientJson(
						GetWorkflowExecutionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workflow_execution_log: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workflow_execution_log"],
	) => {
		const result = decode(
			NullableListDurableWorkflowFactLogEntrySchema,
			await client.getWorkflowExecutionLog(
				fromJson(
					GetWorkflowExecutionLogRequestSchema,
					clientJson(
						GetWorkflowExecutionLogRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	get_workflow_node_detail: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["get_workflow_node_detail"],
	) => {
		const result = decode(
			NullableNodeExecutionViewSchema,
			await client.getWorkflowNodeDetail(
				fromJson(
					GetWorkflowNodeDetailRequestSchema,
					clientJson(
						GetWorkflowNodeDetailRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	resolve_worktree_by_execution: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["resolve_worktree_by_execution"],
	) => {
		const result = decode(
			NullablestringSchema,
			await client.resolveWorktreeByExecution(
				fromJson(
					ResolveWorktreeByExecutionRequestSchema,
					clientJson(
						ResolveWorktreeByExecutionRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	list_facets: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["list_facets"],
	) => {
		const result = decode(
			ListstringSchema,
			await client.listFacets(
				fromJson(
					ListFacetsRequestSchema,
					clientJson(
						ListFacetsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	workflow_submit_output: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["workflow_submit_output"],
	) => {
		const result = decode(
			UnitSchema,
			await client.workflowSubmitOutput(
				fromJson(
					WorkflowSubmitOutputRequestSchema,
					clientJson(
						WorkflowSubmitOutputRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	workflow_validate_output: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["workflow_validate_output"],
	) => {
		const result = decode(
			WorkflowValidateOutputResponseSchema,
			await client.workflowValidateOutput(
				fromJson(
					WorkflowValidateOutputRequestSchema,
					clientJson(
						WorkflowValidateOutputRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	workflow_get_output: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["workflow_get_output"],
	) => {
		const result = decode(
			WorkflowGetOutputResponseSchema,
			await client.workflowGetOutput(
				fromJson(
					WorkflowGetOutputRequestSchema,
					clientJson(
						WorkflowGetOutputRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
	compact_application_shutdown_details: async (
		client: Client<typeof ClientService>,
		args: ClientCommandArgs["compact_application_shutdown_details"],
	) => {
		const result = decode(
			ShutdownPlanDtoV1Schema,
			await client.compactApplicationShutdownDetails(
				fromJson(
					CompactApplicationShutdownDetailsRequestSchema,
					clientJson(
						CompactApplicationShutdownDetailsRequestSchema,
						JSON.parse(JSON.stringify(args ?? {})),
						true,
					),
				),
			),
		);
		return result;
	},
};
export type ClientCommand = keyof typeof commands;
export type EmptyClientCommand = {
	[K in ClientCommand]: ClientCommandArgs[K] extends Record<string, never>
		? K
		: never;
}[ClientCommand];
export async function invokeClient<K extends ClientCommand>(
	command: K,
	args?: ClientCommandArgs[K],
): Promise<ClientCommandResults[K]> {
	const client = await getClient();
	try {
		return (await (
			commands[command] as (
				client: Client<typeof ClientService>,
				args: ClientCommandArgs[K],
			) => Promise<unknown>
		)(client, args ?? ({} as ClientCommandArgs[K]))) as ClientCommandResults[K];
	} catch (error) {
		if (error instanceof ConnectError) {
			const detail = error.findDetails(CommandErrorSchema)[0];
			if (detail) throw decode(CommandErrorSchema, detail);
			refreshClientOnDisconnect(client, error);
		}
		throw error;
	}
}
