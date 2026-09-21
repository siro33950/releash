// Generated from proto/client.proto. Run pnpm generate:protocol.

export type InputAttachTerminalSurfaceRequest = {
	attachmentId: string;
	owner: InputTerminalSurfaceOwnerV1;
	recovery: boolean;
};

export type InputTerminalSurfaceOwnerV1 =
	| ({ kind: "workspace" } & InputTerminalSurfaceOwnerV1Workspace)
	| ({ kind: "session" } & InputTerminalSurfaceOwnerV1Session);

export type InputTerminalSurfaceOwnerV1Workspace = {
	workspacePath: string;
};

export type InputTerminalSurfaceOwnerV1Session = {
	workspacePath: string;
	sessionId: string;
};

export type InputGetReviewBlobRequest = {
	reference: string;
};

export type InputAbortWorkflowRequest = {
	executionId: string;
};

export type InputAckTerminalSurfaceOutputRequest = {
	attachmentId: string;
	sequence: number;
};

export type InputAddRepoPathRequest = {
	path: string;
};

export type InputAppendReviewCommentRequest = {
	worktreeName: string;
	threadId: string;
	content: string;
};

export type InputApproveWorkspaceNodeRequest = {
	worktreePath: string;
	nodeId: string;
};

export type InputArchiveAgentSessionRequest = {
	agentSessionId: string;
	callerRequestId: string;
};

export type InputArchiveWorkspaceWorkflowExecutionRequest = {
	worktreePath: string;
	executionId: string;
};

export type InputBuildReviewThreadHandoffRequest = {
	worktreeName: string;
	threadId: string;
};

export type InputComputeHiddenRangesFromContentRequest = {
	original: string;
	modified: string;
	contextLines: number;
};

export type InputComputeMarkdownDiffRangesRequest = {
	original: string;
	modified: string;
	side: InputMarkdownDiffSideInput;
};

export type InputMarkdownDiffSideInput = "modified" | "original";

export type InputComputeMarkdownInlineChunksRequest = {
	original: string;
	modified: string;
};

export type InputComputeMarkdownSplitRowsRequest = {
	original: string;
	modified: string;
};

export type InputComputeVisibleMarkdownBlocksRequest = {
	original: string;
	modified: string;
	contextLines: number;
};

export type InputConfirmAgentSessionArchiveDeleteRequest = {
	agentSessionId: string;
	callerRequestId: string;
};

export type InputCreateAgentSessionRequest = {
	workspaceIdentity: string;
	worktreePath: string;
	provider: string;
	rows: number;
	cols: number;
	callerRequestId: string;
};

export type InputCreateReviewThreadRequest = {
	worktreeName: string;
	filePath?: string | null;
	lineNumber?: number | null;
	endLine?: number | null;
	content: string;
};

export type InputCreateWorktreeRequest = {
	repoPath: string;
	branch: string;
	createBranch: boolean;
	baseBranch?: string | null;
};

export type InputDeleteAgentSessionRequest = {
	agentSessionId: string;
	callerRequestId: string;
};

export type InputDeleteBranchRequest = {
	repoPath: string;
	branchName: string;
	force: boolean;
};

export type InputDeleteFacetRequest = {
	kind: string;
	key: string;
};

export type InputDeleteNotionConfigRequest = {
	repoPath: string;
};

export type InputDeleteReviewThreadRequest = {
	worktreeName: string;
	threadId: string;
};

export type InputDeleteWorkflowRequest = {
	name: string;
};

export type InputDetachTerminalSurfaceRequest = {
	attachmentId: string;
};

export type InputDetectEditorsRequest = Record<string, never>;

export type InputDiagnoseAllCmdRequest = {
	dir?: string | null;
};

export type InputDuplicateFacetRequest = {
	kind: string;
	sourceKey: string;
	newKey: string;
};

export type InputDuplicateWorkflowRequest = {
	sourceName: string;
	newName: string;
};

export type InputFetchIssuesRequest = {
	repoPath: string;
};

export type InputFetchNotionLabelOptionsRequest = {
	repoPath: string;
};

export type InputGetAgentSessionRequest = {
	agentSessionId: string;
};

export type InputGetAppSettingsRequest = Record<string, never>;

export type InputGetApplicationStartupOutcomeRequest = Record<string, never>;

export type InputGetAutomationConfigDirRequest = Record<string, never>;

export type InputGetBranchBaseRequest = {
	repoPath: string;
	branchName: string;
};

export type InputGetCachedIssuesRequest = {
	repoPath: string;
};

export type InputGetCachedPrStatusRequest = {
	repoPath: string;
};

export type InputGetCurrentBranchRequest = {
	repoPath: string;
};

export type InputGetCwdRequest = Record<string, never>;

export type InputGetExternalEditorRequest = Record<string, never>;

export type InputGetFacetRequest = {
	kind: string;
	key: string;
};

export type InputGetFileNavigationRequest = {
	tree: InputListDiffTreeNodeInput;
	currentFile: string;
};

export type InputListDiffTreeNodeInput = Array<InputDiffTreeNodeInput>;

export type InputDiffTreeNodeInput = {
	id: string;
	name: string;
	path: string;
	node_type: InputDiffTreeNodeType;
	status?: string | null;
	additions?: number | null;
	deletions?: number | null;
	children: InputListDiffTreeNodeInput;
};

export type InputDiffTreeNodeType = "file" | "folder";

export type InputGetLanguageFromPathRequest = {
	filePath: string;
};

export type InputGetMainRepoPathRequest = {
	anyPath: string;
};

export type InputGetNotionConfigRequest = {
	repoPath: string;
};

export type InputGetOrSpawnTerminalSurfaceRequest = {
	rows: number;
	cols: number;
	cwd?: string | null;
	owner: InputTerminalSurfaceOwnerV1;
	label?: string | null;
	startupCommand?: string | null;
};

export type InputGetPerformanceRealAppModeRequest = Record<string, never>;

export type InputGetPerformanceTelemetryEnabledRequest = Record<string, never>;

export type InputGetProviderAvailabilityRequest = Record<string, never>;

export type InputGetReleashBaseRequest = {
	repoPath: string;
};

export type InputGetRepoPathsRequest = Record<string, never>;

export type InputGetReviewFileViewRequest = {
	input: InputReviewFileViewInput;
};

export type InputReviewFileViewInput = {
	worktreePath: string;
	target: InputReviewTargetInput;
	section: string;
	base: string;
	snapshotVersion?: number | null;
	viewport?: InputViewportInput | null;
};

export type InputReviewTargetInput =
	| { by: "fileId"; value: string }
	| { by: "path"; value: string };

export type InputViewportInput = {
	startLine: number;
	endLine: number;
};

export type InputGetReviewSnapshotRequest = {
	input: InputReviewSnapshotInput;
};

export type InputReviewSnapshotInput = {
	worktreePath: string;
	base: string;
};

export type InputGetTerminalPerformanceSwitchesRequest = Record<string, never>;

export type InputGetTerminalSurfaceRequest = {
	owner: InputTerminalSurfaceOwnerV1;
};

export type InputGetWorkflowRequest = {
	name: string;
};

export type InputGetWorkflowConfigRequest = Record<string, never>;

export type InputGetWorkflowExecutionStateRequest = {
	worktreePath: string;
	executionId: string;
};

export type InputGetWorkflowSourceRequest = {
	name: string;
};

export type InputGetWorkspaceNodeDetailRequest = {
	worktreePath: string;
	nodeId: string;
};

export type InputGetWorkspaceSessionNodeIdRequest = {
	worktreePath: string;
	sessionId: string;
};

export type InputGetWorkspaceTreeSelectionReconciliationRequest = {
	worktreePath: string;
	selectedNodeId: string;
};

export type InputGitCreateBranchRequest = {
	repoPath: string;
	branchName: string;
};

export type InputGitStageRequest = {
	repoPath: string;
	paths: InputListstring;
};

export type InputListstring = Array<string>;

export type InputGitStageReviewGroupRequest = {
	input: InputReviewGroupActionInput;
};

export type InputReviewGroupActionInput = {
	worktreePath: string;
	path: string;
	section: string;
	base: string;
	groupId: string;
};

export type InputGitUnstageRequest = {
	repoPath: string;
	paths: InputListstring;
};

export type InputGitUnstageReviewGroupRequest = {
	input: InputReviewGroupActionInput;
};

export type InputKillTerminalSurfaceRequest = {
	owner: InputTerminalSurfaceOwnerV1;
};

export type InputListAgentSessionHistoryRequest = {
	worktreePath: string;
	limit?: number | null;
	after?: string | null;
};

export type InputListAvailableAgentSessionProvidersRequest = Record<
	string,
	never
>;

export type InputListBranchesRequest = {
	repoPath: string;
};

export type InputListBranchesWithStatusRequest = {
	repoPath: string;
};

export type InputListBranchesWithStatusSnapshotRequest = {
	repoPath: string;
};

export type InputListFacetSummariesRequest = {
	kind: string;
};

export type InputListProviderHookHealthWarningsRequest = Record<string, never>;

export type InputListReviewThreadsRequest = {
	worktreeName: string;
	filter?: InputReviewThreadFilterDto | null;
};

export type InputReviewThreadFilterDto = {
	file?: string | null;
	state?: InputReviewThreadStateDto | null;
	author?: InputAuthorScopeDto | null;
	unread?: boolean | null;
	threadId?: InputListstring;
};

export type InputReviewThreadStateDto = "open" | "resolved";

export type InputAuthorScopeDto = "mine" | "other";

export type InputListWorkflowsRequest = Record<string, never>;

export type InputListWorkspaceWorkflowHistoryRequest = {
	worktreePath: string;
};

export type InputListWorkspaceWorktreeNodesRequest = {
	worktreePath: string;
};

export type InputListWorktreesRequest = {
	repoPath: string;
};

export type InputLoadWorkspaceStateRequest = {
	worktreeName: string;
	worktreeRoot: string;
};

export type InputOpenAgentSessionRequest = {
	agentSessionId: string;
	rows: number;
	cols: number;
	callerRequestId: string;
};

export type InputOpenFacetInEditorRequest = {
	kind: string;
	key: string;
};

export type InputOpenFolderInEditorRequest = {
	folderPath: string;
};

export type InputOpenInEditorRequest = {
	filePath: string;
};

export type InputOpenWorkflowInEditorRequest = {
	name: string;
};

export type InputQueryNotionTasksRequest = {
	repoPath: string;
	query: InputNotionTaskQueryInput;
};

export type InputNotionTaskQueryInput = {
	title_filter: string;
	label_filters: InputMapListstring;
	cursor?: string | null;
	page_size?: number | null;
};

export type InputMapListstring = { [key: string]: InputListstring };

export type InputQuitAfterStartupFailureRequest = Record<string, never>;

export type InputRecordTerminalLaunchRendererPhaseRequest = {
	phase: string;
	durationMs: number;
};

export type InputRefreshProviderAvailabilityRequest = Record<string, never>;

export type InputRemoveRepoPathRequest = {
	path: string;
};

export type InputRemoveWorktreeRequest = {
	repoPath: string;
	worktreePath: string;
	force: boolean;
};

export type InputRenameWorkspaceSessionNodeRequest = {
	worktreePath: string;
	nodeId: string;
	name: string;
};

