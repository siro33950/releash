// Generated from proto/client.proto. Run pnpm generate:protocol.

export type TerminalEvent = {
	snapshot?: TerminalSnapshot;
	output?: TerminalOutput;
	resize?: TerminalResize;
	exit?: TerminalExit;
};

export type TerminalSnapshot = {
	sessionKey?: string;
	replay?: string;
	sequence?: number;
	cols?: number;
	rows?: number;
	isExited?: boolean;
	exitCode?: number;
	processedReportUnits?: number;
};

export type TerminalOutput = {
	sessionKey?: string;
	data?: string;
	sequence?: number;
};

export type TerminalResize = {
	sessionKey?: string;
	cols?: number;
	rows?: number;
	sequence?: number;
};

export type TerminalExit = {
	sessionKey?: string;
	exitCode?: number;
	sequence?: number;
};

export type Liststring = Array<string>;

export type WorkspaceListSnapshot = {
	status: WorkspaceListStatus;
	repositories: ListWorkspaceRepositoryList;
};

export type WorkspaceListStatus = {
	state: string;
	loaded: boolean;
	error: string | null;
};

export type ListWorkspaceRepositoryList = Array<WorkspaceRepositoryList>;

export type WorkspaceRepositoryList = {
	path: string;
	status: WorkspaceListStatus;
	branches: ListWorkspaceBranch;
	worktrees: ListWorkspaceWorktreeList;
};

export type ListWorkspaceBranch = Array<WorkspaceBranch>;

export type WorkspaceBranch = {
	name: string;
	is_main_worktree: boolean;
	is_deleting: boolean;
	worktree_path: string;
	dirty_count: number | null;
	is_merged: boolean;
	has_pr: boolean | null;
	dirty_count_error: string | null;
	pull_request_error: string | null;
	pr_number: number | null;
	pr_url: string | null;
};

export type ListWorkspaceWorktreeList = Array<WorkspaceWorktreeList>;

export type WorkspaceWorktreeList = {
	path: string;
	status: WorkspaceListStatus;
	snapshot: WorkspaceTreeSnapshot | null;
	workflowHistory: ListWorkspaceWorkflowHistoryItem;
};

export type WorkspaceTreeSnapshot = {
	nodes: ListWorkspaceTreeItem;
	archivedSessions: ListAgentSessionItemDto;
	preferredNodeId?: string;
};

export type ListWorkspaceTreeItem = Array<WorkspaceTreeItem>;

export type WorkspaceTreeItem =
	| ({ kind: "node" } & WorkspaceNode)
	| ({ kind: "sequence" } & WorkspaceSequence)
	| ({ kind: "fanout" } & WorkspaceFanout);

export type WorkspaceNode = {
	processPresence: NodeProcessPresence;
	id: string;
	title: string;
	status: WorkspaceStatusClassification;
	errorReason?: string;
	contentKind: WorkspaceContentKind;
	capabilities: WorkspaceNodeCapabilitiesDto;
	workflowCapabilities?: WorkspaceWorkflowCapabilities;
	sessionCapabilities?: WorkspaceSessionCapabilities;
	children?: ListWorkspaceTreeItem;
	pastAttempts: ListWorkspaceNode;
	pastAttemptsCollapsed: boolean;
	updatedAt: number;
};

export type NodeProcessPresence = "unknown" | "live" | "confirmed_absent";

export type WorkspaceStatusClassification = "active" | "attention" | "idle";

export type WorkspaceContentKind = "session" | "command";

export type WorkspaceNodeCapabilitiesDto = {
	canResumeSession: boolean;
	canRename: boolean;
	canApprove: boolean;
	canRetry: boolean;
};

export type WorkspaceWorkflowCapabilities = {
	canAbort: boolean;
	canArchive: boolean;
};

export type WorkspaceSessionCapabilities = {
	sessionRef: string;
	canArchive: boolean;
	canDelete: boolean;
};

export type ListWorkspaceNode = Array<WorkspacePastAttempt>;

export type WorkspacePastAttempt = { kind: "node" } & WorkspaceNode;

export type WorkspaceSequence = {
	worktree?: NodeWorktreeDto;
	id: string;
	title: string;
	status: WorkspaceStatusClassification;
	workflowCapabilities?: WorkspaceWorkflowCapabilities;
	children: ListWorkspaceTreeItem;
	updatedAt: number;
};

