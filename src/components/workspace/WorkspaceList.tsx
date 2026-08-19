import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
	Ban,
	Bot,
	ChevronDown,
	ChevronRight,
	ExternalLink,
	GitBranch,
	GitPullRequest,
	Home,
	Loader2,
	MessageSquare,
	MoreHorizontal,
	Plus,
	RefreshCw,
	RotateCcw,
	Settings,
	Square,
	Terminal,
	Trash2,
	Workflow,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuSub,
	DropdownMenuSubContent,
	DropdownMenuSubTrigger,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Textarea } from "@/components/ui/textarea";
import { useWorkflowConfig } from "@/hooks/useWorkflowConfig";
import { useWorkspaceTreeNodes } from "@/hooks/useWorkspaceTreeNodes";
import { useWorktreeList } from "@/hooks/useWorktreeList";
import { notifyAgentSessionChanged } from "@/lib/agentSessionEvents";
import { trackEvent } from "@/lib/telemetry";
import {
	executeWorkflowAction,
	type WorkflowExecutionAction,
} from "@/lib/workflowExecutionActions";
import type {
	AgentSessionHistoryCandidate,
	AgentSessionHistoryPage,
	AgentSessionItem,
} from "@/types/agent-session";
import type { WorktreeBranch } from "@/types/git";
import type {
	CenterSelection,
	WorkspaceNode,
	WorkspaceTreeItem,
	WorkspaceWorkflow,
	WorkspaceWorkflowHistoryItem,
} from "@/types/workspace-tree";
import { CreateWorktreeModal } from "./CreateWorktreeModal";
import { DeleteWorktreeDialog } from "./DeleteWorktreeDialog";
import { FanoutRowStatusIcon } from "./FanoutRowStatusIcon";
import {
	agentSessionIconPresentation,
	isWorkspaceNodePulseStatus,
	workflowNodeIconClasses,
} from "./WorkflowNodeStatusIcon";
import { WorkflowRowStatusIcon } from "./WorkflowRowStatusIcon";

interface WorkspaceListProps {
	repoPaths: string[];
	selectedRootPath: string | null;
	centerSelection?: CenterSelection | null;
	autoSelectPreferredNode?: boolean;
	onSelectWorktree: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
		centerSelection?: CenterSelection,
	) => void;
	onWorkspaceSelectionInvalidated?: (
		worktreePath: string,
		nodeId: string,
	) => void;
	onAddRepo: () => void;
	onShowSettings: () => void;
}

const WORKTREE_NAME_INDENT_PX = 26;
const TREE_LEVEL_INDENT_PX = 22;

function repoNameFromPath(path: string): string {
	return path.split("/").filter(Boolean).pop() ?? path;
}

function agentSessionLabel(session: AgentSessionItem): string {
	return `${session.provider.charAt(0).toUpperCase()}${session.provider.slice(1)} AgentSession`;
}

function isNodeSelected(
	centerSelection: CenterSelection | null,
	node: WorkspaceNode,
): boolean {
	return centerSelection?.kind === "node" && centerSelection.nodeId === node.id;
}

function WorktreeIndicators({ branch }: { branch: WorktreeBranch }) {
	const hasChanges = branch.dirty_count > 0;
	const hasPr = branch.has_pr === true;
	if (!hasChanges && !hasPr) return null;

	return (
		<div className="relative h-5 w-full text-muted-foreground">
			{hasChanges && (
				<span
					className={`absolute top-0 inline-flex h-5 w-5 items-center justify-center rounded text-[10px] leading-none tabular-nums ${
						hasPr ? "right-6" : "right-0"
					}`}
					title={`${branch.dirty_count} changed files`}
				>
					{branch.dirty_count}
				</span>
			)}
			{hasPr && (
				<span
					className="absolute top-0 right-0 inline-flex h-5 w-5 items-center justify-center"
					role="img"
					title={
						branch.pr_number != null
							? `Pull request #${branch.pr_number}`
							: "Pull request"
					}
					aria-label={
						branch.pr_number != null
							? `Pull request #${branch.pr_number}`
							: "Pull request"
					}
				>
					<GitPullRequest className="size-3 shrink-0" aria-hidden="true" />
				</span>
			)}
		</div>
	);
}

function WorkspaceNodeRow({
	node,
	indentPx,
	selected,
	onSelect,
	onClose,
}: {
	node: WorkspaceNode;
	indentPx: number;
	selected?: boolean;
	onSelect: () => void;
	onClose?: () => void;
}) {
	const ContentIcon = node.contentKind === "session" ? Bot : Terminal;
	const pulseClassName = isWorkspaceNodePulseStatus(node.status)
		? "animate-pulse"
		: "";
	const statusTitle =
		node.status === "error" && node.errorReason
			? node.errorReason
			: `${node.contentKind}, ${node.status}`;
	return (
		<div
			className={`group flex h-8 w-full items-center gap-2 rounded-md pr-2 text-left text-sm transition-colors ${
				selected
					? "bg-foreground/10 text-foreground"
					: "text-foreground/90 hover:bg-foreground/5"
			}`}
			style={{ paddingLeft: indentPx }}
		>
			<button
				type="button"
				className="flex min-w-0 flex-1 items-center gap-2 text-left"
				onClick={onSelect}
				aria-current={selected ? "page" : undefined}
				aria-label={`${node.title}, ${node.status}`}
			>
				<span
					className="flex size-5 shrink-0 items-center justify-center"
					title={statusTitle}
				>
					<ContentIcon
						className={`size-3.5 shrink-0 ${workflowNodeIconClasses[node.status]} ${pulseClassName}`}
						aria-hidden="true"
					/>
				</span>
				<span className="min-w-0 flex-1 truncate">{node.title}</span>
			</button>
			{node.capabilities.canClose && (
				<Button
					size="icon-xs"
					variant="ghost"
					className="hidden size-5 shrink-0 text-muted-foreground group-hover:flex group-focus-within:flex"
					onClick={(event) => {
						event.stopPropagation();
						onClose?.();
					}}
					aria-label={`Close ${node.title}`}
					title="Close"
				>
					<X className="size-3" />
				</Button>
			)}
		</div>
	);
}