export type InputRenderFacetPreviewRequest = {
	content: string;
	sampleValues: InputMapstring;
};

export type InputMapstring = { [key: string]: string };

export type InputReportFrontendErrorRequest = {
	payload: InputFrontendErrorPayload;
};

export type InputFrontendErrorPayload = {
	errorType: string;
	message: string;
	stack?: string | null;
};

export type InputReportMountedXtermCountRequest = {
	count: number;
};

export type InputReportUsageEventRequest = {
	name: string;
};

export type InputRequestApplicationQuitRequest = {
	request: InputApplicationQuitRequestDtoV1;
};

export type InputApplicationQuitRequestDtoV1 = {
	intent: InputApplicationQuitIntentDtoV1;
};

export type InputApplicationQuitIntentDtoV1 =
	| ({ type: "exit" } & InputApplicationQuitIntentDtoV1Exit)
	| ({ type: "restart" } & InputApplicationQuitIntentDtoV1Restart);

export type InputApplicationQuitIntentDtoV1Exit = {
	code: number;
};

export type InputApplicationQuitIntentDtoV1Restart = {
	code: number;
};

export type InputResetProviderExecutableRequest = {
	provider: string;
};

export type InputResizeTerminalSurfaceRequest = {
	owner: InputTerminalSurfaceOwnerV1;
	rows: number;
	cols: number;
};

export type InputResolveActiveExecutionByWorktreeRequest = {
	worktreePath: string;
};

export type InputResolveReviewThreadRequest = {
	worktreeName: string;
	threadId: string;
	outcome: string;
	summary: string;
};

export type InputRestoreAgentSessionRequest = {
	agentSessionId: string;
	rows: number;
	cols: number;
	callerRequestId: string;
};

export type InputRestoreWorkspaceWorkflowExecutionRequest = {
	worktreePath: string;
	executionId: string;
};

export type InputResumeAgentSessionRequest = {
	agentSessionId: string;
	rows: number;
	cols: number;
	callerRequestId: string;
};

export type InputResumeAgentSessionHistoryCandidateRequest = {
	workspaceIdentity: string;
	worktreePath: string;
	provider: string;
	providerSessionId: string;
	rows: number;
	cols: number;
	callerRequestId: string;
};

export type InputResumeWorkflowRequest = {
	executionId: string;
};

export type InputRetryWorkspaceNodeRequest = {
	worktreePath: string;
	nodeId: string;
};

export type InputSaveFacetRequest = {
	kind: string;
	key: string;
	content: string;
	isNew?: boolean | null;
};

export type InputSaveNotionConfigRequest = {
	repoPath: string;
	apiToken: string;
	databaseId: string;
	propertyMapping: InputPropertyMappingView;
};

export type InputPropertyMappingView = {
	title?: string;
	labels?: InputListLabelPropertyView;
	branch_name?: string;
	branch_prefix?: string;
};

export type InputListLabelPropertyView = Array<InputLabelPropertyView>;

export type InputLabelPropertyView = {
	name: string;
	property_type: string;
};

export type InputSaveWorkflowSourceRequest = {
	source: string;
	originalName?: string | null;
};

export type InputSaveWorkspaceStateRequest = {
	worktreeName: string;
	state: InputWorkspaceStateDto;
};

export type InputWorkspaceStateDto = {
	version: 1;
	tabs: InputWorkspaceTabsStateDto;
	layout: InputWorkspaceLayoutStateDto;
};

export type InputWorkspaceTabsStateDto = {
	editors: InputListWorkspaceTabEntryDto;
	activeEditorPath?: string | null;
};

export type InputListWorkspaceTabEntryDto = Array<InputWorkspaceTabEntryDto>;

export type InputWorkspaceTabEntryDto = {
	path: string;
	name: string;
};

export type InputWorkspaceLayoutStateDto = {
	centerTab: InputWorkspaceCenterTab;
	activeView: string;
	leftNavCollapsed: boolean;
	rightCollapsed: boolean;
	rightBottomCollapsed: boolean;
	rightBottomActiveTab?: string | null;
	selectedDiffFile?: string | null;
	reviewCollapsed?: boolean;
	diffOnlyMode?: boolean;
};

export type InputWorkspaceCenterTab = "agent" | "editor";

export type InputSetBranchBaseRequest = {
	repoPath: string;
	branchName: string;
	base?: string | null;
};

export type InputSetReleashBaseRequest = {
	repoPath: string;
	base?: string | null;
};

export type InputStartTerminalInputPerformanceCollectionRequest = Record<
	string,
	never
>;

export type InputStartTerminalLaunchPerformanceCollectionRequest = Record<
	string,
	never
>;

export type InputStartWorkflowRequest = {
	workflowName: string;
	worktreePath: string;
	request?: string | null;
	createdFrom?: string | null;
};

export type InputStopWatchingRequest = {
	watcherId: number;
};

export type InputStopWorkflowRequest = {
	executionId: string;
};

export type InputTakeTerminalInputPerformanceSamplesRequest = Record<
	string,
	never
>;

export type InputTakeTerminalLaunchPerformanceSamplesRequest = Record<
	string,
	never
>;

export type InputUpdateAppSettingsRequest = {
	app: InputWindowSettings;
};

export type InputWindowSettings = {
	close_to_tray: boolean;
	start_minimized: boolean;
};

export type InputUpdateLoginItemPreferenceRequest = {
	requested: boolean;
};

export type InputUpdateCrashReportingRequest = {
	enabled: boolean;
};

export type InputUpdateExternalEditorRequest = {
	editor: string;
};

export type InputUpdatePerformanceTelemetryRequest = {
	enabled: boolean;
};

export type InputUpdateProviderExecutableRequest = {
	provider: string;
	executable: string;
};

export type InputUpdateWorkflowConfigRequest = {
	workflow: InputWorkflowSection;
};

export type InputWorkflowSection = {
	approval_auto_approve?: boolean;
};

export type InputValidateNotionConfigRequest = {
	apiToken: string;
	databaseId: string;
};

export type InputWritePathsToTerminalSurfaceRequest = {
	owner: InputTerminalSurfaceOwnerV1;
	paths: InputListstring;
};

export type InputWriteTerminalSurfaceRequest = {
	owner: InputTerminalSurfaceOwnerV1;
	attachmentId: string;
	sequence: number;
	clientStartedAtUnixMs?: number | null;
	data: string;
};

export type InputGetCrashReportingEnabledRequest = Record<string, never>;

export type InputGetFileAtRefRequest = {
	filePath: string;
	gitRef: string;
};

export type InputGetStagedContentRequest = {
	filePath: string;
};

export type InputGetBinaryStagedContentRequest = {
	filePath: string;
};

export type InputGetFileAtBranchBaseRequest = {
	filePath: string;
};

export type InputGetBinaryFileAtBranchBaseRequest = {
	filePath: string;
};

export type InputGetBinaryFileAtRefRequest = {
	filePath: string;
	gitRef: string;
};

export type InputGetBranchDiffSummaryRequest = {
	repoPath: string;
	baseBranch?: string | null;
};

export type InputBuildDiffFileTreeRequest = {
	entries: InputListDiffFileEntryInput;
};

export type InputListDiffFileEntryInput = Array<InputDiffFileEntryInput>;

export type InputDiffFileEntryInput = {
	path: string;
	status: string;
	additions: number;
	deletions: number;
};

export type InputGetHeadDiffFileTreeSnapshotRequest = {
	repoPath: string;
};

export type InputComputeHiddenRangesRequest = {
	hunks: InputListHunkInput;
	totalLines: number;
	contextLines: number;
};

export type InputListHunkInput = Array<InputHunkInput>;

export type InputHunkInput = {
	index: number;
	hunkId?: string;
	oldStart: number;
	oldLines: number;
	newStart: number;
	newLines: number;
	lines: InputListstring;
};

export type InputGetRelativePathRequest = {
	rootPath: string;
	filePath: string;
};

export type InputGetReviewThreadRequest = {
	worktreeName: string;
	threadId: string;
};

export type InputGetReviewThreadHistoryRequest = {
	worktreeName: string;
	threadId: string;
};

export type InputFetchPrStatusRequest = {
	repoPath: string;
};

export type InputGetDefaultBranchRequest = {
	repoPath: string;
};

export type InputGetGitStatusRequest = {
	repoPath: string;
	includeIgnored?: boolean | null;
};

export type InputGetGitStatusSnapshotRequest = {
	repoPath: string;
};

export type InputGetStatusDiffStatsRequest = {
	repoPath: string;
};

export type InputGetStatusDiffStatsSnapshotRequest = {
	repoPath: string;
};

export type InputGetGitLogRequest = {
	repoPath: string;
	limit?: number | null;
};

export type InputGetWorktreeDirtyCountRequest = {
	worktreePath: string;
};

export type InputGetRepoGitDirRequest = {
	filePath: string;
};

export type InputApproveWorkflowNodeRequest = {
	args: InputApproveWorkflowNodeArgs;
};

export type InputApproveWorkflowNodeArgs = {
	executionId: string;
	nodeName: string;
	nodeExecutionId?: string | null;
	comment?: string | null;
};

export type InputListWorkflowExecutionsRequest = {
	status?: string | null;
	worktreePath: string;
};

export type InputGetWorkflowExecutionRequest = {
	executionId: string;
};

export type InputGetWorkflowExecutionLogRequest = {
	worktreePath: string;
	executionId: string;
};

export type InputGetWorkflowNodeDetailRequest = {
	worktreePath: string;
	executionId: string;
	nodeExecutionId: string;
};

export type InputResolveWorktreeByExecutionRequest = {
	executionId: string;
};

export type InputListFacetsRequest = {
	kind: string;
};

export type InputWorkflowSubmitOutputRequest = {
	worktreePath: string;
	nodeExecutionId: string;
	artifact?: InputWorkflowSubmitArtifactInput | null;
};

export type InputWorkflowSubmitArtifactInput = {
	contract: string;
	value: InputWorkflowValue;
};

export type InputWorkflowValue =
	| null
	| InputResultBool
	| InputWorkflowInteger
	| InputResultUint64
	| InputWorkflowNumber
	| InputResultString
	| InputWorkflowValueList
	| InputWorkflowValueObject;

export type InputResultBool = boolean;

export type InputWorkflowInteger = number;

export type InputResultUint64 = number;

export type InputWorkflowNumber = number;

export type InputResultString = string;

export type InputWorkflowValueList = Array<InputWorkflowValue>;

export type InputWorkflowValueObject = { [key: string]: InputWorkflowValue };

export type InputWorkflowValidateOutputRequest = {
	worktreePath: string;
	executionId: string;
	nodeName: string;
	structuredOutput: InputWorkflowValue;
};

export type InputWorkflowGetOutputRequest = {
	worktreePath: string;
	executionId: string;
	nodeName: string;
};

export type ResultString = string;

export type ResultBool = boolean;

export type ReviewThreadDto = {
	id: string;
	worktreeName: string;
	author: ReviewActorWireDto;
	target: ReviewTargetWireDto;
	state: ReviewThreadStateDto;
	comments: ListReviewCommentDto;
	resolve: ReviewResolveInfoDto | null;
	createdAt: number;
	updatedAt: number;
	version: number;
	canResolve: boolean;
};

export type ReviewActorWireDto = {
	kind: ReviewActorKindWireDto;
	backendId: string | null;
	model: string | null;
	displayName: string;
};