export type NodeWorktreeDto = {
	branch: string;
	path: string;
};

export type WorkspaceFanout = {
	worktree?: NodeWorktreeDto;
	id: string;
	title: string;
	status: WorkspaceStatusClassification;
	workflowCapabilities?: WorkspaceWorkflowCapabilities;
	children: ListWorkspaceTreeItem;
	updatedAt: number;
};

export type ListAgentSessionItemDto = Array<AgentSessionItemDto>;

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
	terminalPresence?: string;
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
};

export type ListWorkspaceWorkflowHistoryItem =
	Array<WorkspaceWorkflowHistoryItem>;

export type WorkspaceWorkflowHistoryItem = {
	executionId: string;
	worktreePath: string;
	title: string;
	status: WorkspaceHistoryStatus;
	updatedAt: number;
	archivedAt: number;
	archiveReason: string;
};

export type WorkspaceHistoryStatus =
	| "running"
	| "waiting"
	| "aborted"
	| "completed";

export type WorkspaceTreeSelectionSnapshot = {
	snapshot: WorkspaceTreeSnapshot;
	reconciliation: WorkspaceSelectionReconciliation;
};

export type WorkspaceSelectionReconciliation = {
	selectionInSnapshot: boolean;
};

export type NullableWorkspaceNodeDetailDto = WorkspaceNodeDetailDto | null;

export type WorkspaceNodeDetailDto = {
	processPresence: NodeProcessPresence;
	worktree?: NodeWorktreeDto;
	id: string;
	title: string;
	status: WorkspaceNodeStatus;
	submitReceived: boolean;
	stopReceived: boolean;
	waitingFor?: WorkspaceWaitingFor;
	hasArtifact: boolean;
	errorReason?: string;
	capabilities: WorkspaceNodeCapabilitiesDto;
	updatedAt: number;
	content: WorkspaceNodeContentDto;
};

export type WorkspaceNodeStatus =
	| "running"
	| "waiting"
	| "aborted"
	| "completed";

export type WorkspaceWaitingFor = "submit" | "stop";

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

export type NullableAgentSessionItemDto = AgentSessionItemDto | null;