function AgentSessionRow({
	session,
	selected,
	onSelect,
	onArchive,
	onDelete,
}: {
	session: AgentSessionItem;
	selected: boolean;
	onSelect: () => void;
	onArchive: () => void;
	onDelete: () => void;
}) {
	const label = agentSessionLabel(session);
	const presentation = agentSessionIconPresentation(session);
	const pulseClassName = presentation.pulse ? "animate-pulse" : "";
	return (
		<div
			className={`group flex h-8 w-full items-center gap-2 rounded-md pr-2 text-left text-sm transition-colors ${
				selected
					? "bg-foreground/10 text-foreground"
					: "text-foreground/90 hover:bg-foreground/5"
			}`}
			style={{ paddingLeft: WORKTREE_NAME_INDENT_PX }}
		>
			<button
				type="button"
				className="flex min-w-0 flex-1 items-center gap-2 text-left"
				onClick={onSelect}
				aria-current={selected ? "page" : undefined}
				aria-label={`${label}, ${presentation.statusLabel}`}
				title={session.id}
			>
				<span
					className="flex size-5 shrink-0 items-center justify-center"
					title={presentation.statusLabel}
				>
					<Bot
						className={`size-3.5 shrink-0 ${presentation.className} ${pulseClassName}`}
						aria-hidden="true"
					/>
				</span>
				<span className="min-w-0 flex-1 truncate">{label}</span>
			</button>
			{session.operations.canArchive && (
				<Button
					size="icon-xs"
					variant="ghost"
					className="hidden size-5 shrink-0 text-muted-foreground group-hover:flex group-focus-within:flex"
					onClick={(event) => {
						event.stopPropagation();
						onArchive();
					}}
					aria-label={`Archive ${label}`}
					title="Archive"
				>
					<X className="size-3" />
				</Button>
			)}
			{session.operations.canDelete && (
				<Button
					size="icon-xs"
					variant="ghost"
					className="hidden size-5 shrink-0 text-destructive group-hover:flex group-focus-within:flex"
					onClick={(event) => {
						event.stopPropagation();
						onDelete();
					}}
					aria-label={`Delete ${label}`}
					title="Delete"
				>
					<Trash2 className="size-3" />
				</Button>
			)}
		</div>
	);
}

function WorkspaceBranchRow({
	item,
	indentPx,
	centerSelection,
	onSelectNode,
	onCloseNode,
	onWorkflowAction,
	onArchiveWorkflow,
}: {
	item: Exclude<WorkspaceTreeItem, WorkspaceNode>;
	indentPx: number;
	centerSelection: CenterSelection | null;
	onSelectNode: (node: WorkspaceNode) => void;
	onCloseNode: (node: WorkspaceNode) => void | Promise<void>;
	onWorkflowAction: (
		action: WorkflowExecutionAction,
		workflow: WorkspaceWorkflow,
	) => void | Promise<void>;
	onArchiveWorkflow: (workflow: WorkspaceWorkflow) => void | Promise<void>;
}) {
	const [expanded, setExpanded] = useState(true);
	const [workflowMenuOpen, setWorkflowMenuOpen] = useState(false);
	const workflow = item.kind === "workflow" ? item : null;
	const actionControlsVisible = workflowMenuOpen;
	return (
		<div>
			<div
				className="group flex h-8 w-full items-center gap-2 rounded-md pr-2 text-sm text-foreground/90 transition-colors hover:bg-foreground/5"
				style={{ paddingLeft: indentPx }}
			>
				<button
					type="button"
					className="flex min-w-0 flex-1 items-center gap-2 text-left"
					onClick={() => setExpanded((prev) => !prev)}
				>
					{item.kind === "fanout" ? (
						<FanoutRowStatusIcon status={item.status} />
					) : (
						<WorkflowRowStatusIcon status={item.status} />
					)}
					<span className="min-w-0 truncate">{item.title}</span>
					{expanded ? (
						<ChevronDown className="size-3.5 shrink-0 text-muted-foreground" />
					) : (
						<ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />
					)}
				</button>
				{workflow && (
					<div
						className={`relative h-5 w-11 shrink-0 transition-opacity ${
							actionControlsVisible
								? "visible opacity-100"
								: "invisible opacity-0 group-hover:visible group-hover:opacity-100 group-focus-within:visible group-focus-within:opacity-100"
						}`}
					>
						<DropdownMenu
							open={workflowMenuOpen}
							onOpenChange={setWorkflowMenuOpen}
						>
							<DropdownMenuTrigger asChild>
								<Button
									size="icon-xs"
									variant="ghost"
									className="absolute top-0 right-6 size-5 shrink-0 text-muted-foreground"
									onClick={(event) => event.stopPropagation()}
									aria-label={`Open menu for ${workflow.title}`}
									title="Menu"
								>
									<MoreHorizontal className="size-3" />
								</Button>
							</DropdownMenuTrigger>
							<DropdownMenuContent align="end">
								<DropdownMenuItem
									disabled={!workflow.capabilities.canStop}
									onSelect={() => {
										if (workflow.capabilities.canStop) {
											onWorkflowAction("stop", workflow);
										}
									}}
								>
									<Square className="size-3.5" />
									Stop
								</DropdownMenuItem>
								<DropdownMenuItem
									disabled={!workflow.capabilities.canResume}
									onSelect={() => {
										if (workflow.capabilities.canResume) {
											onWorkflowAction("resume", workflow);
										}
									}}
								>
									<RotateCcw className="size-3.5" />
									Resume
								</DropdownMenuItem>
								<DropdownMenuItem
									variant="destructive"
									disabled={!workflow.capabilities.canAbort}
									onSelect={() => {
										if (workflow.capabilities.canAbort) {
											onWorkflowAction("abort", workflow);
										}
									}}
								>
									<Ban className="size-3.5" />
									Abort
								</DropdownMenuItem>
							</DropdownMenuContent>
						</DropdownMenu>
						<Button
							size="icon-xs"
							variant="ghost"
							className="absolute top-0 right-0 size-5 shrink-0 text-muted-foreground"
							disabled={!workflow.capabilities.canArchive}
							onClick={(event) => {
								event.stopPropagation();
								if (workflow.capabilities.canArchive) {
									onArchiveWorkflow(workflow);
								}
							}}
							aria-label={`Archive ${workflow.title}`}
							title="Archive"
						>
							<X className="size-3" />
						</Button>
					</div>
				)}
			</div>
			{expanded &&
				item.children.map((child) => (
					<WorkspaceTreeItemRow
						key={child.id}
						item={child}
						indentPx={indentPx + TREE_LEVEL_INDENT_PX}
						centerSelection={centerSelection}
						onSelectNode={onSelectNode}
						onCloseNode={onCloseNode}
						onWorkflowAction={onWorkflowAction}
						onArchiveWorkflow={onArchiveWorkflow}
					/>
				))}
		</div>
	);
}