export type ReviewActorKindWireDto = "human" | "agent";

export type ReviewTargetWireDto = {
	filePath: string | null;
	lineNumber: number | null;
	endLine: number | null;
};

export type ReviewThreadStateDto = "open" | "resolved";

export type ListReviewCommentDto = Array<ReviewCommentDto>;

export type ReviewCommentDto = {
	id: string;
	threadId: string;
	author: ReviewActorWireDto;
	content: string;
	createdAt: number;
};

export type ReviewResolveInfoDto = {
	actor: ReviewActorWireDto;
	outcome: string;
	summary: string;
	resolvedAt: number;
};

export type AgentSessionArchiveResponse =
	| "archived"
	| "already_archived"
	| "delete_confirmation_required";

export type ListHiddenRangeDto = Array<HiddenRangeDto>;

export type HiddenRangeDto = {
	startLine: number;
	endLine: number;
	hiddenCount: number;
};

export type ListDiffRangeDto = Array<DiffRangeDto>;

export type DiffRangeDto = {
	startLine: number;
	endLine: number;
	type: DiffRangeKindDto;
};

export type DiffRangeKindDto = "added" | "modified" | "deleted";

export type ListInlineChunkDto = Array<InlineChunkDto>;

export type InlineChunkDto = {
	content: string;
	type: InlineChunkKindDto;
};

export type InlineChunkKindDto = "unchanged" | "added" | "removed";

export type ListSplitRowDto = Array<SplitRowDto>;

export type SplitRowDto = {
	left: string | null;
	right: string | null;
	type: SplitRowKindDto;
};

export type SplitRowKindDto = "unchanged" | "added" | "removed" | "modified";

export type ListVisibleBlockDto = Array<VisibleBlockDto>;

export type VisibleBlockDto = {
	startLine: number;
	endLine: number;
	content: string;
	deletedContent?: string;
};

export type WorktreeEntryDto = {
	name: string;
	path: string;
	branch: string;
	is_main: boolean;
	is_locked: boolean;
	dirty_count: number;
	base_branch: string | null;
};

export type ListEditorInfoDto = Array<EditorInfoDto>;

export type EditorInfoDto = {
	name: string;
	path: string;
};

export type DiagnosticReport = {
	items: ListDiagnosticItem;
	workflow_summaries: MapDiagnosticSummary;
	facet_summaries: MapDiagnosticSummary;
	facet_usage: MapListFacetUsageEntry;
};

export type ListDiagnosticItem = Array<DiagnosticItem>;

export type DiagnosticItem = {
	code: string;
	severity: Severity;
	stage: DiagnosticStage;
	span?: DiagnosticSpan;
	message: string;
	workflow_name?: string;
	node_name?: string;
	facet_key?: string;
	facet_kind?: string;
	field?: string;
};

export type Severity = "error" | "info";

export type DiagnosticStage =
	| "parse_shape"
	| "resolve"
	| "typecheck"
	| "control_flow";

export type DiagnosticSpan = {
	source?: string;
	start_line: number;
	start_col: number;
	end_line: number;
	end_col: number;
};

export type MapDiagnosticSummary = { [key: string]: DiagnosticSummary };

export type DiagnosticSummary = {
	error_count: number;
	info_count: number;
};

export type MapListFacetUsageEntry = { [key: string]: ListFacetUsageEntry };

export type ListFacetUsageEntry = Array<FacetUsageEntry>;

export type FacetUsageEntry = {
	workflow_name: string;
	node_name: string;
	slot: string;
};

export type ListIssueInfoDto = Array<IssueInfoDto>;

export type IssueInfoDto = {
	number: number;
	default_branch_name: string;
	title: string;
	state: string;
	url: string;
	author: PrAuthorDto;
	created_at: string;
	updated_at: string;
	labels: ListIssueLabelDto;
	assignees: ListPrAuthorDto;
	body: string;
	milestone: MilestoneDto | null;
};

export type PrAuthorDto = {
	login: string;
};

export type ListIssueLabelDto = Array<IssueLabelDto>;

export type IssueLabelDto = {
	name: string;
	color: string;
};

export type ListPrAuthorDto = Array<PrAuthorDto>;

export type MilestoneDto = {
	title: string;
};

export type ListNotionLabelOptionView = Array<NotionLabelOptionView>;

export type NotionLabelOptionView = {
	property_name: string;
	property_type: string;
	options: Liststring;
	option_ids: Liststring;
};

export type Liststring = Array<string>;

export type NullableAgentSessionItemDto = AgentSessionItemDto | null;

export type AgentSessionItemDto = {
	id: string;
	workspaceIdentity: string;
	worktreePath: string;
	workspaceWorktreePath: string;
	provider: AgentSessionProviderDto;
	treeLocation: AgentSessionTreeLocationDto;
	lifecycle: AgentSessionLifecycleDto;
	providerSessionId: string | null;
	transcriptRef: string | null;
	operations: AgentSessionOperationsDto;
	lastExitAbnormal: boolean;
};

export type AgentSessionProviderDto = "claude" | "codex";

export type AgentSessionTreeLocationDto = {
	treeId: string;
	nodeExecutionId: string;
};

export type AgentSessionLifecycleDto = "open" | "paused" | "archived";

export type AgentSessionOperationsDto = {
	canArchive: boolean;
	canRestore: boolean;
	canDelete: boolean;
	canResume: boolean;
};

export type AppSection = {
	close_to_tray: boolean;
	auto_launch: boolean;
	start_minimized: boolean;
	last_root_path: string;
	last_repo_paths: Liststring;
	external_editor: string;
};

export type ApplicationStartupOutcomeDtoV1 =
	| { type: "ready" }
	| ({ type: "failed" } & ApplicationStartupOutcomeDtoV1Failed);

export type ApplicationStartupOutcomeDtoV1Failed = {
	kind: StartupFailureKindDtoV1;
	safeDescription: string;
	correlationId: string;
	retryOnNextLaunch: boolean;
	actions: ListStartupFailureActionDtoV1;
};

export type StartupFailureKindDtoV1 =
	| "store_in_use"
	| "storage_unavailable"
	| "unsupported_runtime"
	| "unsupported_store_version"
	| "initialization_state_invalid"
	| "store_validation_failed"
	| "schema_evolution_failed";

export type ListStartupFailureActionDtoV1 = Array<StartupFailureActionDtoV1>;

export type StartupFailureActionDtoV1 = "quit";

export type Nullablestring = string | null;

export type PrStatusDto = {
	open_prs: MapPrInfoDto;
	merged_branches: Liststring;
};

export type MapPrInfoDto = { [key: string]: PrInfoDto };

export type PrInfoDto = {
	number: number;
	url: string;
};

export type FileNavigationResultDto = {
	current_index: number;
	total: number;
	prev_file: string | null;
	next_file: string | null;
};

export type NullableNotionRepoConfigView = NotionRepoConfigView | null;

export type NotionRepoConfigView = {
	api_token: string;
	database_id: string;
	property_mapping: PropertyMappingView;
};

export type PropertyMappingView = {
	title: string;
	labels: ListLabelPropertyView;
	branch_name: string;
	branch_prefix: string;
};

export type ListLabelPropertyView = Array<LabelPropertyView>;

export type LabelPropertyView = {
	name: string;
	property_type: string;
};

export type GetOrSpawnTerminalV1 = {
	session_key: string;
	restored_from_checkpoint: boolean;
	is_new: boolean;
	is_exited: boolean;
	exit_code: number | null;
};

export type ProviderAvailabilitySnapshotResponse = {
	providers: ListProviderAvailabilityItemResponse;
};

export type ListProviderAvailabilityItemResponse =
	Array<ProviderAvailabilityItemResponse>;

export type ProviderAvailabilityItemResponse = {
	provider: string;
	displayName: string;
	defaultExecutable: string;
	configuredExecutable: string | null;
	effectiveExecutable: string;
	available: boolean;
	resolvedExecutable: string | null;
	unavailableReason: string | null;
};

export type ReviewFileViewDto =
	| ({ kind: "textDiff" } & ReviewTextDiffDto)
	| ({ kind: "image" } & ReviewImageDto)
	| ({ kind: "binary" } & ReviewBinaryDto)
	| ({ kind: "fallback" } & ReviewFallbackDto);

export type ReviewTextDiffDto = {
	version: number;
	stale: boolean;
	fileId: string;
	path: string;
	original: string;
	modified: string;
	source: ReviewTextSource;
	hunks: ListHunkDto;
	changeGroups: ListChangeGroupDto;
	limited: boolean;
	viewport: ViewportDto | null;
	totalLines: number;
};

export type ReviewTextSource = "diff" | "added" | "deleted";

export type ListHunkDto = Array<HunkDto>;

export type HunkDto = {
	index: number;
	hunkId: string;
	oldStart: number;
	oldLines: number;
	newStart: number;
	newLines: number;
	lines: Liststring;
};

export type ListChangeGroupDto = Array<ChangeGroupDto>;

export type ChangeGroupDto = {
	groupIndex: number;
	groupId: string;
	hunkIndex: number;
	newStart: number;
	newEnd: number;
	lineOffsetStart: number;
	lineOffsetEnd: number;
	isStaged?: boolean;
};

export type ViewportDto = {
	startLine: number;
	endLine: number;
};

export type ReviewImageDto = {
	version: number;
	stale: boolean;
	fileId: string;
	path: string;
	originalUrl: string | null;
	modifiedUrl: string | null;
	mime: string;
};

export type ReviewBinaryDto = {
	version: number;
	stale: boolean;
	fileId: string;
	path: string;
	originalUrl: string | null;
	modifiedUrl: string | null;
	originalSize: number | null;
	modifiedSize: number | null;
};

export type ReviewFallbackDto = {
	version: number;
	stale: boolean;
	fileId: string;
	path: string;
	reason: ReviewLimitReasonDto;
	totalLines: number | null;
	sizeBytes: number | null;
	hunkCount: number | null;
	limited: true;
};

export type ReviewLimitReasonDto =
	| "fileSize"
	| "lineCount"
	| "hunkCount"
	| "tokenization";

export type ReviewSnapshotDto = {
	version: number;
	stale: boolean;
	loading: boolean;
	limited: boolean;
	base: DiffBase;
	files: ListReviewFileEntryDto;
	stagedFiles: ListFileStatusDto;
	changedFiles: ListFileStatusDto;
	diffStats: ListFileDiffStatDto;
	tree: ListDiffTreeNodeDto;
	stagedTree: ListDiffTreeNodeDto;
	changesTree: ListDiffTreeNodeDto;
	stagedFileCount: number;
	changesFileCount: number;
};

export type DiffBase = "branch-base" | "head";

export type ListReviewFileEntryDto = Array<ReviewFileEntryDto>;

export type ReviewFileEntryDto = {
	fileId: string;
	path: string;
	indexStatus: string;
	worktreeStatus: string;
	additions: number;
	deletions: number;
};

export type ListFileStatusDto = Array<FileStatusDto>;

export type FileStatusDto = {
	path: string;
	index_status: GitIndexStatus;
	worktree_status: GitWorktreeStatus;
};

export type GitIndexStatus =
	| "new"
	| "modified"
	| "deleted"
	| "none"
	| "renamed";

