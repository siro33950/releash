import { AlertTriangle, Ban, RotateCcw, Square, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type TogglePanel, ViewToolbar } from "@/components/layout/ViewToolbar";
import { BoundSessionChat } from "@/components/panels/AgentChatPanel";
import { AgentStateIcon } from "@/components/ui/agent-state-icon";
import { Button } from "@/components/ui/button";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import { WorkflowNodeStatusIcon } from "@/components/workspace/WorkflowNodeStatusIcon";
import {
	submitWorkspaceWorkflowNodeAction,
	useWorkspaceWorkflowNodeDetail,
} from "@/hooks/useWorkspaceWorkflowNodeDetail";
import { useWorktreeSessionStatuses } from "@/hooks/useWorktreeSessionStatuses";
import {
	executeWorkflowAction,
	type WorkflowExecutionAction,
} from "@/lib/workflowExecutionActions";
import type { AgentState } from "@/types/protocol";
import type { Artifact } from "@/types/workflow";
import type {
	CenterSelection,
	CenterSelectionRequest,
	WorkspaceWorkflowNodeDetail,
} from "@/types/workspace-tree";

interface WorkflowViewProps {
	worktreePath: string;
	selectionRequest?: CenterSelection | CenterSelectionRequest | null;
	leftPanels?: TogglePanel[];
	rightSlot?: React.ReactNode;
}

interface WorkflowGridPane {
	key: string;
	label: string;
	sessionId?: string;
	worktreePath: string;
	agentState?: AgentState | null;
	artifact?: Artifact;
}

const WORKFLOW_GRID_MIN_TILE_WIDTH = 320;
const WORKFLOW_GRID_MIN_TILE_HEIGHT = 360;
const WORKFLOW_GRID_GAP = 8;
const WORKFLOW_GRID_PADDING = 8;

export function WorkflowView({
	worktreePath,
	selectionRequest,
	leftPanels,
	rightSlot,
}: WorkflowViewProps) {
	const nodeSelection =
		selectionRequest?.kind === "workflowNode" &&
		selectionRequest.worktreePath === worktreePath
			? selectionRequest
			: null;
	const node = useWorkspaceWorkflowNodeDetail({
		worktreePath: nodeSelection ? worktreePath : null,
		executionId: nodeSelection?.executionId ?? null,
		nodeExecutionId: nodeSelection?.nodeExecutionId ?? null,
	});
	const nodeDetail = node.detail;

	const headerContent = nodeDetail ? (
		<WorkflowNodeHeader
			key={`${nodeDetail.executionId}:${nodeDetail.nodeExecutionId}`}
			node={nodeDetail}
		/>
	) : null;

	return (
		<div className="flex h-full min-h-0 flex-col overflow-hidden">
			<ViewToolbar
				leftPanels={leftPanels}
				centerSlot={headerContent}
				edgePadding="grid"
				rightSlot={rightSlot}
			/>
			<div className="min-h-0 flex-1 overflow-hidden">
				<WorkspaceWorkflowNodeGrid
					node={nodeDetail}
					loading={node.loading}
					error={node.error}
					worktreePath={worktreePath}
				/>
			</div>
		</div>
	);
}

function WorkspaceWorkflowNodeGrid({
	node,
	loading,
	error,
	worktreePath,
}: {
	node: WorkspaceWorkflowNodeDetail | null;
	loading: boolean;
	error: string | null;
	worktreePath: string;
}) {
	// Pane ヘッダーの status アイコンは Rust 中央管理の SessionStatus.agent_state を
	// そのまま消費する（フロントでの導出は禁止）。
	const sessionStatuses = useWorktreeSessionStatuses(worktreePath);
	const [activePaneKey, setActivePaneKey] = useState<string | null>(
		() => node?.sessions[0]?.id ?? null,
	);
	useEffect(() => {
		if (!node) {
			setActivePaneKey(null);
			return;
		}
		if (node.sessions.some((session) => session.id === activePaneKey)) {
			return;
		}
		setActivePaneKey(node.sessions[0]?.id ?? null);
	}, [activePaneKey, node]);

	const panes = useMemo<WorkflowGridPane[]>(() => {
		if (!node) return [];
		if (node.sessions.length === 0) {
			return [
				{
					key: `${node.nodeExecutionId}:empty`,
					label: node.title,
					worktreePath: node.worktreePath,
					artifact: node.artifact,
				},
			];
		}
		return node.sessions.map((session) => ({
			key: session.id,
			label: session.title,
			sessionId: session.id,
			worktreePath: session.worktreePath,
			agentState: sessionStatuses.get(session.id)?.agent_state ?? null,
		}));
	}, [node, sessionStatuses]);

	if (!node) {
		return (
			<div className="flex h-full flex-col items-center justify-center gap-1 bg-background px-4 text-center text-sm text-muted-foreground">
				<div>{loading ? "Loading Node..." : "Node unavailable"}</div>
				{error && <div className="max-w-md break-words text-xs">{error}</div>}
			</div>
		);
	}

	return (
		<div className="flex h-full min-h-0 flex-col bg-background">
			<div className="min-h-0 flex-1 overflow-hidden">
				<WorkflowNodeGrid
					panes={panes}
					activePaneKey={activePaneKey}
					onSelectPane={setActivePaneKey}
				/>
			</div>
		</div>
	);
}

