import { AlertTriangle, X } from "lucide-react";
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
import { Textarea } from "@/components/ui/textarea";
import { WorkflowStepStatusIcon } from "@/components/workspace/WorkflowStepStatusIcon";
import {
	submitWorkspaceWorkflowStepAction,
	useWorkspaceWorkflowStepDetail,
	type WorkspaceWorkflowStepAction,
} from "@/hooks/useWorkspaceWorkflowStepDetail";
import { useWorktreeSessionStatuses } from "@/hooks/useWorktreeSessionStatuses";
import type { AgentState } from "@/types/protocol";
import type {
	CenterSelection,
	CenterSelectionRequest,
	WorkspaceWorkflowStepDetail,
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
	const stepSelection =
		selectionRequest?.kind === "workflowStep" &&
		selectionRequest.worktreePath === worktreePath
			? selectionRequest
			: null;
	const step = useWorkspaceWorkflowStepDetail({
		worktreePath: stepSelection ? worktreePath : null,
		runId: stepSelection?.runId ?? null,
		stepId: stepSelection?.stepId ?? null,
	});
	const stepDetail = step.detail;

	const headerContent = stepDetail ? (
		<WorkflowStepHeader
			key={`${stepDetail.runId}:${stepDetail.id}`}
			step={stepDetail}
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
				<WorkspaceWorkflowStepGrid
					step={stepDetail}
					loading={step.loading}
					error={step.error}
					worktreePath={worktreePath}
				/>
			</div>
		</div>
	);
}

function WorkspaceWorkflowStepGrid({
	step,
	loading,
	error,
	worktreePath,
}: {
	step: WorkspaceWorkflowStepDetail | null;
	loading: boolean;
	error: string | null;
	worktreePath: string;
}) {
	// Pane ヘッダーの status アイコンは Rust 中央管理の SessionStatus.agent_state を
	// そのまま消費する（フロントでの導出は禁止）。
	const sessionStatuses = useWorktreeSessionStatuses(worktreePath);
	const [activePaneKey, setActivePaneKey] = useState<string | null>(
		() => step?.sessions[0]?.id ?? null,
	);
	useEffect(() => {
		if (!step) {
			setActivePaneKey(null);
			return;
		}
		if (step.sessions.some((session) => session.id === activePaneKey)) {
			return;
		}
		setActivePaneKey(step.sessions[0]?.id ?? null);
	}, [activePaneKey, step]);

	const panes = useMemo<WorkflowGridPane[]>(() => {
		if (!step) return [];
		if (step.sessions.length === 0) {
			return [
				{
					key: `${step.id}:empty`,
					label: step.title,
					worktreePath: step.worktreePath,
				},
			];
		}
		return step.sessions.map((session) => ({
			key: session.id,
			label: session.title,
			sessionId: session.id,
			worktreePath: session.worktreePath,
			agentState: sessionStatuses.get(session.id)?.agent_state ?? null,
		}));
	}, [step, sessionStatuses]);

	if (!step) {
		return (
			<div className="flex h-full flex-col items-center justify-center gap-1 bg-background px-4 text-center text-sm text-muted-foreground">
				<div>{loading ? "Loading Step..." : "Step unavailable"}</div>
				{error && <div className="max-w-md break-words text-xs">{error}</div>}
			</div>
		);
	}

	return (
		<div className="flex h-full min-h-0 flex-col bg-background">
			<div className="min-h-0 flex-1 overflow-hidden">
				<WorkflowStepGrid
					panes={panes}
					activePaneKey={activePaneKey}
					onSelectPane={setActivePaneKey}
				/>
			</div>
		</div>
	);
}

function WorkflowStepHeader({ step }: { step: WorkspaceWorkflowStepDetail }) {
	const [pendingAction, setPendingAction] =
		useState<WorkspaceWorkflowStepAction | null>(null);
	const [actionError, setActionError] = useState<string | null>(null);
	const [actionErrorOpen, setActionErrorOpen] = useState(false);

	const handleStepAction = useCallback(
		async (action: WorkspaceWorkflowStepAction, reason?: string) => {
			if (pendingAction) return false;
			setPendingAction(action);
			try {
				const result = await submitWorkspaceWorkflowStepAction({
					worktreePath: step.worktreePath,
					runId: step.runId,
					stepId: step.id,
					stepName: step.title,
					action,
					reason,
				});
				if (!result) {
					throw new Error("Workflow step action failed.");
				}
				setActionError(null);
				setActionErrorOpen(false);
				return true;
			} catch (error) {
				setActionError(error instanceof Error ? error.message : String(error));
				setActionErrorOpen(true);
				return false;
			} finally {
				setPendingAction(null);
			}
		},
		[pendingAction, step],
	);

	return (
		<WorkflowStepHeaderContent
			step={step}
			pendingAction={pendingAction}
			actionError={actionError}
			actionErrorOpen={actionErrorOpen}
			onActionErrorOpenChange={setActionErrorOpen}
			onAction={handleStepAction}
		/>
	);
}