export type GitWorktreeStatus =
	| "new"
	| "modified"
	| "deleted"
	| "ignored"
	| "none";

export type ListFileDiffStatDto = Array<FileDiffStatDto>;

export type FileDiffStatDto = {
	path: string;
	index_additions: number;
	index_deletions: number;
	wt_additions: number;
	wt_deletions: number;
};

export type ListDiffTreeNodeDto = Array<DiffTreeNodeDto>;

export type DiffTreeNodeDto = {
	id: string;
	name: string;
	path: string;
	node_type: DiffTreeNodeType;
	status: string | null;
	additions: number | null;
	deletions: number | null;
	children: ListDiffTreeNodeDto;
};

export type DiffTreeNodeType = "file" | "folder";

export type TerminalPerformanceSwitchesV1 = {
	disableOutputFlowControl: boolean;
	disableTerminalJournal: boolean;
	disableRendererWriteSerialization: boolean;
	disableWebglRenderer: boolean;
};

export type TerminalSurfaceSummaryV1 = {
	session_key: string;
	is_exited: boolean;
	exit_code: number | null;
};

export type WorkflowDto = {
	name: string;
	description: string;
	builtin: boolean;
	sourceFormat: WorkflowSourceFormat;
	schemas?: WorkflowValueObject;
	nodes: ListNodeDefinitionDto;
};

export type WorkflowSourceFormat = "yaml" | "lua";

export type WorkflowValueObject = { [key: string]: WorkflowValue };

export type WorkflowValue =
	| null
	| ResultBool
	| WorkflowInteger
	| ResultUint64
	| WorkflowNumber
	| ResultString
	| WorkflowValueList
	| WorkflowValueObject;

export type WorkflowInteger = number;

export type ResultUint64 = number;

export type WorkflowNumber = number;

export type WorkflowValueList = Array<WorkflowValue>;

export type ListNodeDefinitionDto = Array<NodeDefinitionDto>;

export type NodeDefinitionDto = {
	name: string;
	kind: NodeKindDto;
	command?: string;
	session?: SessionSpecDto;
	fanout?: FanoutSpecDto;
	sequence?: SequenceSpecDto;
	artifact?: string;
	input?: ListInputParamDto;
	completion?: NodeCompletionDto;
	worktree?: WorktreeMode;
};

export type NodeKindDto = "session" | "command" | "fanout" | "sequence";

export type SessionSpecDto = {
	provider: SessionProviderDto;
	model?: string;
	permission?: string;
	facets: FacetRefsDto;
};

export type SessionProviderDto = "claude" | "codex";

export type FacetRefsDto = {
	policy?: string;
	knowledge?: Liststring;
	instruction?: string;
};

export type FanoutSpecDto = {
	children: ListChildEntryDto;
	items?: ItemsSourceDto;
};

export type ListChildEntryDto = Array<ChildEntryDto>;

export type ChildEntryDto = {
	name: string;
	inputs?: ListChildInputDto;
	rules?: ListRuleDto;
};

export type ListChildInputDto = Array<ChildInputDto>;

export type ChildInputDto = {
	parameter: string;
	source: string;
};

export type ListRuleDto = Array<RuleDto>;

export type RuleDto =
	| ({ type: "when" } & RuleDtoWhen)
	| ({ type: "switch" } & RuleDtoSwitch)
	| ({ type: "loop_guard" } & RuleDtoLoopGuard)
	| ({ type: "next" } & RuleDtoNext);

export type RuleDtoWhen = {
	on: PredicateDto;
	then: string;
	next: string;
};

export type PredicateDto = string | PredicateDtoAnd | PredicateDtoOr;

export type PredicateDtoAnd = {
	and: ListPredicateDto;
};

export type ListPredicateDto = Array<PredicateDto>;

export type PredicateDtoOr = {
	or: ListPredicateDto;
};

export type RuleDtoSwitch = {
	on: string;
	cases: Mapstring;
	next: string;
};

export type Mapstring = { [key: string]: string };

export type RuleDtoLoopGuard = {
	max_iterations: number;
	on_exhausted: string;
};

export type RuleDtoNext = {
	next: string;
};

export type ItemsSourceDto = WorkflowValueList | string;

export type SequenceSpecDto = {
	entry?: string;
	children: ListChildEntryDto;
};

export type ListInputParamDto = Array<InputParamDto>;

export type InputParamDto = {
	name: string;
	contract?: string;
};

export type NodeCompletionDto = {
	require?: CompletionRequirementDto;
	delegate?: SessionDelegateDto;
};

export type CompletionRequirementDto = "approval";

export type SessionDelegateDto = {
	child: string;
	inputs: ListChildInputDto;
	when: PredicateDto;
	max_iterations: number;
};

export type WorktreeMode = "shared" | "isolated";

export type WorkflowSection = {
	approval_auto_approve: boolean;
};

export type NullableWorkflowExecutionView = WorkflowExecutionView | null;

export type WorkflowExecutionView = {
	id: string;
	workflowName: string;
	status: ExecutionStatusView;
	currentNode: string | null;
	worktreePath: string;
	createdFrom: ExecutionOriginView;
	startedAt: number;
	updatedAt: number;
	completedAt: number | null;
	errorReason: string | null;
	totalTokenUsage: TokenUsageView;
	nodeExecutions: ListNodeExecutionView;
	artifacts: ListArtifactView;
	fanouts: ListFanoutView;
	approvalTarget: ApprovalTargetView | null;
};

export type ExecutionStatusView = "running" | "completed" | "aborted";

export type ExecutionOriginView = "desktop_ui" | "cli" | "agent" | "api";

export type TokenUsageView = {
	inputTokens: number;
	outputTokens: number;
};

export type ListNodeExecutionView = Array<NodeExecutionView>;

export type NodeExecutionView = {
	worktree?: NodeWorktreeDto;
	recoveryReason?: string;
	id: string;
	executionId: string;
	nodeName: string;
	kind: NodeKindView;
	attempt: number;
	status: NodeExecutionStatusView;
	submitReceived: boolean;
	stopReceived: boolean;
	waitingFor?: NodeCompletionSignalView;
	canApprove: boolean;
	canRetry: boolean;
	hasArtifact: boolean;
	sessionId?: string;
	displayCommand?: string;
	resultSummary?: string;
	artifact?: ArtifactView;
	tokenUsage?: TokenUsageView;
	failure?: NodeExecutionFailureView;
	parent?: ExecutionParentRefView;
	startedAt: number;
	completedAt?: number;
};

export type NodeWorktreeDto = {
	branch: string;
	path: string;
};

export type NodeKindView = "command" | "session" | "fanout" | "sequence";

export type NodeExecutionStatusView =
	| "unresolved"
	| "running"
	| "paused"
	| "waiting_approval"
	| "succeeded"
	| "failed"
	| "aborted";

export type NodeCompletionSignalView = "submit" | "stop";

export type ArtifactView = {
	nodeName: string;
	contract?: string;
	value: WorkflowValue;
	producedAt: number;
};

export type NodeExecutionFailureView = {
	reason: string;
	kind: NodeExecutionFailureKindView;
};

export type NodeExecutionFailureKindView =
	| "startup_timeout"
	| "stale_runtime_timeout"
	| "model_refusal"
	| "structured_output_mismatch"
	| "validation_failure"
	| "user_abort"
	| "infrastructure_crash";

export type ExecutionParentRefView = {
	parentId: string;
	itemIndex?: number;
	childIndex?: number;
};

export type ListArtifactView = Array<ArtifactView>;

export type ListFanoutView = Array<FanoutView>;

export type FanoutView = {
	parent: NodeExecutionView;
	children: ListNodeExecutionView;
	artifact?: ArtifactView;
};

export type ApprovalTargetView = {
	nodeExecutionId: string;
	nodeName: string;
	sessionId?: string;
};

export type NullableWorkspaceNodeDetailDto = WorkspaceNodeDetailDto | null;

export type WorkspaceNodeDetailDto = {
	worktree?: NodeWorktreeDto;
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	statusClassification: WorkspaceStatusClassification;
	submitReceived: boolean;
	stopReceived: boolean;
	waitingFor?: WorkspaceWaitingFor;
	hasArtifact: boolean;
	errorReason?: string;
	recoveryReason?: string;
	capabilities: WorkspaceNodeCapabilitiesDto;
	updatedAt: number;
	content: WorkspaceNodeContentDto;
};

export type WorkspaceNodeStatus =
	| "unresolved"
	| "running"
	| "paused"
	| "failed"
	| "waiting"
	| "aborted"
	| "completed";

export type WorkspaceStatusClassification =
	| "active"
	| "attention"
	| "failure"
	| "idle"
	| "unbound";

export type WorkspaceWaitingFor = "submit" | "stop";

export type WorkspaceNodeCapabilitiesDto = {
	canRename: boolean;
	canApprove: boolean;
	canRetry: boolean;
};

export type WorkspaceNodeContentDto =
	| ({ kind: "session" } & WorkspaceSessionNodeContentDto)
	| ({ kind: "command" } & WorkspaceCommandNodeContentDto);

export type WorkspaceSessionNodeContentDto = {
	sessionId?: string;
};

export type WorkspaceCommandNodeContentDto = {
	displayCommand?: string;
	result?: WorkspaceCommandResultDto;
};

export type WorkspaceCommandResultDto = {
	exitCode: number;
	duration: number;
	stdout: string;
	stderr: string;
};

export type WorkspaceTreeSelectionSnapshotDto = {
	snapshot: WorkspaceTreeSnapshotDto;
	reconciliation: WorkspaceSelectionReconciliationDto;
};

export type WorkspaceTreeSnapshotDto = {
	nodes: ListWorkspaceTreeItemDto;
	archivedSessions: ListAgentSessionItemDto;
	preferredNodeId?: string;
};

export type ListWorkspaceTreeItemDto = Array<WorkspaceTreeItemDto>;

export type WorkspaceTreeItemDto =
	| ({ kind: "node" } & WorkspaceNodeDto)
	| ({ kind: "sequence" } & WorkspaceSequenceDto)
	| ({ kind: "fanout" } & WorkspaceFanoutDto);

export type WorkspaceNodeDto = {
	id: string;
	title: string;
	status: WorkspaceStatusClassification;
	errorReason?: string;
	contentKind: WorkspaceContentKind;
	capabilities: WorkspaceNodeCapabilitiesDto;
	workflowCapabilities?: WorkspaceWorkflowCapabilitiesDto;
	sessionCapabilities?: WorkspaceSessionCapabilitiesDto;
	children?: ListWorkspaceTreeItemDto;
	pastAttempts: ListWorkspaceNodeDto;
	pastAttemptsCollapsed: boolean;
	updatedAt: number;
};

export type WorkspaceContentKind = "session" | "command";

export type WorkspaceWorkflowCapabilitiesDto = {
	canStop: boolean;
	canResume: boolean;
	canAbort: boolean;
	canArchive: boolean;
};

export type WorkspaceSessionCapabilitiesDto = {
	sessionRef: string;
	canArchive: boolean;
	canDelete: boolean;
};

export type ListWorkspaceNodeDto = Array<WorkspacePastAttemptDto>;

export type WorkspacePastAttemptDto = { kind: "node" } & WorkspaceNodeDto;