function WorkspaceTreeItemRow({
	item,
	indentPx,
	centerSelection,
	onSelectNode,
	onCloseNode,
	onWorkflowAction,
	onArchiveWorkflow,
}: {
	item: WorkspaceTreeItem;
	indentPx: number;
	centerSelection: CenterSelection | null;
	onSelectNode: (node: WorkspaceNode) => void;
	onCloseNode: (node: WorkspaceNode) => void | Promise<void>;
	onWorkflowAction: (
		action: WorkflowExecutionAction,
		workflow: WorkspaceWorkflow,
	) => void | Promise<void>;
	onArchiveWorkflow: (workflow: WorkspaceWorkflow) => void | Promise<void>;
}) {
	if (item.kind === "node") {
		return (
			<WorkspaceNodeRow
				node={item}
				indentPx={indentPx}
				selected={isNodeSelected(centerSelection, item)}
				onSelect={() => onSelectNode(item)}
				onClose={() => onCloseNode(item)}
			/>
		);
	}
	return (
		<WorkspaceBranchRow
			item={item}
			indentPx={indentPx}
			centerSelection={centerSelection}
			onSelectNode={onSelectNode}
			onCloseNode={onCloseNode}
			onWorkflowAction={onWorkflowAction}
			onArchiveWorkflow={onArchiveWorkflow}
		/>
	);
}