function WorkflowStepHeaderContent({
	step,
	pendingAction,
	actionError,
	actionErrorOpen,
	onActionErrorOpenChange,
	onAction,
}: {
	step: WorkspaceWorkflowStepDetail;
	pendingAction: WorkspaceWorkflowStepAction | null;
	actionError: string | null;
	actionErrorOpen: boolean;
	onActionErrorOpenChange: (open: boolean) => void;
	onAction: (
		action: WorkspaceWorkflowStepAction,
		reason?: string,
	) => Promise<boolean>;
}) {
	const [rejectOpen, setRejectOpen] = useState(false);
	const [rejectComment, setRejectComment] = useState("");
	const canRespondApproval = step.status === "waiting_approval";
	const canReject = canRespondApproval && step.canReject !== false;
	const canSubmitReject =
		rejectComment.trim().length > 0 && pendingAction == null;
	const submitReject = useCallback(async () => {
		if (!canSubmitReject) return;
		const ok = await onAction("reject", rejectComment.trim());
		if (!ok) return;
		setRejectComment("");
		setRejectOpen(false);
	}, [canSubmitReject, onAction, rejectComment]);
	const approve = useCallback(() => {
		void onAction("approve");
	}, [onAction]);
	return (
		<div className="flex min-w-0 flex-1 items-center gap-3 pl-2">
			<div className="flex min-w-0 items-center gap-2">
				<WorkflowStepStatusIcon status={step.status} />
				<span className="min-w-0 truncate text-sm font-medium">
					{step.title}
				</span>
				<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
					{step.stepType}
				</span>
			</div>
			<div className="ml-auto flex shrink-0 items-center gap-2">
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
				{canRespondApproval && (
					<>
						{canReject && (
							<Popover open={rejectOpen} onOpenChange={setRejectOpen}>
								<PopoverTrigger asChild>
									<Button
										type="button"
										variant="outline"
										size="xs"
										disabled={pendingAction != null}
									>
										{pendingAction === "reject" ? "Rejecting..." : "Reject"}
									</Button>
								</PopoverTrigger>
								<PopoverContent side="bottom" align="end" className="w-80 p-3">
									<div className="flex flex-col gap-2">
										<div className="text-sm font-medium">Reject step</div>
										<Textarea
											value={rejectComment}
											onChange={(event) => setRejectComment(event.target.value)}
											placeholder="Reject comment..."
											className="min-h-20 resize-none text-sm"
											aria-label="Reject comment"
										/>
										<div className="flex justify-end gap-2">
											<Button
												type="button"
												variant="ghost"
												size="xs"
												onClick={() => {
													setRejectComment("");
													setRejectOpen(false);
												}}
											>
												Cancel
											</Button>
											<Button
												type="button"
												size="xs"
												disabled={!canSubmitReject}
												onClick={() => void submitReject()}
											>
												Reject
											</Button>
										</div>
									</div>
								</PopoverContent>
							</Popover>
						)}
						<Button
							type="button"
							size="xs"
							disabled={pendingAction != null}
							onClick={approve}
						>
							{pendingAction === "approve" ? "Approving..." : "Approve"}
						</Button>
					</>
				)}
			</div>
		</div>
	);
}

function WorkflowStepGrid({
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
				data-testid="workflow-step-grid"
				className="grid w-full min-w-0 gap-2"
				style={{
					height: gridHeight,
					gridTemplateColumns: `repeat(${columns}, minmax(${WORKFLOW_GRID_MIN_TILE_WIDTH}px, 1fr))`,
					gridTemplateRows: `repeat(${rows}, ${rowHeight}px)`,
				}}
			>
				{panes.map((pane) => (
					<WorkflowStepPane
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

function WorkflowStepPane({
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
			data-testid="workflow-step-grid-tile"
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