export type WorkspaceSequenceDto = {
	worktree?: NodeWorktreeDto;
	id: string;
	title: string;
	status: WorkspaceStatusClassification;
	workflowCapabilities?: WorkspaceWorkflowCapabilitiesDto;
	children: ListWorkspaceTreeItemDto;
	updatedAt: number;
};

export type WorkspaceFanoutDto = {
	worktree?: NodeWorktreeDto;
	id: string;
	title: string;
	status: WorkspaceStatusClassification;
	workflowCapabilities?: WorkspaceWorkflowCapabilitiesDto;
	children: ListWorkspaceTreeItemDto;
	updatedAt: number;
};

export type ListAgentSessionItemDto = Array<AgentSessionItemDto>;

export type WorkspaceSelectionReconciliationDto = {
	selectionInSnapshot: boolean;
};

export type AgentSessionHistoryPageDto = {
	items: ListAgentSessionHistoryCandidateDto;
	nextAfter: string | null;
};

export type ListAgentSessionHistoryCandidateDto =
	Array<AgentSessionHistoryCandidateDto>;

export type AgentSessionHistoryCandidateDto = {
	provider: AgentSessionProviderDto;
	providerSessionId: string;
	label: string;
	updatedAtMs: number;
};

export type ListAgentSessionProviderDto = Array<AgentSessionProviderDto>;

export type ListBranchDto = Array<BranchDto>;

export type BranchDto = {
	name: string;
	is_remote: boolean;
};

export type ListBranchCardDto = Array<BranchCardDto>;

export type BranchCardDto = {
	name: string;
	is_main_worktree: boolean;
	worktree_path: string | null;
	dirty_count: number;
	is_merged: boolean;
	ahead: number;
	behind: number;
	has_upstream: boolean;
	base_ahead: number;
};

export type RepositoryBranchCardsSnapshotDto = {
	version: number;
	stale: boolean;
	loading: boolean;
	limited: boolean;
	branches: ListBranchCardDto;
	worktree_display_groups: WorktreeDisplayGroupsDto;
};

export type WorktreeDisplayGroupsDto = {
	working_areas: ListBranchCardDto;
};

export type ListFacetSummaryDto = Array<FacetSummaryDto>;

export type FacetSummaryDto = {
	key: string;
	kind: string;
	description: string;
	builtin: boolean;
};

export type ListProviderHookHealthWarningResponse =
	Array<ProviderHookHealthWarningResponse>;

export type ProviderHookHealthWarningResponse = {
	provider: ProviderHookHealthProviderResponse;
	launchId: string;
	reason: string;
};

export type ProviderHookHealthProviderResponse = "claude" | "codex";

export type ListReviewThreadDto = Array<ReviewThreadDto>;

export type ListWorkflowSummaryDto = Array<WorkflowSummaryDto>;

export type WorkflowSummaryDto = {
	name: string;
	description: string;
	builtin: boolean;
	is_running: boolean;
	sourceFormat: WorkflowSourceFormat;
};

export type ListWorkspaceWorkflowHistoryItemDto =
	Array<WorkspaceWorkflowHistoryItemDto>;

export type WorkspaceWorkflowHistoryItemDto = {
	executionId: string;
	worktreePath: string;
	title: string;
	status: WorkspaceHistoryStatus;
	updatedAt: number;
	archivedAt: number;
	archiveReason: string;
};

export type WorkspaceHistoryStatus =
	| "unresolved"
	| "running"
	| "paused"
	| "failed"
	| "waiting"
	| "aborted"
	| "completed";

export type ListWorktreeEntryDto = Array<WorktreeEntryDto>;

export type NullableWorkspaceStateDto = WorkspaceStateDto | null;

export type WorkspaceStateDto = {
	version: 1;
	tabs: WorkspaceTabsStateDto;
	layout: WorkspaceLayoutStateDto;
};

export type WorkspaceTabsStateDto = {
	editors: ListWorkspaceTabEntryDto;
	activeEditorPath: string | null;
};

export type ListWorkspaceTabEntryDto = Array<WorkspaceTabEntryDto>;

export type WorkspaceTabEntryDto = {
	path: string;
	name: string;
};

export type WorkspaceLayoutStateDto = {
	centerTab: WorkspaceCenterTab;
	activeView: string;
	leftNavCollapsed: boolean;
	rightCollapsed: boolean;
	rightBottomCollapsed: boolean;
	rightBottomActiveTab?: string;
	selectedDiffFile?: string;
	reviewCollapsed?: boolean;
	diffOnlyMode?: boolean;
};

export type WorkspaceCenterTab = "agent" | "editor";

export type AgentSessionOpenResponse =
	| "attached"
	| "resumed"
	| "restored"
	| "paused"
	| "indeterminate"
	| "garbage_collected";

export type NotionTaskPageView = {
	tasks: ListNotionTaskView;
	has_more: boolean;
	next_cursor: string | null;
};

export type ListNotionTaskView = Array<NotionTaskView>;

export type NotionTaskView = {
	id: string;
	title: string;
	url: string;
	labels: MapListstring;
	branch_name: string;
	created_at: string;
	last_edited_at: string;
};

export type MapListstring = { [key: string]: Liststring };

export type StartupFailureQuitOutcomeDtoV1 = {
	type: "accepted";
} & StartupFailureQuitOutcomeDtoV1Accepted;

export type StartupFailureQuitOutcomeDtoV1Accepted = {
	correlationId: string;
};

export type ApplicationQuitOutcomeDtoV1 = { type: "accepted" };

export type SaveWorkflowSourceResultDto =
	| SaveWorkflowSuccess
	| SaveWorkflowDiagnostics;

export type SaveWorkflowSuccess = {
	ok: true;
	workflow: WorkflowDto;
};

export type SaveWorkflowDiagnostics = {
	ok: false;
	error: string;
	diagnostics: ListDiagnosticItem;
};

export type ListTerminalInputPerformanceSampleV1 =
	Array<TerminalInputPerformanceSampleV1>;

export type TerminalInputPerformanceSampleV1 = {
	sequence: number;
	onDataToCommandIngressMs: number;
	commandIngressToAdmissionMs: number;
	admissionToWriterEnqueueMs: number;
	writerEnqueueToOutputReadMs: number;
	outputReadToModelApplyMs: number;
	modelApplyToEventPublishMs: number;
	eventPublishedAtUnixMs: number;
};

export type ListTerminalLaunchPerformanceSampleV1 =
	Array<TerminalLaunchPerformanceSampleV1>;

export type TerminalLaunchPerformanceSampleV1 = {
	phase: string;
	durationMs: number;
};

export type NotionValidationResultView = {
	status: NotionConfigStatusView;
	properties: ListNotionPropertyInfoView;
};

export type NotionConfigStatusView =
	| "not_configured"
	| "configured"
	| "invalid_token"
	| "invalid_database"
	| "network_error";

export type ListNotionPropertyInfoView = Array<NotionPropertyInfoView>;

export type NotionPropertyInfoView = {
	name: string;
	property_type: string;
	options: Liststring;
};

export type BranchDiffSummaryDto = {
	base_branch: string;
	changed_files: ListChangedFileDto;
	stats: DiffStatsDto;
};

export type ListChangedFileDto = Array<ChangedFileDto>;

export type ChangedFileDto = {
	path: string;
	old_path: string | null;
	status: string;
	binary: boolean;
	stats: DiffStatsDto;
};

export type DiffStatsDto = {
	additions: number;
	deletions: number;
};

export type RepositoryHeadDiffFileTreeSnapshotDto = {
	version: number;
	stale: boolean;
	loading: boolean;
	limited: boolean;
	combined_tree: ListDiffTreeNodeDto;
	staged_tree: ListDiffTreeNodeDto;
	changes_tree: ListDiffTreeNodeDto;
	staged_file_count: number;
	changes_file_count: number;
};

export type ListReviewHistoryEntryDto = Array<ReviewHistoryEntryDto>;

export type ReviewHistoryEntryDto =
	| ({ kind: "thread_created" } & ReviewHistoryEntryDtoThreadCreated)
	| ({ kind: "comment_appended" } & ReviewHistoryEntryDtoCommentAppended)
	| ({ kind: "thread_resolved" } & ReviewHistoryEntryDtoThreadResolved)
	| ({ kind: "thread_deleted" } & ReviewHistoryEntryDtoThreadDeleted);

export type ReviewHistoryEntryDtoThreadCreated = {
	id: string;
	threadId: string;
	commentId: string;
	actor: ReviewActorWireDto;
	target: ReviewTargetWireDto;
	content: string;
	at: number;
};

export type ReviewHistoryEntryDtoCommentAppended = {
	id: string;
	threadId: string;
	commentId: string;
	actor: ReviewActorWireDto;
	content: string;
	at: number;
};

export type ReviewHistoryEntryDtoThreadResolved = {
	id: string;
	threadId: string;
	actor: ReviewActorWireDto;
	outcome: string;
	summary: string;
	at: number;
};

export type ReviewHistoryEntryDtoThreadDeleted = {
	id: string;
	threadId: string;
	actor: ReviewActorWireDto;
	at: number;
};

export type RepositoryStatusSnapshotDto = {
	version: number;
	stale: boolean;
	loading: boolean;
	limited: boolean;
	status: ListFileStatusDto;
};

export type RepositoryDiffStatsSnapshotDto = {
	version: number;
	stale: boolean;
	loading: boolean;
	limited: boolean;
	diff_stats: ListFileDiffStatDto;
};

export type ListCommitDto = Array<CommitDto>;

export type CommitDto = {
	hash: string;
	short_hash: string;
	message: string;
	author_name: string;
	author_email: string;
	timestamp: number;
};

export type ResultUint32 = number;

export type ListWorkflowExecutionSummaryDto =
	Array<WorkflowExecutionSummaryDto>;

export type WorkflowExecutionSummaryDto = {
	executionId: string;
	workflowName: string;
	status: ExecutionStatusDto;
	worktreePath: string;
	currentNode?: string;
	createdFrom: ExecutionOriginDto;
	startedAt: number;
	updatedAt: number;
	completedAt?: number;
	errorReason?: string;
	totalTokenUsage: TokenUsageDto;
};

export type ExecutionStatusDto = "running" | "completed" | "aborted";

export type ExecutionOriginDto = "desktop_ui" | "cli" | "agent" | "api";

export type TokenUsageDto = {
	inputTokens: number;
	outputTokens: number;
};

export type NullableWorkflowExecutionSummaryDto =
	WorkflowExecutionSummaryDto | null;

export type NullableListDurableWorkflowFactLogEntry =
	ListDurableWorkflowFactLogEntry | null;

export type ListDurableWorkflowFactLogEntry =
	Array<DurableWorkflowFactLogEntry>;

export type DurableWorkflowFactLogEntry = {
	event: string;
	execution_id: string;
	timestampMs: number;
} & { [key: string]: WorkflowValue };

export type NullableNodeExecutionView = NodeExecutionView | null;

export type WorkflowValidateOutputResponse =
	| { status: "valid" }
	| ({ status: "invalid" } & WorkflowValidateOutputResponseInvalid);

export type WorkflowValidateOutputResponseInvalid = {
	reason: string;
	details: string;
};

export type WorkflowGetOutputResponse =
	| ({ status: "submitted" } & WorkflowGetOutputResponseSubmitted)
	| { status: "not_submitted" };

