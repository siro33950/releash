import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
	Archive,
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
import { archiveSession, restoreSession } from "@/hooks/useSessionStore";
import { useWorkflowConfig } from "@/hooks/useWorkflowConfig";
import { useWorkspaceTreeNodes } from "@/hooks/useWorkspaceTreeNodes";
import { useWorktreeList } from "@/hooks/useWorktreeList";
import { trackEvent } from "@/lib/telemetry";
import {
	executeWorkflowAction,
	type WorkflowExecutionAction,
} from "@/lib/workflowExecutionActions";
import type { WorktreeBranch } from "@/types/git";
import type {
	CenterSelection,
	NewSessionCreationStatus,
	WorkspaceNode,
	WorkspaceSessionHistoryItem,
	WorkspaceTreeItem,
	WorkspaceWorkflow,
	WorkspaceWorkflowHistoryItem,
} from "@/types/workspace-tree";
import { CreateWorktreeModal } from "./CreateWorktreeModal";
import { DeleteWorktreeDialog } from "./DeleteWorktreeDialog";
import { FanoutRowStatusIcon } from "./FanoutRowStatusIcon";
import { workflowNodeIconClasses } from "./WorkflowNodeStatusIcon";
import { WorkflowRowStatusIcon } from "./WorkflowRowStatusIcon";

interface WorkspaceListProps {
	repoPaths: string[];
	selectedRootPath: string | null;
	centerSelection?: CenterSelection | null;
	autoSelectPreferredNode?: boolean;
	newSessionCreationStatusByWorktree?: Record<string, NewSessionCreationStatus>;
	onSelectWorktree: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
		centerSelection?: CenterSelection,
	) => void;
	onCreateSession: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
	) => void;
	onAddRepo: () => void;
	onShowSettings: () => void;
}

const WORKTREE_NAME_INDENT_PX = 26;
const TREE_LEVEL_INDENT_PX = 22;
const DEFAULT_SESSION_TITLE = "NewSession";

function repoNameFromPath(path: string): string {
	return path.split("/").filter(Boolean).pop() ?? path;
}