function WorkflowNodeHeader({ node }: { node: WorkspaceWorkflowNodeDetail }) {
	const [approving, setApproving] = useState(false);
	const [executionAction, setExecutionAction] =
		useState<WorkflowExecutionAction | null>(null);
	const [actionError, setActionError] = useState<string | null>(null);
	const [actionErrorOpen, setActionErrorOpen] = useState(false);

	const handleApprove = useCallback(async () => {
		if (approving || executionAction !== null) return false;
		setApproving(true);
		try {
			const result = await submitWorkspaceWorkflowNodeAction({
				worktreePath: node.worktreePath,
				executionId: node.executionId,
				nodeName: node.nodeName,
				nodeExecutionId: node.nodeExecutionId,
			});
			if (!result) {
				throw new Error("Workflow node action failed.");
			}
			setActionError(null);
			setActionErrorOpen(false);
			return true;
		} catch (error) {
			setActionError(error instanceof Error ? error.message : String(error));
			setActionErrorOpen(true);
			return false;
		} finally {
			setApproving(false);
		}
	}, [approving, executionAction, node]);

	const handleExecutionAction = useCallback(
		async (action: WorkflowExecutionAction, allowed: boolean) => {
			if (!allowed || approving || executionAction !== null) return false;
			setExecutionAction(action);
			try {
				await executeWorkflowAction(action, node.executionId);
				setActionError(null);
				setActionErrorOpen(false);
				window.dispatchEvent(
					new CustomEvent("workspace-tree-refresh", {
						detail: { worktreePath: node.worktreePath },
					}),
				);
				return true;
			} catch (error) {
				setActionError(error instanceof Error ? error.message : String(error));
				setActionErrorOpen(true);
				return false;
			} finally {
				setExecutionAction(null);
			}
		},
		[approving, executionAction, node.executionId, node.worktreePath],
	);

	return (
		<WorkflowNodeHeaderContent
			node={node}
			approving={approving}
			executionAction={executionAction}
			actionError={actionError}
			actionErrorOpen={actionErrorOpen}
			onActionErrorOpenChange={setActionErrorOpen}
			onApprove={handleApprove}
			onStop={() => handleExecutionAction("stop", node.canStop)}
			onResume={() => handleExecutionAction("resume", node.canResume)}
			onAbort={() => handleExecutionAction("abort", node.canAbort)}
		/>
	);
}