export type WorkflowGetOutputResponseSubmitted = {
	contract: string | null;
	structured_output: WorkflowValue;
	submitted_at?: number;
	request_id?: string;
	timestamp: number;
};

export type AgentSessionChangedPayload = {
	worktreePath: string;
};

export type FileChangeEvent = {
	watcher_id: number;
	path: string;
	kind: string;
};

export type GitStatusChangedEvent = {
	repo_path: string;
};

export type RepositorySnapshotChangedEvent = {
	worktree_path: string;
	version: number;
	stale: boolean;
	loading: boolean;
	limited: boolean;
};

export type WorkflowExecutionChangedPayloadView = {
	worktreePath: string;
	workflowExecution: WorkflowExecutionView;
};

export interface ClientCommandArgs {
	attach_terminal_surface: InputAttachTerminalSurfaceRequest;
	get_review_blob: InputGetReviewBlobRequest;
	abort_workflow: InputAbortWorkflowRequest;
	ack_terminal_surface_output: InputAckTerminalSurfaceOutputRequest;
	add_repo_path: InputAddRepoPathRequest;
	append_review_comment: InputAppendReviewCommentRequest;
	approve_workspace_node: InputApproveWorkspaceNodeRequest;
	archive_agent_session: InputArchiveAgentSessionRequest;
	archive_workspace_workflow_execution: InputArchiveWorkspaceWorkflowExecutionRequest;
	build_review_thread_handoff: InputBuildReviewThreadHandoffRequest;
	compute_hidden_ranges_from_content: InputComputeHiddenRangesFromContentRequest;
	compute_markdown_diff_ranges: InputComputeMarkdownDiffRangesRequest;
	compute_markdown_inline_chunks: InputComputeMarkdownInlineChunksRequest;
	compute_markdown_split_rows: InputComputeMarkdownSplitRowsRequest;
	compute_visible_markdown_blocks: InputComputeVisibleMarkdownBlocksRequest;
	confirm_agent_session_archive_delete: InputConfirmAgentSessionArchiveDeleteRequest;
	create_agent_session: InputCreateAgentSessionRequest;
	create_review_thread: InputCreateReviewThreadRequest;
	create_worktree: InputCreateWorktreeRequest;
	delete_agent_session: InputDeleteAgentSessionRequest;
	delete_branch: InputDeleteBranchRequest;
	delete_facet: InputDeleteFacetRequest;
	delete_notion_config: InputDeleteNotionConfigRequest;
	delete_review_thread: InputDeleteReviewThreadRequest;
	delete_workflow: InputDeleteWorkflowRequest;
	detach_terminal_surface: InputDetachTerminalSurfaceRequest;
	detect_editors: InputDetectEditorsRequest;
	diagnose_all_cmd: InputDiagnoseAllCmdRequest;
	duplicate_facet: InputDuplicateFacetRequest;
	duplicate_workflow: InputDuplicateWorkflowRequest;
	fetch_issues: InputFetchIssuesRequest;
	fetch_notion_label_options: InputFetchNotionLabelOptionsRequest;
	get_agent_session: InputGetAgentSessionRequest;
	get_app_settings: InputGetAppSettingsRequest;
	get_application_startup_outcome: InputGetApplicationStartupOutcomeRequest;
	get_automation_config_dir: InputGetAutomationConfigDirRequest;
	get_branch_base: InputGetBranchBaseRequest;
	get_cached_issues: InputGetCachedIssuesRequest;
	get_cached_pr_status: InputGetCachedPrStatusRequest;
	get_current_branch: InputGetCurrentBranchRequest;
	get_cwd: InputGetCwdRequest;
	get_external_editor: InputGetExternalEditorRequest;
	get_facet: InputGetFacetRequest;
	get_file_navigation: InputGetFileNavigationRequest;
	get_language_from_path: InputGetLanguageFromPathRequest;
	get_main_repo_path: InputGetMainRepoPathRequest;
	get_notion_config: InputGetNotionConfigRequest;
	get_or_spawn_terminal_surface: InputGetOrSpawnTerminalSurfaceRequest;
	get_performance_real_app_mode: InputGetPerformanceRealAppModeRequest;
	get_performance_telemetry_enabled: InputGetPerformanceTelemetryEnabledRequest;
	get_provider_availability: InputGetProviderAvailabilityRequest;
	get_releash_base: InputGetReleashBaseRequest;
	get_repo_paths: InputGetRepoPathsRequest;
	get_review_file_view: InputGetReviewFileViewRequest;
	get_review_snapshot: InputGetReviewSnapshotRequest;
	get_terminal_performance_switches: InputGetTerminalPerformanceSwitchesRequest;
	get_terminal_surface: InputGetTerminalSurfaceRequest;
	get_workflow: InputGetWorkflowRequest;
	get_workflow_config: InputGetWorkflowConfigRequest;
	get_workflow_execution_state: InputGetWorkflowExecutionStateRequest;
	get_workflow_source: InputGetWorkflowSourceRequest;
	get_workspace_node_detail: InputGetWorkspaceNodeDetailRequest;
	get_workspace_session_node_id: InputGetWorkspaceSessionNodeIdRequest;
	get_workspace_tree_selection_reconciliation: InputGetWorkspaceTreeSelectionReconciliationRequest;
	git_create_branch: InputGitCreateBranchRequest;
	git_stage: InputGitStageRequest;
	git_stage_review_group: InputGitStageReviewGroupRequest;
	git_unstage: InputGitUnstageRequest;
	git_unstage_review_group: InputGitUnstageReviewGroupRequest;
	kill_terminal_surface: InputKillTerminalSurfaceRequest;
	list_agent_session_history: InputListAgentSessionHistoryRequest;
	list_available_agent_session_providers: InputListAvailableAgentSessionProvidersRequest;
	list_branches: InputListBranchesRequest;
	list_branches_with_status: InputListBranchesWithStatusRequest;
	list_branches_with_status_snapshot: InputListBranchesWithStatusSnapshotRequest;
	list_facet_summaries: InputListFacetSummariesRequest;
	list_provider_hook_health_warnings: InputListProviderHookHealthWarningsRequest;
	list_review_threads: InputListReviewThreadsRequest;
	list_workflows: InputListWorkflowsRequest;
	list_workspace_workflow_history: InputListWorkspaceWorkflowHistoryRequest;
	list_workspace_worktree_nodes: InputListWorkspaceWorktreeNodesRequest;
	list_worktrees: InputListWorktreesRequest;
	load_workspace_state: InputLoadWorkspaceStateRequest;
	open_agent_session: InputOpenAgentSessionRequest;
	open_facet_in_editor: InputOpenFacetInEditorRequest;
	open_folder_in_editor: InputOpenFolderInEditorRequest;
	open_in_editor: InputOpenInEditorRequest;
	open_workflow_in_editor: InputOpenWorkflowInEditorRequest;
	query_notion_tasks: InputQueryNotionTasksRequest;
	quit_after_startup_failure: InputQuitAfterStartupFailureRequest;
	record_terminal_launch_renderer_phase: InputRecordTerminalLaunchRendererPhaseRequest;
	refresh_provider_availability: InputRefreshProviderAvailabilityRequest;
	remove_repo_path: InputRemoveRepoPathRequest;
	remove_worktree: InputRemoveWorktreeRequest;
	rename_workspace_session_node: InputRenameWorkspaceSessionNodeRequest;
	render_facet_preview: InputRenderFacetPreviewRequest;
	report_frontend_error: InputReportFrontendErrorRequest;
	report_mounted_xterm_count: InputReportMountedXtermCountRequest;
	report_usage_event: InputReportUsageEventRequest;
	request_application_quit: InputRequestApplicationQuitRequest;
	reset_provider_executable: InputResetProviderExecutableRequest;
	resize_terminal_surface: InputResizeTerminalSurfaceRequest;
	resolve_active_execution_by_worktree: InputResolveActiveExecutionByWorktreeRequest;
	resolve_review_thread: InputResolveReviewThreadRequest;
	restore_agent_session: InputRestoreAgentSessionRequest;
	restore_workspace_workflow_execution: InputRestoreWorkspaceWorkflowExecutionRequest;
	resume_agent_session: InputResumeAgentSessionRequest;
	resume_agent_session_history_candidate: InputResumeAgentSessionHistoryCandidateRequest;
	resume_workflow: InputResumeWorkflowRequest;
	retry_workspace_node: InputRetryWorkspaceNodeRequest;
	save_facet: InputSaveFacetRequest;
	save_notion_config: InputSaveNotionConfigRequest;
	save_workflow_source: InputSaveWorkflowSourceRequest;
	save_workspace_state: InputSaveWorkspaceStateRequest;
	set_branch_base: InputSetBranchBaseRequest;
	set_releash_base: InputSetReleashBaseRequest;
	start_terminal_input_performance_collection: InputStartTerminalInputPerformanceCollectionRequest;
	start_terminal_launch_performance_collection: InputStartTerminalLaunchPerformanceCollectionRequest;
	start_workflow: InputStartWorkflowRequest;
	stop_watching: InputStopWatchingRequest;
	stop_workflow: InputStopWorkflowRequest;
	take_terminal_input_performance_samples: InputTakeTerminalInputPerformanceSamplesRequest;
	take_terminal_launch_performance_samples: InputTakeTerminalLaunchPerformanceSamplesRequest;
	update_app_settings: InputUpdateAppSettingsRequest;
	update_login_item_preference: InputUpdateLoginItemPreferenceRequest;
	update_crash_reporting: InputUpdateCrashReportingRequest;
	update_external_editor: InputUpdateExternalEditorRequest;
	update_performance_telemetry: InputUpdatePerformanceTelemetryRequest;
	update_provider_executable: InputUpdateProviderExecutableRequest;
	update_workflow_config: InputUpdateWorkflowConfigRequest;
	validate_notion_config: InputValidateNotionConfigRequest;
	write_paths_to_terminal_surface: InputWritePathsToTerminalSurfaceRequest;
	write_terminal_surface: InputWriteTerminalSurfaceRequest;
	get_crash_reporting_enabled: InputGetCrashReportingEnabledRequest;
	get_file_at_ref: InputGetFileAtRefRequest;
	get_staged_content: InputGetStagedContentRequest;
	get_binary_staged_content: InputGetBinaryStagedContentRequest;
	get_file_at_branch_base: InputGetFileAtBranchBaseRequest;
	get_binary_file_at_branch_base: InputGetBinaryFileAtBranchBaseRequest;
	get_binary_file_at_ref: InputGetBinaryFileAtRefRequest;
	get_branch_diff_summary: InputGetBranchDiffSummaryRequest;
	build_diff_file_tree: InputBuildDiffFileTreeRequest;
	get_head_diff_file_tree_snapshot: InputGetHeadDiffFileTreeSnapshotRequest;
	compute_hidden_ranges: InputComputeHiddenRangesRequest;
	get_relative_path: InputGetRelativePathRequest;
	get_review_thread: InputGetReviewThreadRequest;
	get_review_thread_history: InputGetReviewThreadHistoryRequest;
	fetch_pr_status: InputFetchPrStatusRequest;
	get_default_branch: InputGetDefaultBranchRequest;
	get_git_status: InputGetGitStatusRequest;
	get_git_status_snapshot: InputGetGitStatusSnapshotRequest;
	get_status_diff_stats: InputGetStatusDiffStatsRequest;
	get_status_diff_stats_snapshot: InputGetStatusDiffStatsSnapshotRequest;
	get_git_log: InputGetGitLogRequest;
	get_worktree_dirty_count: InputGetWorktreeDirtyCountRequest;
	get_repo_git_dir: InputGetRepoGitDirRequest;
	approve_workflow_node: InputApproveWorkflowNodeRequest;
	list_workflow_executions: InputListWorkflowExecutionsRequest;
	get_workflow_execution: InputGetWorkflowExecutionRequest;
	get_workflow_execution_log: InputGetWorkflowExecutionLogRequest;
	get_workflow_node_detail: InputGetWorkflowNodeDetailRequest;
	resolve_worktree_by_execution: InputResolveWorktreeByExecutionRequest;
	list_facets: InputListFacetsRequest;
	workflow_submit_output: InputWorkflowSubmitOutputRequest;
	workflow_validate_output: InputWorkflowValidateOutputRequest;
	workflow_get_output: InputWorkflowGetOutputRequest;
}