function sessionLabel(session: WorkspaceSessionHistoryItem): string {
	return session.firstMessage.trim() || DEFAULT_SESSION_TITLE;
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
	const pulseClassName =
		node.status === "running" || node.status === "waiting"
			? "animate-pulse"
			: "";
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
					title={`${node.contentKind}, ${node.status}`}
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
	newSessionCreationStatus,
	onSelectWorktree,
	onCreateSession,
	onDelete,
}: {
	branch: WorktreeBranch;
	repoName: string;
	selectedRootPath: string | null;
	centerSelection: CenterSelection | null;
	autoSelectPreferredNode: boolean;
	newSessionCreationStatus?: NewSessionCreationStatus;
	onSelectWorktree: WorkspaceListProps["onSelectWorktree"];
	onCreateSession: WorkspaceListProps["onCreateSession"];
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
	const [workflowStarting, setWorkflowStarting] = useState(false);
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
	const isSelected =
		branch.worktree_path === selectedRootPath && scopedCenterSelection == null;
	const actionControlsVisible = worktreeMenuOpen || createMenuOpen;
	const hasWorktree = branch.worktree_path != null;
	const canDelete = !branch.is_main_worktree;
	const {
		nodes,
		preferredNodeId,
		closedSessions,
		workflowHistory,
		loading: treeLoading,
		error: treeError,
		refresh: refreshTree,
	} = useWorkspaceTreeNodes(branch.worktree_path);
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
		if (!autoSelectPreferredNode) return;
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
				await refreshTree();
			} catch (e) {
				setWorkflowActionError(`Archive workflow failed: ${String(e)}`);
			}
		},
		[branch.worktree_path, refreshTree],
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

	const handleNewSession = useCallback(() => {
		if (!branch.worktree_path) return;
		onCreateSession(branch.worktree_path, branch.name, repoName);
	}, [branch.name, branch.worktree_path, onCreateSession, repoName]);

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
				permissionMode: "ask",
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

	const handleRestoreSession = useCallback(
		async (session: WorkspaceSessionHistoryItem) => {
			if (!branch.worktree_path) return;
			await restoreSession(session.id);
			await refreshTree();
			const nodeId = await invoke<string | null>(
				"get_workspace_session_node_id",
				{
					worktreePath: branch.worktree_path,
					sessionId: session.id,
				},
			);
			if (!nodeId) return;
			selectCenter({
				kind: "node",
				worktreePath: branch.worktree_path,
				nodeId,
			});
		},
		[branch.worktree_path, refreshTree, selectCenter],
	);

	const handleArchiveSession = useCallback(
		async (sessionId: string) => {
			await archiveSession(sessionId);
			await refreshTree();
		},
		[refreshTree],
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
								if (open) void refreshTree();
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
										{closedSessions.length === 0 ? (
											<DropdownMenuItem disabled>
												No closed sessions
											</DropdownMenuItem>
										) : (
											closedSessions.map((session) => (
												<DropdownMenuItem
													key={session.id}
													onSelect={() => handleRestoreSession(session)}
												>
													<span className="max-w-52 truncate">
														{sessionLabel(session)}
													</span>
													<Button
														size="icon-xs"
														variant="ghost"
														className="ml-2 size-5"
														onClick={(event) => {
															event.preventDefault();
															event.stopPropagation();
															void handleArchiveSession(session.id);
														}}
														aria-label={`Archive ${sessionLabel(session)}`}
														title="Archive"
													>
														<Archive className="size-3" />
													</Button>
												</DropdownMenuItem>
											))
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
								if (open) void refreshTree();
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
								<DropdownMenuItem
									disabled={newSessionCreationStatus?.pending === true}
									onSelect={handleNewSession}
								>
									{newSessionCreationStatus?.pending ? (
										<Loader2 className="size-3.5 animate-spin" />
									) : (
										<MessageSquare className="size-3.5" />
									)}
									{newSessionCreationStatus?.pending
										? "Creating Session..."
										: "NewSession"}
								</DropdownMenuItem>
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
					) : nodes.length === 0 ? (
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
					{workflowActionError && (
						<div
							role="alert"
							className="mt-1 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
							style={{ marginLeft: WORKTREE_NAME_INDENT_PX }}
						>
							{workflowActionError}
						</div>
					)}
					{newSessionCreationStatus?.error && (
						<div
							role="alert"
							className="mt-1 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
							style={{ marginLeft: WORKTREE_NAME_INDENT_PX }}
						>
							{newSessionCreationStatus.error}
						</div>
					)}
				</div>
			)}
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
	newSessionCreationStatusByWorktree,
	onSelectWorktree,
	onCreateSession,
}: {
	repoPath: string;
	branches: WorktreeBranch[];
	loading: boolean;
	refresh: (options?: { silent?: boolean }) => Promise<void>;
	selectedRootPath: string | null;
	centerSelection: CenterSelection | null;
	autoSelectPreferredNode: boolean;
	newSessionCreationStatusByWorktree?: Record<string, NewSessionCreationStatus>;
	onSelectWorktree: WorkspaceListProps["onSelectWorktree"];
	onCreateSession: WorkspaceListProps["onCreateSession"];
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
					await invoke("kill_ptys_by_worktree", {
						worktreePath: branch.worktree_path,
					}).catch(() => {});
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
								newSessionCreationStatus={
									branch.worktree_path
										? newSessionCreationStatusByWorktree?.[branch.worktree_path]
										: undefined
								}
								onSelectWorktree={onSelectWorktree}
								onCreateSession={onCreateSession}
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
	newSessionCreationStatusByWorktree,
	onSelectWorktree,
	onCreateSession,
}: {
	repoPath: string;
	selectedRootPath: string | null;
	centerSelection: CenterSelection | null;
	autoSelectPreferredNode: boolean;
	newSessionCreationStatusByWorktree?: Record<string, NewSessionCreationStatus>;
	onSelectWorktree: WorkspaceListProps["onSelectWorktree"];
	onCreateSession: WorkspaceListProps["onCreateSession"];
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
			newSessionCreationStatusByWorktree={newSessionCreationStatusByWorktree}
			onSelectWorktree={onSelectWorktree}
			onCreateSession={onCreateSession}
		/>
	);
}

export function WorkspaceList({
	repoPaths,
	selectedRootPath,
	centerSelection,
	autoSelectPreferredNode = false,
	newSessionCreationStatusByWorktree,
	onSelectWorktree,
	onCreateSession,
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
						newSessionCreationStatusByWorktree={
							newSessionCreationStatusByWorktree
						}
						onSelectWorktree={onSelectWorktree}
						onCreateSession={onCreateSession}
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