function WorktreeTreeItem({
	branch,
	repoName,
	selectedRootPath,
	centerSelection,
	autoSelectPreferredNode,
	onSelectWorktree,
	onWorkspaceSelectionInvalidated,
	onDelete,
}: {
	branch: WorktreeBranch;
	repoName: string;
	selectedRootPath: string | null;
	centerSelection: CenterSelection | null;
	autoSelectPreferredNode: boolean;
	onSelectWorktree: WorkspaceListProps["onSelectWorktree"];
	onWorkspaceSelectionInvalidated: WorkspaceListProps["onWorkspaceSelectionInvalidated"];
	onDelete: (branch: WorktreeBranch) => void;
}) {
	const [expanded, setExpanded] = useState(true);
	const [worktreeMenuOpen, setWorktreeMenuOpen] = useState(false);
	const [createMenuOpen, setCreateMenuOpen] = useState(false);
	const [selectedWorkflowName, setSelectedWorkflowName] = useState<
		string | null
	>(null);
	const [workflowRequestInput, setWorkflowRequestInput] = useState("");
	const [workflowStartError, setWorkflowStartError] = useState<string | null>(
		null,
	);
	const [workflowActionError, setWorkflowActionError] = useState<string | null>(
		null,
	);
	const [providerActionError, setProviderActionError] = useState<string | null>(
		null,
	);
	const [availableProviders, setAvailableProviders] = useState<string[]>([]);
	const [providerMenuLoading, setProviderMenuLoading] = useState(false);
	const [providerCreating, setProviderCreating] = useState<string | null>(null);
	const [providerHistory, setProviderHistory] = useState<
		AgentSessionHistoryCandidate[]
	>([]);
	const [providerHistoryNextAfter, setProviderHistoryNextAfter] = useState<
		string | null
	>(null);
	const [providerHistoryLoading, setProviderHistoryLoading] = useState(false);
	const [archiveFallbackDelete, setArchiveFallbackDelete] =
		useState<AgentSessionItem | null>(null);
	const [workflowStarting, setWorkflowStarting] = useState(false);
	const notifiedReconciliationSeqRef = useRef<number | null>(null);
	const preferredSelectionRequestRef = useRef<{
		worktreePath: string | null;
		requested: boolean;
	}>({
		worktreePath: branch.worktree_path,
		requested: false,
	});
	if (
		preferredSelectionRequestRef.current.worktreePath !== branch.worktree_path
	) {
		preferredSelectionRequestRef.current = {
			worktreePath: branch.worktree_path,
			requested: false,
		};
	}
	const scopedCenterSelection =
		centerSelection?.worktreePath === branch.worktree_path
			? centerSelection
			: null;
	const scopedCenterSelectionRef = useRef(scopedCenterSelection);
	scopedCenterSelectionRef.current = scopedCenterSelection;
	const scopedNodeSelection =
		scopedCenterSelection?.kind === "node" ? scopedCenterSelection : null;
	const isSelected =
		branch.worktree_path === selectedRootPath && scopedCenterSelection == null;
	const actionControlsVisible = worktreeMenuOpen || createMenuOpen;
	const hasWorktree = branch.worktree_path != null;
	const canDelete = !branch.is_main_worktree;
	const {
		nodes,
		sessions: workspaceAgentSessions,
		preferredNodeId,
		workflowHistory,
		reconciliationEvent,
		loading: treeLoading,
		error: treeError,
		refresh: refreshTree,
		beginArchiveReconciliation,
		synchronizeSelectedNodeId,
		isReconciliationEventCurrent,
	} = useWorkspaceTreeNodes(branch.worktree_path);
	const agentSessions = useMemo(
		() =>
			workspaceAgentSessions.filter(
				(session) => session.lifecycle !== "archived",
			),
		[workspaceAgentSessions],
	);
	const archivedAgentSessions = useMemo(
		() =>
			workspaceAgentSessions.filter(
				(session) => session.lifecycle === "archived",
			),
		[workspaceAgentSessions],
	);
	const providerSessionsLoading = treeLoading;
	const providerSessionsError = treeError;
	const archivedProviderSessionsLoading = treeLoading && worktreeMenuOpen;
	const archivedProviderSessionsError = treeError;
	const refreshAgentSessions = useCallback(async () => {
		await refreshTree();
	}, [refreshTree]);
	const {
		workflows,
		loading: workflowsLoading,
		error: workflowsError,
	} = useWorkflowConfig(createMenuOpen);
	const selectCenter = useCallback(
		(centerSelection: CenterSelection) => {
			if (!branch.worktree_path) return;
			onSelectWorktree(
				branch.worktree_path,
				branch.name,
				repoName,
				centerSelection,
			);
		},
		[branch.name, branch.worktree_path, onSelectWorktree, repoName],
	);

	useEffect(() => {
		synchronizeSelectedNodeId(scopedNodeSelection?.nodeId ?? null);
	}, [scopedNodeSelection?.nodeId, synchronizeSelectedNodeId]);

	useEffect(() => {
		if (!autoSelectPreferredNode) {
			preferredSelectionRequestRef.current.requested = false;
			return;
		}
		if (preferredSelectionRequestRef.current.requested) return;
		if (!branch.worktree_path) return;
		if (branch.worktree_path !== selectedRootPath) return;
		if (scopedCenterSelection != null || treeLoading || treeError) return;
		if (!preferredNodeId) return;
		preferredSelectionRequestRef.current.requested = true;
		selectCenter({
			kind: "node",
			worktreePath: branch.worktree_path,
			nodeId: preferredNodeId,
		});
	}, [
		autoSelectPreferredNode,
		branch.worktree_path,
		preferredNodeId,
		scopedCenterSelection,
		selectCenter,
		selectedRootPath,
		treeError,
		treeLoading,
	]);

	useEffect(() => {
		if (!reconciliationEvent || reconciliationEvent.selectionInSnapshot) return;
		if (
			notifiedReconciliationSeqRef.current === reconciliationEvent.refreshSeq ||
			!isReconciliationEventCurrent(
				reconciliationEvent,
				scopedNodeSelection?.nodeId ?? null,
			)
		) {
			return;
		}
		notifiedReconciliationSeqRef.current = reconciliationEvent.refreshSeq;
		onWorkspaceSelectionInvalidated?.(
			reconciliationEvent.requestContext.worktreePath,
			reconciliationEvent.requestContext.selectedNodeId,
		);
	}, [
		isReconciliationEventCurrent,
		onWorkspaceSelectionInvalidated,
		reconciliationEvent,
		scopedNodeSelection?.nodeId,
	]);

	const handleSelectNode = useCallback(
		(node: WorkspaceNode) => {
			if (!branch.worktree_path) return;
			selectCenter({
				kind: "node",
				worktreePath: branch.worktree_path,
				nodeId: node.id,
			});
		},
		[branch.worktree_path, selectCenter],
	);
	const handleSelectAgentSession = useCallback(
		(session: AgentSessionItem) => {
			if (!branch.worktree_path) return;
			selectCenter({
				kind: "agent_session",
				worktreePath: branch.worktree_path,
				agentSessionId: session.id,
			});
		},
		[branch.worktree_path, selectCenter],
	);
	const handleArchiveAgentSession = useCallback(
		async (session: AgentSessionItem) => {
			setProviderActionError(null);
			try {
				const outcome = await invoke<
					"archived" | "already_archived" | "delete_confirmation_required"
				>("archive_agent_session", {
					agentSessionId: session.id,
					callerRequestId: `archive.${crypto.randomUUID()}`,
				});
				if (outcome === "delete_confirmation_required") {
					setArchiveFallbackDelete(session);
					return;
				}
				notifyAgentSessionChanged(session.worktreePath);
				await refreshAgentSessions();
				if (
					scopedCenterSelection?.kind === "agent_session" &&
					scopedCenterSelection.agentSessionId === session.id &&
					branch.worktree_path
				) {
					onSelectWorktree(branch.worktree_path, branch.name, repoName);
				}
			} catch (error) {
				setProviderActionError(
					error instanceof Error ? error.message : String(error),
				);
			}
		},
		[
			branch.name,
			branch.worktree_path,
			onSelectWorktree,
			refreshAgentSessions,
			repoName,
			scopedCenterSelection,
		],
	);
	const handleDeleteAgentSession = useCallback(
		async (session: AgentSessionItem, archiveFallback: boolean) => {
			setProviderActionError(null);
			try {
				await invoke(
					archiveFallback
						? "confirm_agent_session_archive_delete"
						: "delete_agent_session",
					{
						agentSessionId: session.id,
						callerRequestId: `delete.${crypto.randomUUID()}`,
					},
				);
				setArchiveFallbackDelete(null);
				notifyAgentSessionChanged(session.worktreePath);
				await refreshAgentSessions();
				if (
					scopedCenterSelection?.kind === "agent_session" &&
					scopedCenterSelection.agentSessionId === session.id &&
					branch.worktree_path
				) {
					onSelectWorktree(branch.worktree_path, branch.name, repoName);
				}
			} catch (error) {
				setProviderActionError(
					error instanceof Error ? error.message : String(error),
				);
			}
		},
		[
			branch.name,
			branch.worktree_path,
			onSelectWorktree,
			refreshAgentSessions,
			repoName,
			scopedCenterSelection,
		],
	);
	const handleRestoreAgentSession = useCallback(
		async (session: AgentSessionItem) => {
			if (!branch.worktree_path) return;
			setProviderActionError(null);
			try {
				await invoke("restore_agent_session", {
					agentSessionId: session.id,
					rows: 24,
					cols: 80,
					callerRequestId: `restore.${crypto.randomUUID()}`,
				});
				notifyAgentSessionChanged(session.worktreePath);
				await refreshAgentSessions();
				selectCenter({
					kind: "agent_session",
					worktreePath: branch.worktree_path,
					agentSessionId: session.id,
				});
			} catch (error) {
				setProviderActionError(
					error instanceof Error ? error.message : String(error),
				);
			}
		},
		[branch.worktree_path, refreshAgentSessions, selectCenter],
	);

	const refreshProviderHistory = useCallback(async () => {
		if (!branch.worktree_path) return;
		setProviderHistoryLoading(true);
		try {
			const page = await invoke<AgentSessionHistoryPage>(
				"list_agent_session_history",
				{
					worktreePath: branch.worktree_path,
					limit: 100,
				},
			);
			setProviderHistory(page?.items ?? []);
			setProviderHistoryNextAfter(page?.nextAfter ?? null);
		} catch (error) {
			setProviderHistory([]);
			setProviderHistoryNextAfter(null);
			setProviderActionError(
				error instanceof Error ? error.message : String(error),
			);
		} finally {
			setProviderHistoryLoading(false);
		}
	}, [branch.worktree_path]);

	const loadMoreProviderHistory = useCallback(async () => {
		if (!branch.worktree_path || !providerHistoryNextAfter) return;
		setProviderHistoryLoading(true);
		try {
			const page = await invoke<AgentSessionHistoryPage>(
				"list_agent_session_history",
				{
					worktreePath: branch.worktree_path,
					limit: 100,
					after: providerHistoryNextAfter,
				},
			);
			setProviderHistory((current) => [...current, ...(page?.items ?? [])]);
			setProviderHistoryNextAfter(page?.nextAfter ?? null);
			setProviderActionError(null);
		} catch (error) {
			setProviderActionError(
				error instanceof Error ? error.message : String(error),
			);
		} finally {
			setProviderHistoryLoading(false);
		}
	}, [branch.worktree_path, providerHistoryNextAfter]);

	const handleResumeProviderHistory = useCallback(
		async (candidate: AgentSessionHistoryCandidate) => {
			if (!branch.worktree_path) return;
			setProviderActionError(null);
			try {
				const agentSessionId = await invoke<string>(
					"resume_agent_session_history_candidate",
					{
						workspaceIdentity: branch.worktree_path,
						worktreePath: branch.worktree_path,
						provider: candidate.provider,
						providerSessionId: candidate.providerSessionId,
						rows: 24,
						cols: 80,
						callerRequestId: `history-resume.${crypto.randomUUID()}`,
					},
				);
				await refreshAgentSessions();
				selectCenter({
					kind: "agent_session",
					worktreePath: branch.worktree_path,
					agentSessionId,
				});
			} catch (error) {
				setProviderActionError(
					error instanceof Error ? error.message : String(error),
				);
			}
		},
		[branch.worktree_path, refreshAgentSessions, selectCenter],
	);

	const handleRestoreWorkflow = useCallback(
		async (workflow: WorkspaceWorkflowHistoryItem) => {
			if (!branch.worktree_path) return;
			await invoke("restore_workspace_workflow_execution", {
				worktreePath: branch.worktree_path,
				executionId: workflow.executionId,
			});
			await refreshTree();
		},
		[branch.worktree_path, refreshTree],
	);

	const handleArchiveWorkflow = useCallback(
		async (workflow: WorkspaceWorkflow) => {
			if (!branch.worktree_path) return;
			setWorkflowActionError(null);
			try {
				await invoke("archive_workspace_workflow_execution", {
					worktreePath: branch.worktree_path,
					executionId: workflow.id,
				});
				if (scopedNodeSelection?.nodeId) {
					await beginArchiveReconciliation(scopedNodeSelection.nodeId);
				} else {
					await refreshTree();
				}
			} catch (e) {
				setWorkflowActionError(`Archive workflow failed: ${String(e)}`);
			}
		},
		[
			beginArchiveReconciliation,
			branch.worktree_path,
			refreshTree,
			scopedNodeSelection?.nodeId,
		],
	);

	const handleWorkflowExecutionAction = useCallback(
		async (action: WorkflowExecutionAction, workflow: WorkspaceWorkflow) => {
			setWorkflowActionError(null);
			try {
				await executeWorkflowAction(action, workflow.id);
				await refreshTree();
			} catch (error) {
				setWorkflowActionError(
					error instanceof Error ? error.message : String(error),
				);
			}
		},
		[refreshTree],
	);

	const refreshAvailableProviders = useCallback(async () => {
		if (!branch.worktree_path) return;
		setProviderMenuLoading(true);
		setProviderActionError(null);
		try {
			const providers = await invoke<string[]>(
				"list_available_agent_session_providers",
			);
			setAvailableProviders(providers ?? []);
		} catch (error) {
			setAvailableProviders([]);
			setProviderActionError(
				error instanceof Error ? error.message : String(error),
			);
		} finally {
			setProviderMenuLoading(false);
		}
	}, [branch.worktree_path]);

	const handleCreateAgentSession = useCallback(
		async (provider: string) => {
			if (!branch.worktree_path || providerCreating) return;
			setCreateMenuOpen(false);
			setProviderCreating(provider);
			setProviderActionError(null);
			// 作成完了を待たずに中央paneを起動中表示へ切り替え、
			// クリック直後の視覚フィードバックを保証する
			const launchToken = crypto.randomUUID();
			selectCenter({
				kind: "agent_session_launching",
				worktreePath: branch.worktree_path,
				provider,
				launchToken,
			});
			// 解決時に選択が同一launchTokenの起動中表示のままである場合のみ遷移し、
			// pending中に移動したユーザーの選択を奪わない
			const isLaunchSelectionCurrent = () => {
				const current = scopedCenterSelectionRef.current;
				return (
					current?.kind === "agent_session_launching" &&
					current.launchToken === launchToken
				);
			};
			try {
				const agentSessionId = await invoke<string>("create_agent_session", {
					workspaceIdentity: branch.worktree_path,
					worktreePath: branch.worktree_path,
					provider,
					rows: 24,
					cols: 80,
					callerRequestId: `create.${launchToken}`,
				});
				if (isLaunchSelectionCurrent()) {
					selectCenter({
						kind: "agent_session",
						worktreePath: branch.worktree_path,
						agentSessionId,
						initialAttachment: {
							agentSessionId,
							workspaceIdentity: branch.worktree_path,
							worktreePath: branch.worktree_path,
							provider: provider as AgentSessionItem["provider"],
						},
					});
				}
				void refreshAgentSessions();
			} catch (error) {
				const message = error instanceof Error ? error.message : String(error);
				setProviderActionError(message);
				if (isLaunchSelectionCurrent()) {
					selectCenter({
						kind: "agent_session_launching",
						worktreePath: branch.worktree_path,
						provider,
						launchToken,
						error: message,
					});
				}
			} finally {
				setProviderCreating(null);
			}
		},
		[
			branch.worktree_path,
			providerCreating,
			refreshAgentSessions,
			selectCenter,
		],
	);

	const handleSelectWorkflowForStart = useCallback((workflowName: string) => {
		setCreateMenuOpen(false);
		setSelectedWorkflowName(workflowName);
		setWorkflowRequestInput("");
		setWorkflowStartError(null);
	}, []);

	const handleStartWorkflow = useCallback(async () => {
		if (!branch.worktree_path || !selectedWorkflowName || workflowStarting) {
			return;
		}
		setWorkflowStarting(true);
		setWorkflowStartError(null);
		try {
			await invoke<string>("start_workflow", {
				workflowName: selectedWorkflowName,
				worktreePath: branch.worktree_path,
				request: workflowRequestInput.trim(),
			});
			setSelectedWorkflowName(null);
			setWorkflowRequestInput("");
			await refreshTree();
		} catch (e) {
			setWorkflowStartError(String(e));
		} finally {
			setWorkflowStarting(false);
		}
	}, [
		branch.worktree_path,
		refreshTree,
		selectedWorkflowName,
		workflowStarting,
		workflowRequestInput,
	]);

	const handleWorkflowDialogOpenChange = useCallback((open: boolean) => {
		if (open) return;
		setSelectedWorkflowName(null);
		setWorkflowRequestInput("");
		setWorkflowStartError(null);
	}, []);

	const handleCloseNode = useCallback(
		async (node: WorkspaceNode) => {
			if (!branch.worktree_path || !node.capabilities.canClose) return;
			setWorkflowActionError(null);
			try {
				await invoke("close_workspace_node", {
					worktreePath: branch.worktree_path,
					nodeId: node.id,
				});
				window.dispatchEvent(
					new CustomEvent("workspace-tree-refresh", {
						detail: { worktreePath: branch.worktree_path },
					}),
				);
				await refreshTree();
			} catch (error) {
				setWorkflowActionError(
					error instanceof Error ? error.message : String(error),
				);
			}
		},
		[branch.worktree_path, refreshTree],
	);

	return (
		<div>
			<div
				className={`group flex h-8 w-full items-center gap-1 rounded-md px-2 text-sm transition-colors ${
					isSelected
						? "bg-foreground/10 text-foreground"
						: hasWorktree
							? "text-foreground hover:bg-foreground/5"
							: "text-muted-foreground hover:bg-foreground/5"
				}`}
			>
				<button
					type="button"
					data-testid={`worktree-item-${branch.name}`}
					aria-expanded={expanded}
					className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
					onClick={() => setExpanded((prev) => !prev)}
				>
					{branch.is_main_worktree ? (
						<Home
							className="size-3 shrink-0 text-muted-foreground"
							aria-label="Main repository"
						/>
					) : (
						<GitBranch
							className="size-3 shrink-0 text-muted-foreground"
							aria-label="Worktree"
						/>
					)}
					<span className="min-w-0 truncate">{branch.name}</span>
					{expanded ? (
						<ChevronDown className="hidden size-3.5 shrink-0 text-muted-foreground group-hover:block" />
					) : (
						<ChevronRight className="hidden size-3.5 shrink-0 text-muted-foreground group-hover:block" />
					)}
				</button>
				<div className="relative h-5 w-11 shrink-0">
					<div
						className={`absolute inset-0 items-center justify-end ${
							actionControlsVisible
								? "hidden"
								: "flex group-hover:hidden group-focus-within:hidden"
						}`}
					>
						<WorktreeIndicators branch={branch} />
					</div>
					<div
						className={`absolute inset-0 transition-opacity ${
							actionControlsVisible
								? "visible opacity-100"
								: "invisible opacity-0 group-hover:visible group-hover:opacity-100 group-focus-within:visible group-focus-within:opacity-100"
						}`}
					>
						<DropdownMenu
							open={worktreeMenuOpen}
							onOpenChange={(open) => {
								setWorktreeMenuOpen(open);
								if (open) {
									void refreshTree();
									void refreshProviderHistory();
								}
							}}
						>
							<DropdownMenuTrigger asChild>
								<Button
									size="icon-xs"
									variant="ghost"
									className="absolute top-0 right-6 size-5 shrink-0 text-muted-foreground"
									onClick={(event) => event.stopPropagation()}
									aria-label={`Open menu for ${branch.name}`}
									title="Menu"
								>
									<MoreHorizontal className="size-3" />
								</Button>
							</DropdownMenuTrigger>
							<DropdownMenuContent align="end">
								<DropdownMenuSub>
									<DropdownMenuSubTrigger>
										<MessageSquare className="size-3.5" />
										SessionHistory
									</DropdownMenuSubTrigger>
									<DropdownMenuSubContent>
										{(providerHistoryLoading ||
											archivedProviderSessionsLoading) && (
											<DropdownMenuItem disabled>
												<Loader2 className="size-3.5 animate-spin" />
												Loading Session history
											</DropdownMenuItem>
										)}
										{archivedAgentSessions.length === 0 &&
										providerHistory.length === 0 &&
										!providerHistoryLoading &&
										!archivedProviderSessionsLoading ? (
											<DropdownMenuItem disabled>
												No session history
											</DropdownMenuItem>
										) : null}
										{archivedAgentSessions.map((session) => (
											<DropdownMenuItem
												key={session.id}
												onSelect={() => void handleRestoreAgentSession(session)}
											>
												<span className="max-w-52 truncate">
													{agentSessionLabel(session)}
												</span>
												<Button
													size="icon-xs"
													variant="ghost"
													className="ml-2 size-5 text-destructive"
													onClick={(event) => {
														event.preventDefault();
														event.stopPropagation();
														void handleDeleteAgentSession(session, false);
													}}
													aria-label={`Delete ${agentSessionLabel(session)}`}
													title="Delete"
												>
													<Trash2 className="size-3" />
												</Button>
											</DropdownMenuItem>
										))}
										{archivedProviderSessionsError && (
											<DropdownMenuItem disabled>
												<span className="max-w-52 truncate text-destructive">
													{archivedProviderSessionsError}
												</span>
											</DropdownMenuItem>
										)}
										{providerHistory.map((candidate) => (
											<DropdownMenuItem
												key={`${candidate.provider}:${candidate.providerSessionId}`}
												onSelect={() =>
													void handleResumeProviderHistory(candidate)
												}
											>
												<span className="max-w-52 truncate">
													{candidate.provider} {candidate.providerSessionId}
												</span>
											</DropdownMenuItem>
										))}
										{providerHistory.length > 0 && (
											<DropdownMenuItem
												disabled={
													providerHistoryLoading || !providerHistoryNextAfter
												}
												onSelect={(event) => {
													event.preventDefault();
													void loadMoreProviderHistory();
												}}
											>
												{providerHistoryNextAfter
													? "Load more Provider history"
													: "All Provider history loaded"}
											</DropdownMenuItem>
										)}
									</DropdownMenuSubContent>
								</DropdownMenuSub>
								<DropdownMenuSub>
									<DropdownMenuSubTrigger>
										<Workflow className="size-3.5" />
										WorkflowHistory
									</DropdownMenuSubTrigger>
									<DropdownMenuSubContent>
										{workflowHistory.length === 0 ? (
											<DropdownMenuItem disabled>No workflows</DropdownMenuItem>
										) : (
											workflowHistory.map((node) => (
												<DropdownMenuItem
													key={node.executionId}
													onSelect={() => void handleRestoreWorkflow(node)}
												>
													<span className="max-w-52 truncate">
														{node.title}
													</span>
												</DropdownMenuItem>
											))
										)}
									</DropdownMenuSubContent>
								</DropdownMenuSub>
								<DropdownMenuItem
									disabled={!branch.pr_url}
									onSelect={() => {
										if (branch.pr_url) {
											openUrl(branch.pr_url);
										}
									}}
								>
									<ExternalLink className="size-3.5" />
									PR Link
								</DropdownMenuItem>
								<DropdownMenuItem
									variant="destructive"
									disabled={!canDelete}
									onSelect={() => {
										if (canDelete) {
											onDelete(branch);
										}
									}}
								>
									<Trash2 className="size-3.5" />
									Delete
								</DropdownMenuItem>
							</DropdownMenuContent>
						</DropdownMenu>
						<DropdownMenu
							open={createMenuOpen}
							onOpenChange={(open) => {
								setCreateMenuOpen(open);
								if (open) {
									void refreshTree();
									void refreshAvailableProviders();
								}
							}}
						>
							<DropdownMenuTrigger asChild>
								<Button
									size="icon-xs"
									variant="ghost"
									className="absolute top-0 right-0 size-5 shrink-0 text-muted-foreground"
									disabled={!hasWorktree}
									onClick={(event) => event.stopPropagation()}
									aria-label={`Create in ${branch.name}`}
									title="Create"
								>
									<Plus className="size-3" />
								</Button>
							</DropdownMenuTrigger>
							<DropdownMenuContent align="end" className="w-56">
								<DropdownMenuSub>
									<DropdownMenuSubTrigger>
										<Bot className="size-3.5" />
										NewSession
									</DropdownMenuSubTrigger>
									<DropdownMenuSubContent className="w-56">
										{providerMenuLoading ? (
											<DropdownMenuItem disabled>
												<Loader2 className="size-3.5 animate-spin" />
												Loading Providers
											</DropdownMenuItem>
										) : availableProviders.length === 0 ? (
											<DropdownMenuItem disabled>
												No available Providers
											</DropdownMenuItem>
										) : (
											availableProviders.map((provider) => (
												<DropdownMenuItem
													key={provider}
													disabled={providerCreating != null}
													onSelect={() =>
														void handleCreateAgentSession(provider)
													}
												>
													{providerCreating === provider && (
														<Loader2 className="size-3.5 animate-spin" />
													)}
													{provider}
												</DropdownMenuItem>
											))
										)}
									</DropdownMenuSubContent>
								</DropdownMenuSub>
								<DropdownMenuSeparator />
								<DropdownMenuSub>
									<DropdownMenuSubTrigger>
										<Workflow className="size-3.5" />
										NewWorkflow
									</DropdownMenuSubTrigger>
									<DropdownMenuSubContent className="w-64">
										{workflowsLoading ? (
											<DropdownMenuItem disabled>
												<Loader2 className="size-3.5 animate-spin" />
												Loading workflows
											</DropdownMenuItem>
										) : workflowsError ? (
											<DropdownMenuItem disabled>
												<span className="truncate text-destructive">
													{workflowsError}
												</span>
											</DropdownMenuItem>
										) : workflows.length === 0 ? (
											<DropdownMenuItem disabled>
												No workflows configured
											</DropdownMenuItem>
										) : (
											workflows.map((workflow) => (
												<DropdownMenuItem
													key={workflow.name}
													onSelect={() =>
														handleSelectWorkflowForStart(workflow.name)
													}
												>
													<div className="min-w-0">
														<div className="truncate">{workflow.name}</div>
														{workflow.description && (
															<div className="truncate text-xs text-muted-foreground">
																{workflow.description}
															</div>
														)}
													</div>
												</DropdownMenuItem>
											))
										)}
									</DropdownMenuSubContent>
								</DropdownMenuSub>
							</DropdownMenuContent>
						</DropdownMenu>
					</div>
				</div>
			</div>
			{expanded && hasWorktree && (
				<div className="mt-0.5">
					{agentSessions.map((session) => (
						<AgentSessionRow
							key={session.id}
							session={session}
							selected={
								scopedCenterSelection?.kind === "agent_session" &&
								scopedCenterSelection.agentSessionId === session.id
							}
							onSelect={() => handleSelectAgentSession(session)}
							onArchive={() => void handleArchiveAgentSession(session)}
							onDelete={() => void handleDeleteAgentSession(session, false)}
						/>
					))}
					{treeLoading ? (
						<div
							className="flex h-8 items-center text-muted-foreground"
							style={{ paddingLeft: WORKTREE_NAME_INDENT_PX }}
						>
							<Loader2 className="size-3.5 animate-spin" />
						</div>
					) : treeError && nodes.length === 0 ? (
						<div
							className="truncate py-1 text-xs text-destructive"
							style={{ paddingLeft: WORKTREE_NAME_INDENT_PX }}
						>
							{treeError}
						</div>
					) : nodes.length === 0 && agentSessions.length === 0 ? (
						<div
							className="truncate py-1 text-xs text-muted-foreground"
							style={{ paddingLeft: WORKTREE_NAME_INDENT_PX }}
						>
							No sessions or workflows
						</div>
					) : (
						nodes.map((node) => (
							<WorkspaceTreeItemRow
								key={node.id}
								item={node}
								indentPx={WORKTREE_NAME_INDENT_PX}
								centerSelection={scopedCenterSelection}
								onSelectNode={handleSelectNode}
								onCloseNode={handleCloseNode}
								onWorkflowAction={handleWorkflowExecutionAction}
								onArchiveWorkflow={handleArchiveWorkflow}
							/>
						))
					)}
					{providerSessionsLoading && agentSessions.length === 0 && (
						<div
							className="flex h-8 items-center text-muted-foreground"
							style={{ paddingLeft: WORKTREE_NAME_INDENT_PX }}
						>
							<Loader2 className="size-3.5 animate-spin" />
						</div>
					)}
					{providerSessionsError && (
						<div
							className="truncate py-1 text-xs text-destructive"
							style={{ paddingLeft: WORKTREE_NAME_INDENT_PX }}
						>
							{providerSessionsError}
						</div>
					)}
					{providerActionError && (
						<div
							role="alert"
							className="mt-1 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
							style={{ marginLeft: WORKTREE_NAME_INDENT_PX }}
						>
							{providerActionError}
						</div>
					)}
					{workflowActionError && (
						<div
							role="alert"
							className="mt-1 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
							style={{ marginLeft: WORKTREE_NAME_INDENT_PX }}
						>
							{workflowActionError}
						</div>
					)}
				</div>
			)}
			<Dialog
				open={archiveFallbackDelete != null}
				onOpenChange={(open) => {
					if (!open) setArchiveFallbackDelete(null);
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Delete AgentSession?</DialogTitle>
						<DialogDescription>
							This AgentSession has no Provider session ID and cannot be
							archived. Deleting removes Releash-owned state. Provider history
							may still offer a recovery candidate.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button
							type="button"
							variant="outline"
							onClick={() => setArchiveFallbackDelete(null)}
						>
							Cancel
						</Button>
						<Button
							type="button"
							variant="destructive"
							onClick={() => {
								if (archiveFallbackDelete) {
									void handleDeleteAgentSession(archiveFallbackDelete, true);
								}
							}}
						>
							Delete
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
			<Dialog
				open={selectedWorkflowName != null}
				onOpenChange={handleWorkflowDialogOpenChange}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>NewWorkflow</DialogTitle>
						<DialogDescription>{selectedWorkflowName}</DialogDescription>
					</DialogHeader>
					<form
						className="space-y-4"
						onSubmit={(event) => {
							event.preventDefault();
							void handleStartWorkflow();
						}}
					>
						<Textarea
							value={workflowRequestInput}
							onChange={(event) => setWorkflowRequestInput(event.target.value)}
							placeholder="Request (optional)"
							aria-label="Workflow request"
							disabled={workflowStarting}
						/>
						{workflowStartError && (
							<div
								role="alert"
								className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
							>
								{workflowStartError}
							</div>
						)}
						<DialogFooter>
							<Button
								type="button"
								variant="outline"
								onClick={() => handleWorkflowDialogOpenChange(false)}
								disabled={workflowStarting}
							>
								Cancel
							</Button>
							<Button type="submit" disabled={workflowStarting}>
								{workflowStarting ? "Starting..." : "Start"}
							</Button>
						</DialogFooter>
					</form>
				</DialogContent>
			</Dialog>
		</div>
	);
}

function RepoTreeSectionView({
	repoPath,
	branches,
	loading,
	refresh,
	selectedRootPath,
	centerSelection,
	autoSelectPreferredNode,
	onSelectWorktree,
	onWorkspaceSelectionInvalidated,
}: {
	repoPath: string;
	branches: WorktreeBranch[];
	loading: boolean;
	refresh: (options?: { silent?: boolean }) => Promise<void>;
	selectedRootPath: string | null;
	centerSelection: CenterSelection | null;
	autoSelectPreferredNode: boolean;
	onSelectWorktree: WorkspaceListProps["onSelectWorktree"];
	onWorkspaceSelectionInvalidated: WorkspaceListProps["onWorkspaceSelectionInvalidated"];
}) {
	const [collapsed, setCollapsed] = useState(false);
	const [refreshing, setRefreshing] = useState(false);
	const [deletingBranch, setDeletingBranch] = useState<WorktreeBranch | null>(
		null,
	);
	const repoName = useMemo(() => repoNameFromPath(repoPath), [repoPath]);

	const handleRefresh = useCallback(async () => {
		setRefreshing(true);
		try {
			await refresh();
		} finally {
			setRefreshing(false);
		}
	}, [refresh]);

	const handleDeleteConfirm = useCallback(
		async (branch: WorktreeBranch, force: boolean) => {
			try {
				if (branch.worktree_path) {
					await invoke("remove_worktree", {
						repoPath,
						worktreePath: branch.worktree_path,
						force,
					});
					trackEvent("worktree_removed");
				} else if (branch.is_merged) {
					await invoke("delete_branch", {
						repoPath,
						branchName: branch.name,
						force,
					});
				}
				await refresh();
			} finally {
				setDeletingBranch(null);
			}
		},
		[repoPath, refresh],
	);

	return (
		<div className="space-y-0.5">
			<div className="flex h-7 items-center gap-1 px-2 text-xs font-medium text-muted-foreground">
				<button
					type="button"
					className="flex min-w-0 flex-1 items-center gap-1.5 rounded text-left transition-colors hover:text-foreground"
					onClick={() => setCollapsed((prev) => !prev)}
				>
					<span className="min-w-0 truncate">{repoName}</span>
					{collapsed ? (
						<ChevronRight className="size-3.5 shrink-0" />
					) : (
						<ChevronDown className="size-3.5 shrink-0" />
					)}
					<span className="ml-auto shrink-0 text-[11px]">
						{branches.length}
					</span>
				</button>
				<Button
					size="icon-xs"
					variant="ghost"
					className="size-5"
					onClick={handleRefresh}
					disabled={refreshing}
					aria-label={`Refresh ${repoName}`}
					title={`Refresh ${repoName}`}
				>
					<RefreshCw className={`size-3 ${refreshing ? "animate-spin" : ""}`} />
				</Button>
			</div>
			{!collapsed && (
				<div className="space-y-1">
					{loading ? (
						<div className="flex items-center justify-center py-4">
							<Loader2 className="size-4 animate-spin text-muted-foreground" />
						</div>
					) : (
						branches.map((branch) => (
							<WorktreeTreeItem
								key={branch.name}
								branch={branch}
								repoName={repoName}
								selectedRootPath={selectedRootPath}
								centerSelection={centerSelection}
								autoSelectPreferredNode={autoSelectPreferredNode}
								onSelectWorktree={onSelectWorktree}
								onWorkspaceSelectionInvalidated={
									onWorkspaceSelectionInvalidated
								}
								onDelete={setDeletingBranch}
							/>
						))
					)}
				</div>
			)}
			<DeleteWorktreeDialog
				open={!!deletingBranch}
				branch={deletingBranch}
				onConfirm={handleDeleteConfirm}
				onCancel={() => setDeletingBranch(null)}
			/>
		</div>
	);
}

function RepoTreeSection({
	repoPath,
	selectedRootPath,
	centerSelection,
	autoSelectPreferredNode,
	onSelectWorktree,
	onWorkspaceSelectionInvalidated,
}: {
	repoPath: string;
	selectedRootPath: string | null;
	centerSelection: CenterSelection | null;
	autoSelectPreferredNode: boolean;
	onSelectWorktree: WorkspaceListProps["onSelectWorktree"];
	onWorkspaceSelectionInvalidated: WorkspaceListProps["onWorkspaceSelectionInvalidated"];
}) {
	const { branches, loading, refresh } = useWorktreeList(repoPath);
	return (
		<RepoTreeSectionView
			repoPath={repoPath}
			branches={branches}
			loading={loading}
			refresh={refresh}
			selectedRootPath={selectedRootPath}
			centerSelection={centerSelection}
			autoSelectPreferredNode={autoSelectPreferredNode}
			onSelectWorktree={onSelectWorktree}
			onWorkspaceSelectionInvalidated={onWorkspaceSelectionInvalidated}
		/>
	);
}

export function WorkspaceList({
	repoPaths,
	selectedRootPath,
	centerSelection,
	autoSelectPreferredNode = false,
	onSelectWorktree,
	onWorkspaceSelectionInvalidated,
	onAddRepo,
	onShowSettings,
}: WorkspaceListProps) {
	const [showCreate, setShowCreate] = useState(false);

	return (
		<div className="flex h-full flex-col">
			<div className="flex h-9 shrink-0 items-center justify-between px-2">
				<span className="text-xs font-semibold tracking-wide text-muted-foreground">
					Workspaces
				</span>
				<div className="flex items-center gap-0.5">
					<Button
						size="icon-xs"
						variant="ghost"
						className="size-5"
						onClick={() => setShowCreate(true)}
						title="Add worktree"
						aria-label="Add Worktree"
					>
						<Plus className="size-3" />
					</Button>
				</div>
			</div>

			<div className="flex-1 space-y-2 overflow-y-auto px-2 py-1">
				{repoPaths.map((repoPath) => (
					<RepoTreeSection
						key={repoPath}
						repoPath={repoPath}
						selectedRootPath={selectedRootPath}
						centerSelection={centerSelection ?? null}
						autoSelectPreferredNode={autoSelectPreferredNode}
						onSelectWorktree={onSelectWorktree}
						onWorkspaceSelectionInvalidated={onWorkspaceSelectionInvalidated}
					/>
				))}
				{repoPaths.length === 0 && (
					<div className="px-2 py-8 text-center text-xs text-muted-foreground">
						No Repository
					</div>
				)}
			</div>

			<div className="flex h-9 shrink-0 items-center justify-between border-t border-border px-2">
				<Button
					size="sm"
					variant="ghost"
					className="h-7 px-2 text-xs"
					onClick={onAddRepo}
				>
					<Plus className="mr-1 size-3.5" />
					Add Repository
				</Button>
				<Button
					size="icon"
					variant="ghost"
					className="size-7"
					onClick={onShowSettings}
					title="Settings"
				>
					<Settings className="size-3.5" />
				</Button>
			</div>

			{showCreate && repoPaths.length > 0 && (
				<CreateWorktreeModal
					open={showCreate}
					repoPaths={repoPaths}
					onCreated={(rootPath, branchName, repoName) => {
						setShowCreate(false);
						emit("branch-list-sync");
						onSelectWorktree(rootPath, branchName, repoName);
					}}
					onClose={() => setShowCreate(false)}
				/>
			)}
		</div>
	);
}