export interface ClientCommands {
	get_review_blob(
		args: ClientCommandArgs["get_review_blob"],
	): Promise<ResultString>;
	abort_workflow(args: ClientCommandArgs["abort_workflow"]): Promise<void>;
	ack_terminal_surface_output(
		args: ClientCommandArgs["ack_terminal_surface_output"],
	): Promise<void>;
	add_repo_path(args: ClientCommandArgs["add_repo_path"]): Promise<ResultBool>;
	append_review_comment(
		args: ClientCommandArgs["append_review_comment"],
	): Promise<ReviewThreadDto>;
	approve_workspace_node(
		args: ClientCommandArgs["approve_workspace_node"],
	): Promise<void>;
	archive_agent_session(
		args: ClientCommandArgs["archive_agent_session"],
	): Promise<AgentSessionArchiveResponse>;
	archive_workspace_workflow_execution(
		args: ClientCommandArgs["archive_workspace_workflow_execution"],
	): Promise<void>;
	build_review_thread_handoff(
		args: ClientCommandArgs["build_review_thread_handoff"],
	): Promise<ResultString>;
	compute_hidden_ranges_from_content(
		args: ClientCommandArgs["compute_hidden_ranges_from_content"],
	): Promise<ListHiddenRangeDto>;
	compute_markdown_diff_ranges(
		args: ClientCommandArgs["compute_markdown_diff_ranges"],
	): Promise<ListDiffRangeDto>;
	compute_markdown_inline_chunks(
		args: ClientCommandArgs["compute_markdown_inline_chunks"],
	): Promise<ListInlineChunkDto>;
	compute_markdown_split_rows(
		args: ClientCommandArgs["compute_markdown_split_rows"],
	): Promise<ListSplitRowDto>;
	compute_visible_markdown_blocks(
		args: ClientCommandArgs["compute_visible_markdown_blocks"],
	): Promise<ListVisibleBlockDto>;
	confirm_agent_session_archive_delete(
		args: ClientCommandArgs["confirm_agent_session_archive_delete"],
	): Promise<void>;
	create_agent_session(
		args: ClientCommandArgs["create_agent_session"],
	): Promise<ResultString>;
	create_review_thread(
		args: ClientCommandArgs["create_review_thread"],
	): Promise<ReviewThreadDto>;
	create_worktree(
		args: ClientCommandArgs["create_worktree"],
	): Promise<WorktreeEntryDto>;
	delete_agent_session(
		args: ClientCommandArgs["delete_agent_session"],
	): Promise<void>;
	delete_branch(args: ClientCommandArgs["delete_branch"]): Promise<void>;
	delete_facet(args: ClientCommandArgs["delete_facet"]): Promise<void>;
	delete_notion_config(
		args: ClientCommandArgs["delete_notion_config"],
	): Promise<void>;
	delete_review_thread(
		args: ClientCommandArgs["delete_review_thread"],
	): Promise<void>;
	delete_workflow(args: ClientCommandArgs["delete_workflow"]): Promise<void>;
	detach_terminal_surface(
		args: ClientCommandArgs["detach_terminal_surface"],
	): Promise<void>;
	detect_editors(
		args: ClientCommandArgs["detect_editors"],
	): Promise<ListEditorInfoDto>;
	diagnose_all_cmd(
		args: ClientCommandArgs["diagnose_all_cmd"],
	): Promise<DiagnosticReport>;
	duplicate_facet(args: ClientCommandArgs["duplicate_facet"]): Promise<void>;
	duplicate_workflow(
		args: ClientCommandArgs["duplicate_workflow"],
	): Promise<void>;
	fetch_issues(
		args: ClientCommandArgs["fetch_issues"],
	): Promise<ListIssueInfoDto>;
	fetch_notion_label_options(
		args: ClientCommandArgs["fetch_notion_label_options"],
	): Promise<ListNotionLabelOptionView>;
	get_agent_session(
		args: ClientCommandArgs["get_agent_session"],
	): Promise<NullableAgentSessionItemDto>;
	get_app_settings(
		args: ClientCommandArgs["get_app_settings"],
	): Promise<AppSection>;
	get_application_startup_outcome(
		args: ClientCommandArgs["get_application_startup_outcome"],
	): Promise<ApplicationStartupOutcomeDtoV1>;
	get_automation_config_dir(
		args: ClientCommandArgs["get_automation_config_dir"],
	): Promise<ResultString>;
	get_branch_base(
		args: ClientCommandArgs["get_branch_base"],
	): Promise<Nullablestring>;
	get_cached_issues(
		args: ClientCommandArgs["get_cached_issues"],
	): Promise<ListIssueInfoDto>;
	get_cached_pr_status(
		args: ClientCommandArgs["get_cached_pr_status"],
	): Promise<PrStatusDto>;
	get_current_branch(
		args: ClientCommandArgs["get_current_branch"],
	): Promise<ResultString>;
	get_cwd(args: ClientCommandArgs["get_cwd"]): Promise<ResultString>;
	get_external_editor(
		args: ClientCommandArgs["get_external_editor"],
	): Promise<ResultString>;
	get_facet(args: ClientCommandArgs["get_facet"]): Promise<ResultString>;
	get_file_navigation(
		args: ClientCommandArgs["get_file_navigation"],
	): Promise<FileNavigationResultDto>;
	get_language_from_path(
		args: ClientCommandArgs["get_language_from_path"],
	): Promise<ResultString>;
	get_main_repo_path(
		args: ClientCommandArgs["get_main_repo_path"],
	): Promise<ResultString>;
	get_notion_config(
		args: ClientCommandArgs["get_notion_config"],
	): Promise<NullableNotionRepoConfigView>;
	get_or_spawn_terminal_surface(
		args: ClientCommandArgs["get_or_spawn_terminal_surface"],
	): Promise<GetOrSpawnTerminalV1>;
	get_performance_real_app_mode(
		args: ClientCommandArgs["get_performance_real_app_mode"],
	): Promise<ResultBool>;
	get_performance_telemetry_enabled(
		args: ClientCommandArgs["get_performance_telemetry_enabled"],
	): Promise<ResultBool>;
	get_provider_availability(
		args: ClientCommandArgs["get_provider_availability"],
	): Promise<ProviderAvailabilitySnapshotResponse>;
	get_releash_base(
		args: ClientCommandArgs["get_releash_base"],
	): Promise<Nullablestring>;
	get_repo_paths(
		args: ClientCommandArgs["get_repo_paths"],
	): Promise<Liststring>;
	get_review_file_view(
		args: ClientCommandArgs["get_review_file_view"],
	): Promise<ReviewFileViewDto>;
	get_review_snapshot(
		args: ClientCommandArgs["get_review_snapshot"],
	): Promise<ReviewSnapshotDto>;
	get_terminal_performance_switches(
		args: ClientCommandArgs["get_terminal_performance_switches"],
	): Promise<TerminalPerformanceSwitchesV1>;
	get_terminal_surface(
		args: ClientCommandArgs["get_terminal_surface"],
	): Promise<TerminalSurfaceSummaryV1>;
	get_workflow(args: ClientCommandArgs["get_workflow"]): Promise<WorkflowDto>;
	get_workflow_config(
		args: ClientCommandArgs["get_workflow_config"],
	): Promise<WorkflowSection>;
	get_workflow_execution_state(
		args: ClientCommandArgs["get_workflow_execution_state"],
	): Promise<NullableWorkflowExecutionView>;
	get_workflow_source(
		args: ClientCommandArgs["get_workflow_source"],
	): Promise<ResultString>;
	get_workspace_node_detail(
		args: ClientCommandArgs["get_workspace_node_detail"],
	): Promise<NullableWorkspaceNodeDetailDto>;
	get_workspace_session_node_id(
		args: ClientCommandArgs["get_workspace_session_node_id"],
	): Promise<Nullablestring>;
	get_workspace_tree_selection_reconciliation(
		args: ClientCommandArgs["get_workspace_tree_selection_reconciliation"],
	): Promise<WorkspaceTreeSelectionSnapshotDto>;
	git_create_branch(
		args: ClientCommandArgs["git_create_branch"],
	): Promise<void>;
	git_stage(args: ClientCommandArgs["git_stage"]): Promise<void>;
	git_stage_review_group(
		args: ClientCommandArgs["git_stage_review_group"],
	): Promise<void>;
	git_unstage(args: ClientCommandArgs["git_unstage"]): Promise<void>;
	git_unstage_review_group(
		args: ClientCommandArgs["git_unstage_review_group"],
	): Promise<void>;
	kill_terminal_surface(
		args: ClientCommandArgs["kill_terminal_surface"],
	): Promise<void>;
	list_agent_session_history(
		args: ClientCommandArgs["list_agent_session_history"],
	): Promise<AgentSessionHistoryPageDto>;
	list_available_agent_session_providers(
		args: ClientCommandArgs["list_available_agent_session_providers"],
	): Promise<ListAgentSessionProviderDto>;
	list_branches(
		args: ClientCommandArgs["list_branches"],
	): Promise<ListBranchDto>;
	list_branches_with_status(
		args: ClientCommandArgs["list_branches_with_status"],
	): Promise<ListBranchCardDto>;
	list_branches_with_status_snapshot(
		args: ClientCommandArgs["list_branches_with_status_snapshot"],
	): Promise<RepositoryBranchCardsSnapshotDto>;
	list_facet_summaries(
		args: ClientCommandArgs["list_facet_summaries"],
	): Promise<ListFacetSummaryDto>;
	list_provider_hook_health_warnings(
		args: ClientCommandArgs["list_provider_hook_health_warnings"],
	): Promise<ListProviderHookHealthWarningResponse>;
	list_review_threads(
		args: ClientCommandArgs["list_review_threads"],
	): Promise<ListReviewThreadDto>;
	list_workflows(
		args: ClientCommandArgs["list_workflows"],
	): Promise<ListWorkflowSummaryDto>;
	list_workspace_workflow_history(
		args: ClientCommandArgs["list_workspace_workflow_history"],
	): Promise<ListWorkspaceWorkflowHistoryItemDto>;
	list_workspace_worktree_nodes(
		args: ClientCommandArgs["list_workspace_worktree_nodes"],
	): Promise<WorkspaceTreeSnapshotDto>;
	list_worktrees(
		args: ClientCommandArgs["list_worktrees"],
	): Promise<ListWorktreeEntryDto>;
	load_workspace_state(
		args: ClientCommandArgs["load_workspace_state"],
	): Promise<NullableWorkspaceStateDto>;
	open_agent_session(
		args: ClientCommandArgs["open_agent_session"],
	): Promise<AgentSessionOpenResponse>;
	open_facet_in_editor(
		args: ClientCommandArgs["open_facet_in_editor"],
	): Promise<void>;
	open_folder_in_editor(
		args: ClientCommandArgs["open_folder_in_editor"],
	): Promise<void>;
	open_in_editor(args: ClientCommandArgs["open_in_editor"]): Promise<void>;
	open_workflow_in_editor(
		args: ClientCommandArgs["open_workflow_in_editor"],
	): Promise<void>;
	query_notion_tasks(
		args: ClientCommandArgs["query_notion_tasks"],
	): Promise<NotionTaskPageView>;
	quit_after_startup_failure(
		args: ClientCommandArgs["quit_after_startup_failure"],
	): Promise<StartupFailureQuitOutcomeDtoV1>;
	record_terminal_launch_renderer_phase(
		args: ClientCommandArgs["record_terminal_launch_renderer_phase"],
	): Promise<void>;
	refresh_provider_availability(
		args: ClientCommandArgs["refresh_provider_availability"],
	): Promise<ProviderAvailabilitySnapshotResponse>;
	remove_repo_path(
		args: ClientCommandArgs["remove_repo_path"],
	): Promise<ResultBool>;
	remove_worktree(args: ClientCommandArgs["remove_worktree"]): Promise<void>;
	rename_workspace_session_node(
		args: ClientCommandArgs["rename_workspace_session_node"],
	): Promise<void>;
	render_facet_preview(
		args: ClientCommandArgs["render_facet_preview"],
	): Promise<ResultString>;
	report_frontend_error(
		args: ClientCommandArgs["report_frontend_error"],
	): Promise<void>;
	report_mounted_xterm_count(
		args: ClientCommandArgs["report_mounted_xterm_count"],
	): Promise<void>;
	report_usage_event(
		args: ClientCommandArgs["report_usage_event"],
	): Promise<void>;
	request_application_quit(
		args: ClientCommandArgs["request_application_quit"],
	): Promise<ApplicationQuitOutcomeDtoV1>;
	reset_provider_executable(
		args: ClientCommandArgs["reset_provider_executable"],
	): Promise<ProviderAvailabilitySnapshotResponse>;
	resize_terminal_surface(
		args: ClientCommandArgs["resize_terminal_surface"],
	): Promise<void>;
	resolve_active_execution_by_worktree(
		args: ClientCommandArgs["resolve_active_execution_by_worktree"],
	): Promise<Nullablestring>;
	resolve_review_thread(
		args: ClientCommandArgs["resolve_review_thread"],
	): Promise<ReviewThreadDto>;
	restore_agent_session(
		args: ClientCommandArgs["restore_agent_session"],
	): Promise<AgentSessionOpenResponse>;
	restore_workspace_workflow_execution(
		args: ClientCommandArgs["restore_workspace_workflow_execution"],
	): Promise<void>;
	resume_agent_session(
		args: ClientCommandArgs["resume_agent_session"],
	): Promise<AgentSessionOpenResponse>;
	resume_agent_session_history_candidate(
		args: ClientCommandArgs["resume_agent_session_history_candidate"],
	): Promise<ResultString>;
	resume_workflow(args: ClientCommandArgs["resume_workflow"]): Promise<void>;
	retry_workspace_node(
		args: ClientCommandArgs["retry_workspace_node"],
	): Promise<void>;
	save_facet(args: ClientCommandArgs["save_facet"]): Promise<void>;
	save_notion_config(
		args: ClientCommandArgs["save_notion_config"],
	): Promise<void>;
	save_workflow_source(
		args: ClientCommandArgs["save_workflow_source"],
	): Promise<SaveWorkflowSourceResultDto>;
	save_workspace_state(
		args: ClientCommandArgs["save_workspace_state"],
	): Promise<void>;
	set_branch_base(args: ClientCommandArgs["set_branch_base"]): Promise<void>;
	set_releash_base(args: ClientCommandArgs["set_releash_base"]): Promise<void>;
	start_terminal_input_performance_collection(
		args: ClientCommandArgs["start_terminal_input_performance_collection"],
	): Promise<void>;
	start_terminal_launch_performance_collection(
		args: ClientCommandArgs["start_terminal_launch_performance_collection"],
	): Promise<void>;
	start_workflow(
		args: ClientCommandArgs["start_workflow"],
	): Promise<ResultString>;
	stop_watching(args: ClientCommandArgs["stop_watching"]): Promise<void>;
	stop_workflow(args: ClientCommandArgs["stop_workflow"]): Promise<void>;
	take_terminal_input_performance_samples(
		args: ClientCommandArgs["take_terminal_input_performance_samples"],
	): Promise<ListTerminalInputPerformanceSampleV1>;
	take_terminal_launch_performance_samples(
		args: ClientCommandArgs["take_terminal_launch_performance_samples"],
	): Promise<ListTerminalLaunchPerformanceSampleV1>;
	update_app_settings(
		args: ClientCommandArgs["update_app_settings"],
	): Promise<void>;
	update_login_item_preference(
		args: ClientCommandArgs["update_login_item_preference"],
	): Promise<void>;
	update_crash_reporting(
		args: ClientCommandArgs["update_crash_reporting"],
	): Promise<void>;
	update_external_editor(
		args: ClientCommandArgs["update_external_editor"],
	): Promise<void>;
	update_performance_telemetry(
		args: ClientCommandArgs["update_performance_telemetry"],
	): Promise<void>;
	update_provider_executable(
		args: ClientCommandArgs["update_provider_executable"],
	): Promise<ProviderAvailabilitySnapshotResponse>;
	update_workflow_config(
		args: ClientCommandArgs["update_workflow_config"],
	): Promise<void>;
	validate_notion_config(
		args: ClientCommandArgs["validate_notion_config"],
	): Promise<NotionValidationResultView>;
	write_paths_to_terminal_surface(
		args: ClientCommandArgs["write_paths_to_terminal_surface"],
	): Promise<void>;
	write_terminal_surface(
		args: ClientCommandArgs["write_terminal_surface"],
	): Promise<void>;
	get_crash_reporting_enabled(
		args: ClientCommandArgs["get_crash_reporting_enabled"],
	): Promise<ResultBool>;
	get_file_at_ref(
		args: ClientCommandArgs["get_file_at_ref"],
	): Promise<ResultString>;
	get_staged_content(
		args: ClientCommandArgs["get_staged_content"],
	): Promise<ResultString>;
	get_binary_staged_content(
		args: ClientCommandArgs["get_binary_staged_content"],
	): Promise<ResultString>;
	get_file_at_branch_base(
		args: ClientCommandArgs["get_file_at_branch_base"],
	): Promise<ResultString>;
	get_binary_file_at_branch_base(
		args: ClientCommandArgs["get_binary_file_at_branch_base"],
	): Promise<ResultString>;
	get_binary_file_at_ref(
		args: ClientCommandArgs["get_binary_file_at_ref"],
	): Promise<ResultString>;
	get_branch_diff_summary(
		args: ClientCommandArgs["get_branch_diff_summary"],
	): Promise<BranchDiffSummaryDto>;
	build_diff_file_tree(
		args: ClientCommandArgs["build_diff_file_tree"],
	): Promise<ListDiffTreeNodeDto>;
	get_head_diff_file_tree_snapshot(
		args: ClientCommandArgs["get_head_diff_file_tree_snapshot"],
	): Promise<RepositoryHeadDiffFileTreeSnapshotDto>;
	compute_hidden_ranges(
		args: ClientCommandArgs["compute_hidden_ranges"],
	): Promise<ListHiddenRangeDto>;
	get_relative_path(
		args: ClientCommandArgs["get_relative_path"],
	): Promise<Nullablestring>;
	get_review_thread(
		args: ClientCommandArgs["get_review_thread"],
	): Promise<ReviewThreadDto>;
	get_review_thread_history(
		args: ClientCommandArgs["get_review_thread_history"],
	): Promise<ListReviewHistoryEntryDto>;
	fetch_pr_status(
		args: ClientCommandArgs["fetch_pr_status"],
	): Promise<PrStatusDto>;
	get_default_branch(
		args: ClientCommandArgs["get_default_branch"],
	): Promise<ResultString>;
	get_git_status(
		args: ClientCommandArgs["get_git_status"],
	): Promise<ListFileStatusDto>;
	get_git_status_snapshot(
		args: ClientCommandArgs["get_git_status_snapshot"],
	): Promise<RepositoryStatusSnapshotDto>;
	get_status_diff_stats(
		args: ClientCommandArgs["get_status_diff_stats"],
	): Promise<ListFileDiffStatDto>;
	get_status_diff_stats_snapshot(
		args: ClientCommandArgs["get_status_diff_stats_snapshot"],
	): Promise<RepositoryDiffStatsSnapshotDto>;
	get_git_log(args: ClientCommandArgs["get_git_log"]): Promise<ListCommitDto>;
	get_worktree_dirty_count(
		args: ClientCommandArgs["get_worktree_dirty_count"],
	): Promise<ResultUint32>;
	get_repo_git_dir(
		args: ClientCommandArgs["get_repo_git_dir"],
	): Promise<ResultString>;
	approve_workflow_node(
		args: ClientCommandArgs["approve_workflow_node"],
	): Promise<void>;
	list_workflow_executions(
		args: ClientCommandArgs["list_workflow_executions"],
	): Promise<ListWorkflowExecutionSummaryDto>;
	get_workflow_execution(
		args: ClientCommandArgs["get_workflow_execution"],
	): Promise<NullableWorkflowExecutionSummaryDto>;
	get_workflow_execution_log(
		args: ClientCommandArgs["get_workflow_execution_log"],
	): Promise<NullableListDurableWorkflowFactLogEntry>;
	get_workflow_node_detail(
		args: ClientCommandArgs["get_workflow_node_detail"],
	): Promise<NullableNodeExecutionView>;
	resolve_worktree_by_execution(
		args: ClientCommandArgs["resolve_worktree_by_execution"],
	): Promise<Nullablestring>;
	list_facets(args: ClientCommandArgs["list_facets"]): Promise<Liststring>;
	workflow_submit_output(
		args: ClientCommandArgs["workflow_submit_output"],
	): Promise<void>;
	workflow_validate_output(
		args: ClientCommandArgs["workflow_validate_output"],
	): Promise<WorkflowValidateOutputResponse>;
	workflow_get_output(
		args: ClientCommandArgs["workflow_get_output"],
	): Promise<WorkflowGetOutputResponse>;
}
export type ClientCommandResults = {
	[K in keyof ClientCommands]: Awaited<ReturnType<ClientCommands[K]>>;
};

export interface ClientPushPayloads {
	"agent-session-changed": AgentSessionChangedPayload;
	"branch-list-sync": null;
	"file-change": FileChangeEvent;
	"git-status-changed": GitStatusChangedEvent;
	"repo-paths-changed": Liststring;
	"repository-snapshot-changed": RepositorySnapshotChangedEvent;
	"review-comments-changed": ResultString;
	"workflow-execution-changed": WorkflowExecutionChangedPayloadView;
	resync: null;
}