export type AgentSessionHistoryPageDto = {
	items: ListAgentSessionHistoryCandidateDto;
	hasMore: boolean;
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

export type Nullablestring = string | null;

export type ListBranchStatus = Array<BranchStatus>;

export type BranchStatus = {
	name: string;
	has_worktree: boolean;
};

export type ResultString = string;

export type IssuesSnapshot = {
	issues?: ListIssueInfoDto;
	readError?: string;
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

export type NotionTasksSnapshot = {
	page?: NotionTaskPageView;
	readError?: NotionReadFailure;
};

export type NotionTaskPageView = {
	tasks: ListNotionTaskView;
	has_more: boolean;
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

export type NotionReadFailure = {
	code?: number;
	message?: string;
	configMissing?: boolean;
};

export type NotionLabelOptionsSnapshot = {
	options?: ListNotionLabelOptionView;
	readError?: NotionReadFailure;
};

export type ListNotionLabelOptionView = Array<NotionLabelOptionView>;

export type NotionLabelOptionView = {
	property_name: string;
	property_type: string;
	options: Liststring;
	option_ids: Liststring;
};

export type ListWorktreeEntryDto = Array<WorktreeEntryDto>;

export type WorktreeEntryDto = {
	name: string;
	path: string;
	branch: string;
	is_main: boolean;
	is_locked: boolean;
};

export type NullableStartupWorktree = StartupWorktree | null;

export type StartupWorktree = {
	path: string;
	branch: string;
	repositoryName: string;
};

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

export type ReviewSnapshotDto = {
	version: number;
	stale: boolean;
	loading: boolean;
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

export type ListReviewThreadDto = Array<ReviewThreadDto>;

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

export type ListWorkflowSummaryDto = Array<WorkflowSummaryDto>;

export type WorkflowSummaryDto = {
	readError?: string;
	name: string;
	description: string;
	builtin: boolean;
	is_running: boolean;
	sourceFormat: WorkflowSourceFormat;
};

export type WorkflowSourceFormat = "yaml" | "lua";

export type NullableWorkflowDto = WorkflowDto | null;

export type WorkflowDto = {
	name: string;
	description: string;
	builtin: boolean;
	sourceFormat: WorkflowSourceFormat;
	schemas?: WorkflowValueObject;
	nodes: ListNodeDefinitionDto;
};

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

export type ResultBool = boolean;

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

export type ListFacetSummaryDto = Array<FacetSummaryDto>;

export type FacetSummaryDto = {
	key: string;
	kind: string;
	description: string;
	builtin: boolean;
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

export type DesktopSettings = {
	closeToTray: boolean;
	startMinimized: boolean;
	crashReporting: boolean;
	performanceTelemetry: boolean;
	autoLaunch: boolean;
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
	configurationRevision: number;
};

export type ExternalEditorState = {
	selected: string;
	editors: ListEditorInfoDto;
};

export type ListEditorInfoDto = Array<EditorInfoDto>;

export type EditorInfoDto = {
	name: string;
	path: string;
};

export type WorkflowSection = {
	approval_auto_approve: boolean;
};

export type NullableWorkflowExecutionView = {
	value?: WorkflowExecutionView;
};

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
	canResumeSession: boolean;
	processPresence: NodeProcessPresence;
	worktree?: NodeWorktreeDto;
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
	parent?: ExecutionParentRefView;
	startedAt: number;
	completedAt?: number;
};

export type NodeKindView = "command" | "session" | "fanout" | "sequence";

export type NodeExecutionStatusView =
	| "running"
	| "waiting_approval"
	| "succeeded"
	| "aborted";

export type NodeCompletionSignalView = "submit" | "stop";

export type ArtifactView = {
	nodeName: string;
	contract?: string;
	value: WorkflowValue;
	producedAt: number;
};

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

export type NullableWorkflowOutputView = {
	value?: WorkflowOutputView;
};

export type WorkflowOutputView =
	| ({ status: "submitted" } & WorkflowOutputViewSubmitted)
	| { status: "not_submitted" };

export type WorkflowOutputViewSubmitted = {
	contract: string | null;
	structured_output: WorkflowValue;
	submitted_at?: number;
	request_id?: string;
	timestamp: number;
};

export type NullableListReviewThreadDto = {
	value?: ListReviewThreadDto;
};

export type NullableReviewThreadDto = {
	value?: ReviewThreadDto;
};

export type NullableListReviewHistoryEntryDto = {
	value?: ListReviewHistoryEntryDto;
};

export type ListReviewHistoryEntryDto = {
	items?: Array<ReviewHistoryEntryDto>;
};

export type ReviewHistoryEntryDto = {
	threadCreated?: ReviewHistoryThreadCreatedDto;
	commentAppended?: ReviewHistoryCommentAppendedDto;
	threadResolved?: ReviewHistoryThreadResolvedDto;
	threadDeleted?: ReviewHistoryThreadDeletedDto;
};

export type ReviewHistoryThreadCreatedDto = {
	id?: string;
	threadId?: string;
	commentId?: string;
	actor?: ReviewActorWireDto;
	target?: ReviewTargetWireDto;
	content?: string;
	at?: number;
};

export type ReviewHistoryCommentAppendedDto = {
	id?: string;
	threadId?: string;
	commentId?: string;
	actor?: ReviewActorWireDto;
	content?: string;
	at?: number;
};

export type ReviewHistoryThreadResolvedDto = {
	id?: string;
	threadId?: string;
	actor?: ReviewActorWireDto;
	outcome?: string;
	summary?: string;
	at?: number;
};

export type ReviewHistoryThreadDeletedDto = {
	id?: string;
	threadId?: string;
	actor?: ReviewActorWireDto;
	at?: number;
};

export type ProviderHookHealthSnapshot = {
	warnings: ListProviderHookHealthWarningResponse;
	readErrors?: Array<string>;
};

export type ListProviderHookHealthWarningResponse =
	Array<ProviderHookHealthWarningResponse>;

export type ProviderHookHealthWarningResponse = {
	provider: ProviderHookHealthProviderResponse;
	launchId: string;
	reason: string;
};

export type ProviderHookHealthProviderResponse = "claude" | "codex";

export type ServerInfo = {
	release?: string;
	daemonId?: string;
	pid?: number;
	processStartedAt?: number;
	protocol?: number;
	capabilities?: Array<string>;
	servingStatus?:
		| "SERVING_STATUS_UNSPECIFIED"
		| "SERVING_STATUS_STARTING"
		| "SERVING_STATUS_SERVING"
		| "SERVING_STATUS_STOPPING"
		| "SERVING_STATUS_STOPPED"
		| "SERVING_STATUS_FAILED";
};

export type InputResolveSessionReviewThreadRequest = {
	sessionId: string;
	threadId: string;
	outcome: string;
	summary: string;
};

export type InputAppendSessionReviewCommentRequest = {
	sessionId: string;
	threadId: string;
	content: string;
};

export type InputCreateSessionReviewThreadRequest = {
	sessionId: string;
	filePath?: string;
	lineNumber?: number;
	endLine?: number;
	content: string;
};

export type InputDiagnoseWorkflowDirectoryRequest = {
	dir: string;
};

export type InputAbortWorkflowRequest = {
	executionId: string;
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

export type InputGetOrSpawnTerminalSurfaceRequest = {
	rows: number;
	cols: number;
	cwd?: string | null;
	owner: InputTerminalSurfaceOwnerV1;
	label?: string | null;
	startupCommand?: string | null;
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

export type InputStopDaemonRequest = Record<string, never>;

export type InputResetProviderExecutableRequest = {
	provider: string;
};

export type InputResizeTerminalSurfaceRequest = {
	owner: InputTerminalSurfaceOwnerV1;
	rows: number;
	cols: number;
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

export type InputResumeAgentSessionHistoryCandidateRequest = {
	workspaceIdentity: string;
	worktreePath: string;
	provider: string;
	providerSessionId: string;
	rows: number;
	cols: number;
	callerRequestId: string;
};

export type InputRetryWorkspaceNodeRequest = {
	worktreePath: string;
	nodeId: string;
};

export type InputResumeWorkspaceSessionNodeRequest = {
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

export type InputStartWorkflowRequest = {
	workflowName: string;
	worktreePath: string;
	request?: string | null;
	createdFrom?: string | null;
};

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
	data: string;
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

export type InputApproveWorkflowNodeRequest = {
	args: InputApproveWorkflowNodeArgs;
};

export type InputApproveWorkflowNodeArgs = {
	executionId: string;
	nodeName: string;
	nodeExecutionId?: string | null;
	comment?: string | null;
};

export type InputWorkflowSubmitOutputRequest = {
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

export type InputRefreshWorkspacesRequest = Record<string, never>;

export type InputFindRepositoryRootRequest = {
	path: string;
};

export type ResolveSessionReviewThreadResponse = {
	thread?: ReviewThreadDto;
};

export type AppendSessionReviewCommentResponse = {
	thread?: ReviewThreadDto;
};

export type CreateSessionReviewThreadResponse = {
	thread?: ReviewThreadDto;
};

export type DiagnoseWorkflowDirectoryResponse = {
	report?: DiagnosticReport;
};

export type AgentSessionArchiveResponse = "archived" | "already_archived";

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

export type SessionSelection = {
	agentSessionId: string;
	nodeId: string;
};

export type FileNavigationResultDto = {
	current_index: number;
	total: number;
	prev_file: string | null;
	next_file: string | null;
};

export type GetOrSpawnTerminalV1 = {
	session_key: string;
};

export type SaveWorkflowSourceResultDto =
	| SaveWorkflowSuccess
	| SaveWorkflowDiagnostics;

export type SaveWorkflowSuccess = {
	ok: true;
	name: string;
};

export type SaveWorkflowDiagnostics = {
	ok: false;
	error: string;
	diagnostics: ListDiagnosticItem;
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

export interface ClientCommandArgs {
	resolve_session_review_thread: InputResolveSessionReviewThreadRequest;
	append_session_review_comment: InputAppendSessionReviewCommentRequest;
	create_session_review_thread: InputCreateSessionReviewThreadRequest;
	diagnose_workflow_directory: InputDiagnoseWorkflowDirectoryRequest;
	abort_workflow: InputAbortWorkflowRequest;
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
	create_agent_session: InputCreateAgentSessionRequest;
	create_review_thread: InputCreateReviewThreadRequest;
	create_worktree: InputCreateWorktreeRequest;
	delete_agent_session: InputDeleteAgentSessionRequest;
	delete_facet: InputDeleteFacetRequest;
	delete_notion_config: InputDeleteNotionConfigRequest;
	delete_review_thread: InputDeleteReviewThreadRequest;
	delete_workflow: InputDeleteWorkflowRequest;
	duplicate_facet: InputDuplicateFacetRequest;
	duplicate_workflow: InputDuplicateWorkflowRequest;
	fetch_issues: InputFetchIssuesRequest;
	get_file_navigation: InputGetFileNavigationRequest;
	get_language_from_path: InputGetLanguageFromPathRequest;
	get_or_spawn_terminal_surface: InputGetOrSpawnTerminalSurfaceRequest;
	git_create_branch: InputGitCreateBranchRequest;
	git_stage: InputGitStageRequest;
	git_stage_review_group: InputGitStageReviewGroupRequest;
	git_unstage: InputGitUnstageRequest;
	git_unstage_review_group: InputGitUnstageReviewGroupRequest;
	kill_terminal_surface: InputKillTerminalSurfaceRequest;
	open_agent_session: InputOpenAgentSessionRequest;
	open_facet_in_editor: InputOpenFacetInEditorRequest;
	open_folder_in_editor: InputOpenFolderInEditorRequest;
	open_in_editor: InputOpenInEditorRequest;
	open_workflow_in_editor: InputOpenWorkflowInEditorRequest;
	refresh_provider_availability: InputRefreshProviderAvailabilityRequest;
	remove_repo_path: InputRemoveRepoPathRequest;
	remove_worktree: InputRemoveWorktreeRequest;
	rename_workspace_session_node: InputRenameWorkspaceSessionNodeRequest;
	render_facet_preview: InputRenderFacetPreviewRequest;
	report_frontend_error: InputReportFrontendErrorRequest;
	report_mounted_xterm_count: InputReportMountedXtermCountRequest;
	report_usage_event: InputReportUsageEventRequest;
	stop_daemon: InputStopDaemonRequest;
	reset_provider_executable: InputResetProviderExecutableRequest;
	resize_terminal_surface: InputResizeTerminalSurfaceRequest;
	resolve_review_thread: InputResolveReviewThreadRequest;
	restore_agent_session: InputRestoreAgentSessionRequest;
	restore_workspace_workflow_execution: InputRestoreWorkspaceWorkflowExecutionRequest;
	resume_agent_session_history_candidate: InputResumeAgentSessionHistoryCandidateRequest;
	retry_workspace_node: InputRetryWorkspaceNodeRequest;
	resume_workspace_session_node: InputResumeWorkspaceSessionNodeRequest;
	save_facet: InputSaveFacetRequest;
	save_notion_config: InputSaveNotionConfigRequest;
	save_workflow_source: InputSaveWorkflowSourceRequest;
	save_workspace_state: InputSaveWorkspaceStateRequest;
	set_branch_base: InputSetBranchBaseRequest;
	set_releash_base: InputSetReleashBaseRequest;
	start_workflow: InputStartWorkflowRequest;
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
	build_diff_file_tree: InputBuildDiffFileTreeRequest;
	compute_hidden_ranges: InputComputeHiddenRangesRequest;
	approve_workflow_node: InputApproveWorkflowNodeRequest;
	workflow_submit_output: InputWorkflowSubmitOutputRequest;
	refresh_workspaces: InputRefreshWorkspacesRequest;
	find_repository_root: InputFindRepositoryRootRequest;
}

export interface ClientCommands {
	resolve_session_review_thread(
		args: ClientCommandArgs["resolve_session_review_thread"],
	): Promise<ResolveSessionReviewThreadResponse>;
	append_session_review_comment(
		args: ClientCommandArgs["append_session_review_comment"],
	): Promise<AppendSessionReviewCommentResponse>;
	create_session_review_thread(
		args: ClientCommandArgs["create_session_review_thread"],
	): Promise<CreateSessionReviewThreadResponse>;
	diagnose_workflow_directory(
		args: ClientCommandArgs["diagnose_workflow_directory"],
	): Promise<DiagnoseWorkflowDirectoryResponse>;
	abort_workflow(args: ClientCommandArgs["abort_workflow"]): Promise<void>;
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
	create_agent_session(
		args: ClientCommandArgs["create_agent_session"],
	): Promise<SessionSelection>;
	create_review_thread(
		args: ClientCommandArgs["create_review_thread"],
	): Promise<ReviewThreadDto>;
	create_worktree(
		args: ClientCommandArgs["create_worktree"],
	): Promise<ResultString>;
	delete_agent_session(
		args: ClientCommandArgs["delete_agent_session"],
	): Promise<void>;
	delete_facet(args: ClientCommandArgs["delete_facet"]): Promise<void>;
	delete_notion_config(
		args: ClientCommandArgs["delete_notion_config"],
	): Promise<void>;
	delete_review_thread(
		args: ClientCommandArgs["delete_review_thread"],
	): Promise<void>;
	delete_workflow(args: ClientCommandArgs["delete_workflow"]): Promise<void>;
	duplicate_facet(args: ClientCommandArgs["duplicate_facet"]): Promise<void>;
	duplicate_workflow(
		args: ClientCommandArgs["duplicate_workflow"],
	): Promise<void>;
	fetch_issues(args: ClientCommandArgs["fetch_issues"]): Promise<void>;
	get_file_navigation(
		args: ClientCommandArgs["get_file_navigation"],
	): Promise<FileNavigationResultDto>;
	get_language_from_path(
		args: ClientCommandArgs["get_language_from_path"],
	): Promise<ResultString>;
	get_or_spawn_terminal_surface(
		args: ClientCommandArgs["get_or_spawn_terminal_surface"],
	): Promise<GetOrSpawnTerminalV1>;
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
	open_agent_session(
		args: ClientCommandArgs["open_agent_session"],
	): Promise<void>;
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
	refresh_provider_availability(
		args: ClientCommandArgs["refresh_provider_availability"],
	): Promise<void>;
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
	stop_daemon(args: ClientCommandArgs["stop_daemon"]): Promise<void>;
	reset_provider_executable(
		args: ClientCommandArgs["reset_provider_executable"],
	): Promise<void>;
	resize_terminal_surface(
		args: ClientCommandArgs["resize_terminal_surface"],
	): Promise<void>;
	resolve_review_thread(
		args: ClientCommandArgs["resolve_review_thread"],
	): Promise<ReviewThreadDto>;
	restore_agent_session(
		args: ClientCommandArgs["restore_agent_session"],
	): Promise<SessionSelection>;
	restore_workspace_workflow_execution(
		args: ClientCommandArgs["restore_workspace_workflow_execution"],
	): Promise<void>;
	resume_agent_session_history_candidate(
		args: ClientCommandArgs["resume_agent_session_history_candidate"],
	): Promise<SessionSelection>;
	retry_workspace_node(
		args: ClientCommandArgs["retry_workspace_node"],
	): Promise<void>;
	resume_workspace_session_node(
		args: ClientCommandArgs["resume_workspace_session_node"],
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
	start_workflow(
		args: ClientCommandArgs["start_workflow"],
	): Promise<ResultString>;
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
	): Promise<void>;
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
	build_diff_file_tree(
		args: ClientCommandArgs["build_diff_file_tree"],
	): Promise<ListDiffTreeNodeDto>;
	compute_hidden_ranges(
		args: ClientCommandArgs["compute_hidden_ranges"],
	): Promise<ListHiddenRangeDto>;
	approve_workflow_node(
		args: ClientCommandArgs["approve_workflow_node"],
	): Promise<void>;
	workflow_submit_output(
		args: ClientCommandArgs["workflow_submit_output"],
	): Promise<void>;
	refresh_workspaces(
		args: ClientCommandArgs["refresh_workspaces"],
	): Promise<void>;
	find_repository_root(
		args: ClientCommandArgs["find_repository_root"],
	): Promise<Nullablestring>;
}
export type ClientCommandResults = {
	[K in keyof ClientCommands]: Awaited<ReturnType<ClientCommands[K]>>;
};