function WorkflowNodeHeaderContent({
	node,
	approving,
	executionAction,
	actionError,
	actionErrorOpen,
	onActionErrorOpenChange,
	onApprove,
	onStop,
	onResume,
	onAbort,
}: {
	node: WorkspaceWorkflowNodeDetail;
	approving: boolean;
	executionAction: WorkflowExecutionAction | null;
	actionError: string | null;
	actionErrorOpen: boolean;
	onActionErrorOpenChange: (open: boolean) => void;
	onApprove: () => Promise<boolean>;
	onStop: () => Promise<boolean>;
	onResume: () => Promise<boolean>;
	onAbort: () => Promise<boolean>;
}) {
	const approve = useCallback(() => {
		void onApprove();
	}, [onApprove]);
	return (
		<div className="flex min-w-0 flex-1 items-center gap-3 pl-2">
			<div className="flex min-w-0 items-center gap-2">
				<WorkflowNodeStatusIcon status={node.status} />
				<span className="min-w-0 truncate text-sm font-medium">
					{node.nodeName}
				</span>
				<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
					{node.nodeKind}
				</span>
				<span className="text-xs text-muted-foreground">
					{node.nodeExecutionStatus ?? node.status}
				</span>
				<span className="text-xs text-muted-foreground">
					attempt {node.attempt}
				</span>
				<span
					className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground"
					title={`Execution status: ${node.executionStatus}`}
				>
					execution: {node.executionStatus}
				</span>
				{node.interruptionReason && (
					<span
						className="rounded bg-orange-500/10 px-1.5 py-0.5 text-[10px] text-orange-700 dark:text-orange-300"
						title={`Interrupted: ${node.interruptionReason}`}
					>
						{node.interruptionReason}
					</span>
				)}
				{node.resumeFromNode && (
					<span
						className="max-w-28 truncate text-[10px] text-muted-foreground"
						title={`Resume from ${node.resumeFromNode}`}
					>
						resume: {node.resumeFromNode}
					</span>
				)}
				<span
					className="max-w-36 truncate font-mono text-[10px] text-muted-foreground"
					title={`NodeExecution ${node.nodeExecutionId}`}
				>
					{node.nodeExecutionId}
				</span>
				{node.sessionId && (
					<span
						className="max-w-32 truncate font-mono text-[10px] text-muted-foreground"
						title={`Session ${node.sessionId}`}
					>
						{node.sessionId}
					</span>
				)}
				{node.fanoutParent && (
					<span
						className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground"
						title={`fanout parent ${node.fanoutParent.parentNode} attempt ${node.fanoutParent.parentAttempt}`}
					>
						{node.fanoutParent.parentNode}#{node.fanoutParent.parentAttempt}
						{" · item "}
						{node.fanoutParent.itemIndex ?? "–"}
						{" · child "}
						{node.fanoutParent.childIndex}
					</span>
				)}
			</div>
			<div className="ml-auto flex shrink-0 items-center gap-2">
				{node.artifact !== undefined && (
					<Popover>
						<PopoverTrigger asChild>
							<Button type="button" variant="outline" size="xs">
								Artifact
							</Button>
						</PopoverTrigger>
						<PopoverContent side="bottom" align="end" className="w-96 p-0">
							<pre className="max-h-80 overflow-auto p-3 text-xs">
								{JSON.stringify(node.artifact, null, 2)}
							</pre>
						</PopoverContent>
					</Popover>
				)}
				{actionError && (
					<Popover
						open={actionErrorOpen}
						onOpenChange={onActionErrorOpenChange}
					>
						<PopoverTrigger asChild>
							<Button
								type="button"
								variant="ghost"
								size="icon-xs"
								className="size-6 text-destructive hover:text-destructive"
								aria-label="Show action error"
								title="Action error"
							>
								<AlertTriangle className="size-3.5" />
							</Button>
						</PopoverTrigger>
						<PopoverContent side="bottom" align="end" className="w-80 p-3">
							<div className="flex items-start gap-2">
								<AlertTriangle className="mt-0.5 size-3.5 shrink-0 text-destructive" />
								<div className="min-w-0 flex-1">
									<div className="text-sm font-medium">Action failed</div>
									<div className="mt-1 break-words text-xs text-muted-foreground">
										{actionError}
									</div>
								</div>
								<Button
									type="button"
									variant="ghost"
									size="icon-xs"
									className="size-6"
									aria-label="Close action error"
									onClick={() => onActionErrorOpenChange(false)}
								>
									<X className="size-3" />
								</Button>
							</div>
						</PopoverContent>
					</Popover>
				)}
				{node.canApprove === true && (
					<Button
						type="button"
						size="xs"
						disabled={approving || executionAction !== null}
						onClick={approve}
					>
						{approving ? "Approving..." : "Approve"}
					</Button>
				)}
				<Button
					type="button"
					variant="outline"
					size="xs"
					disabled={!node.canStop || approving || executionAction !== null}
					onClick={() => void onStop()}
				>
					<Square className="size-3.5" />
					{executionAction === "stop" ? "Stopping..." : "Stop"}
				</Button>
				<Button
					type="button"
					variant="outline"
					size="xs"
					disabled={!node.canResume || approving || executionAction !== null}
					onClick={() => void onResume()}
				>
					<RotateCcw className="size-3.5" />
					{executionAction === "resume" ? "Resuming..." : "Resume"}
				</Button>
				<Button
					type="button"
					variant="destructive"
					size="xs"
					disabled={!node.canAbort || approving || executionAction !== null}
					onClick={() => void onAbort()}
				>
					<Ban className="size-3.5" />
					{executionAction === "abort" ? "Aborting..." : "Abort"}
				</Button>
			</div>
		</div>
	);
}

function WorkflowNodeGrid({
	panes,
	activePaneKey,
	onSelectPane,
}: {
	panes: WorkflowGridPane[];
	activePaneKey: string | null;
	onSelectPane: (key: string) => void;
}) {
	const paneCount = Math.max(1, panes.length);
	const { containerRef, columns, rowHeight } = useWorkflowGridLayout(paneCount);
	const rows = Math.max(1, Math.ceil(paneCount / columns));
	const gridHeight = rows * rowHeight + (rows - 1) * WORKFLOW_GRID_GAP;

	return (
		<div
			ref={containerRef}
			className="h-full min-h-0 overflow-x-hidden overflow-y-auto bg-background p-2"
		>
			<div
				data-testid="workflow-node-grid"
				className="grid w-full min-w-0 gap-2"
				style={{
					height: gridHeight,
					gridTemplateColumns: `repeat(${columns}, minmax(${WORKFLOW_GRID_MIN_TILE_WIDTH}px, 1fr))`,
					gridTemplateRows: `repeat(${rows}, ${rowHeight}px)`,
				}}
			>
				{panes.map((pane) => (
					<WorkflowNodePane
						key={pane.key}
						pane={pane}
						selected={activePaneKey === pane.key}
						onSelect={() => onSelectPane(pane.key)}
					/>
				))}
			</div>
		</div>
	);
}

function WorkflowNodePane({
	pane,
	selected,
	onSelect,
}: {
	pane: WorkflowGridPane;
	selected: boolean;
	onSelect: () => void;
}) {
	return (
		<div
			data-testid="workflow-node-grid-tile"
			data-active={selected ? "true" : undefined}
			title={pane.label}
			onPointerDown={onSelect}
			className={`flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-md border bg-background transition-colors ${
				selected ? "border-primary/60" : "border-border hover:bg-muted/30"
			}`}
		>
			<div className="flex shrink-0 items-center gap-2 border-b border-border px-2 py-1">
				<AgentStateIcon state={pane.agentState} />
				<span className="min-w-0 truncate text-xs font-medium">
					{pane.label}
				</span>
			</div>
			<div className="flex min-h-0 flex-1 flex-col overflow-hidden">
				{pane.sessionId ? (
					<BoundSessionChat
						sessionId={pane.sessionId}
						worktreePath={pane.worktreePath}
					/>
				) : pane.artifact !== undefined ? (
					<pre className="h-full overflow-auto p-4 text-xs">
						{JSON.stringify(pane.artifact.value, null, 2)}
					</pre>
				) : (
					<div className="flex h-full items-center justify-center px-4 text-center text-sm text-muted-foreground">
						No agent conversation for this node.
					</div>
				)}
			</div>
		</div>
	);
}

function useWorkflowGridLayout(count: number) {
	const containerRef = useRef<HTMLDivElement | null>(null);
	const [layout, setLayout] = useState({
		columns: 1,
		rowHeight: WORKFLOW_GRID_MIN_TILE_HEIGHT,
	});

	useEffect(() => {
		const element = containerRef.current;
		if (!element) return;

		const update = () => {
			const width = Math.max(
				0,
				element.clientWidth - WORKFLOW_GRID_PADDING * 2,
			);
			const maxByWidth = Math.max(
				1,
				Math.floor(
					(width + WORKFLOW_GRID_GAP) /
						(WORKFLOW_GRID_MIN_TILE_WIDTH + WORKFLOW_GRID_GAP),
				),
			);
			const ideal = Math.ceil(Math.sqrt(count));
			const columns = Math.max(1, Math.min(count, ideal, maxByWidth));
			const rows = Math.max(1, Math.ceil(count / columns));
			const height = Math.max(
				0,
				element.clientHeight - WORKFLOW_GRID_PADDING * 2,
			);
			const rowSpace = Math.max(0, height - (rows - 1) * WORKFLOW_GRID_GAP);
			const rowHeight = Math.max(
				WORKFLOW_GRID_MIN_TILE_HEIGHT,
				Math.floor(rowSpace / rows),
			);
			setLayout((prev) =>
				prev.columns === columns && prev.rowHeight === rowHeight
					? prev
					: { columns, rowHeight },
			);
		};

		update();
		if (typeof ResizeObserver === "undefined") {
			window.addEventListener("resize", update);
			return () => window.removeEventListener("resize", update);
		}
		const observer = new ResizeObserver(update);
		observer.observe(element);
		return () => observer.disconnect();
	}, [count]);

	return { containerRef, ...layout };
}
