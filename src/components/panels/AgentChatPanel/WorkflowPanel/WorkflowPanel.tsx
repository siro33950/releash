import { invoke } from "@tauri-apps/api/core";
import { Check, History, Square, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useWorkflowConfig } from "@/hooks/useWorkflowConfig";
import type { WorkflowLogEvent, WorkflowState } from "@/types/workflow";
import { WorkflowTrace } from "./WorkflowTrace";

interface WorkflowPanelProps {
	workflowState: WorkflowState | null;
	worktreePath: string;
	chatSessionId: string | null;
	onSessionClick?: (sessionId: string) => void;
}

export function WorkflowPanel({
	workflowState,
	worktreePath,
	chatSessionId,
	onSessionClick,
}: WorkflowPanelProps) {
	const [executionIds, setExecutionIds] = useState<string[]>([]);
	const [openPastIds, setOpenPastIds] = useState<string[]>([]);
	const [activeTab, setActiveTab] = useState<string>("current");
	const [historyOpen, setHistoryOpen] = useState(false);

	const fetchExecutionIds = useCallback(() => {
		invoke<string[]>("list_workflow_executions", { worktreePath })
			.then(setExecutionIds)
			.catch((e) =>
				console.warn("[WorkflowPanel] list_workflow_executions failed", e),
			);
	}, [worktreePath]);

	useEffect(() => {
		fetchExecutionIds();
	}, [fetchExecutionIds]);

	// Refresh on workflow completion
	const stateType = workflowState?.state.type;
	useEffect(() => {
		if (
			stateType === "completed" ||
			stateType === "failed" ||
			stateType === "aborted"
		) {
			fetchExecutionIds();
		}
	}, [stateType, fetchExecutionIds]);

	// Switch to current tab when a new workflow starts
	const executionId = workflowState?.executionId;
	useEffect(() => {
		if (executionId) {
			setActiveTab("current");
		}
	}, [executionId]);

	// Reset when worktree changes
	// biome-ignore lint/correctness/useExhaustiveDependencies: worktreePath change should reset state
	useEffect(() => {
		setActiveTab("current");
		setOpenPastIds([]);
	}, [worktreePath]);

	const pastExecutionIds = useMemo(
		() => executionIds.filter((id) => id !== executionId),
		[executionIds, executionId],
	);

	const visiblePastIds = useMemo(
		() => openPastIds.filter((id) => pastExecutionIds.includes(id)),
		[openPastIds, pastExecutionIds],
	);

	const closedPastIds = useMemo(
		() => pastExecutionIds.filter((id) => !openPastIds.includes(id)),
		[pastExecutionIds, openPastIds],
	);

	const hasCurrent = workflowState !== null;
	const hasVisibleTabs = hasCurrent || visiblePastIds.length > 0;

	// Compute effective tab (handle invalid active tab)
	let effectiveTab = activeTab;
	if (!hasVisibleTabs) {
		effectiveTab = "";
	} else if (activeTab === "current" && !hasCurrent) {
		effectiveTab = visiblePastIds[0] ?? "";
	} else if (activeTab !== "current" && !visiblePastIds.includes(activeTab)) {
		effectiveTab = hasCurrent ? "current" : (visiblePastIds[0] ?? "");
	}

	const handleClosePastTab = useCallback(
		(id: string) => {
			setOpenPastIds((prev) => prev.filter((pid) => pid !== id));
			if (activeTab === id) {
				const remaining = visiblePastIds.filter((pid) => pid !== id);
				setActiveTab(hasCurrent ? "current" : (remaining[0] ?? ""));
			}
		},
		[activeTab, visiblePastIds, hasCurrent],
	);

	const handleRestoreExecution = useCallback((id: string) => {
		setOpenPastIds((prev) => (prev.includes(id) ? prev : [...prev, id]));
		setActiveTab(id);
		setHistoryOpen(false);
	}, []);

	return (
		<Tabs
			value={effectiveTab}
			onValueChange={setActiveTab}
			className="flex flex-col h-full gap-0"
		>
			<div className="flex items-center gap-2 shrink-0 px-2 pt-2 bg-background border-b">
				<TabsList className="w-auto max-w-full overflow-x-auto overflow-y-hidden justify-start [&::-webkit-scrollbar]:hidden [scrollbar-width:none]">
					{hasCurrent && (
						<TabsTrigger value="current" className="gap-1.5">
							<span className="truncate max-w-[120px]">
								{workflowState.workflowName}
							</span>
							<StatusBadge state={workflowState.state.type} />
						</TabsTrigger>
					)}
					{visiblePastIds.map((id) => (
						<TabsTrigger key={id} value={id} asChild>
							{/* biome-ignore lint/a11y/noStaticElementInteractions: TabsTrigger asChild assigns role */}
							{/* biome-ignore lint/a11y/useKeyWithClickEvents: TabsTrigger handles keyboard */}
							<div className="gap-2" onClick={() => setActiveTab(id)}>
								<span className="truncate max-w-[120px]">{id.slice(0, 8)}</span>
								<button
									type="button"
									onPointerDown={(e) => e.stopPropagation()}
									onMouseDown={(e) => e.stopPropagation()}
									onClick={(e) => {
										e.stopPropagation();
										handleClosePastTab(id);
									}}
									className="p-0.5 rounded hover:bg-muted-foreground/20 transition-colors shrink-0"
									aria-label={`Close ${id.slice(0, 8)}`}
								>
									<X className="size-3.5" />
								</button>
							</div>
						</TabsTrigger>
					))}
				</TabsList>
				<div className="flex-1" />
				{chatSessionId && <NewWorkflowButton chatSessionId={chatSessionId} />}
				<Popover open={historyOpen} onOpenChange={setHistoryOpen}>
					<PopoverTrigger asChild>
						<button
							type="button"
							aria-label="Execution history"
							className="p-1 rounded hover:bg-muted-foreground/20 transition-colors shrink-0"
						>
							<History className="size-3.5" />
						</button>
					</PopoverTrigger>
					<PopoverContent align="end" className="w-64 p-0">
						{closedPastIds.length > 0 ? (
							<ul className="max-h-60 overflow-y-auto">
								{closedPastIds.map((id) => (
									<li key={id}>
										<button
											type="button"
											className="w-full text-left px-3 py-2 text-sm hover:bg-muted transition-colors truncate"
											onClick={() => handleRestoreExecution(id)}
										>
											{id}
										</button>
									</li>
								))}
							</ul>
						) : (
							<p className="px-3 py-4 text-sm text-muted-foreground text-center">
								No execution history
							</p>
						)}
					</PopoverContent>
				</Popover>
			</div>

			{hasCurrent && (
				<TabsContent value="current" className="flex-1 min-h-0 mt-0">
					<WorkflowActivePanel
						workflowState={workflowState}
						worktreePath={worktreePath}
						onSessionClick={onSessionClick}
					/>
				</TabsContent>
			)}

			{visiblePastIds.map((id) => (
				<TabsContent key={id} value={id} className="flex-1 min-h-0 mt-0">
					<ExecutionView executionId={id} onSessionClick={onSessionClick} />
				</TabsContent>
			))}

			{!hasVisibleTabs && (
				<div className="flex-1 flex items-center justify-center text-sm text-muted-foreground">
					No workflow running
				</div>
			)}
		</Tabs>
	);
}

function NewWorkflowButton({ chatSessionId }: { chatSessionId: string }) {
	const [open, setOpen] = useState(false);
	const { workflows } = useWorkflowConfig(open);

	const handleStart = useCallback(
		(workflowName: string) => {
			setOpen(false);
			invoke("start_workflow", {
				workflowName,
				chatSessionId,
			}).catch((e) => console.warn("[WorkflowPanel] start_workflow failed", e));
		},
		[chatSessionId],
	);

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					aria-label="New workflow"
					className="px-2 h-full text-sm text-muted-foreground hover:text-foreground transition-colors shrink-0"
				>
					+
				</button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-48 p-0">
				{workflows.length > 0 ? (
					<ul>
						{workflows.map((wf) => (
							<li key={wf.name}>
								<button
									type="button"
									className="w-full text-left px-3 py-2 text-sm hover:bg-muted transition-colors"
									onClick={() => handleStart(wf.name)}
								>
									<div className="font-medium truncate text-foreground">
										{wf.name}
									</div>
									{wf.description && (
										<div className="text-xs text-muted-foreground truncate">
											{wf.description}
										</div>
									)}
								</button>
							</li>
						))}
					</ul>
				) : (
					<p className="px-3 py-4 text-sm text-muted-foreground text-center">
						No workflows configured
					</p>
				)}
			</PopoverContent>
		</Popover>
	);
}

function ExecutionView({
	executionId,
	onSessionClick,
}: {
	executionId: string;
	onSessionClick?: (sessionId: string) => void;
}) {
	const [historyState, setHistoryState] = useState<WorkflowState | null>(null);
	const [events, setEvents] = useState<WorkflowLogEvent[]>([]);

	useEffect(() => {
		invoke<WorkflowState | null>("get_workflow_execution_state", {
			executionId,
		})
			.then((state) => setHistoryState(state ?? null))
			.catch((e) =>
				console.warn("[ExecutionView] get_workflow_execution_state failed", e),
			);

		invoke<WorkflowLogEvent[]>("get_workflow_execution_log", {
			executionId,
		})
			.then(setEvents)
			.catch((e) =>
				console.warn("[ExecutionView] get_workflow_execution_log failed", e),
			);
	}, [executionId]);

	if (!historyState) {
		return (
			<div className="flex items-center justify-center h-full text-sm text-muted-foreground">
				Loading...
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full overflow-hidden">
			{/* Header */}
			<div className="flex items-center justify-between px-3 py-2 border-b shrink-0">
				<div className="flex items-center gap-2">
					<span className="text-sm font-medium">
						{historyState.workflowName}
					</span>
					<StatusBadge state={historyState.state.type} />
				</div>
			</div>

			{/* Trace */}
			<div className="flex-1 overflow-auto min-h-0">
				<WorkflowTrace
					workflowState={historyState}
					events={events}
					onSessionClick={onSessionClick}
				/>
			</div>
		</div>
	);
}

function WorkflowActivePanel({
	workflowState,
	worktreePath,
	onSessionClick,
}: {
	workflowState: WorkflowState;
	worktreePath: string;
	onSessionClick?: (sessionId: string) => void;
}) {
	const isRunning =
		workflowState.state.type === "running" ||
		workflowState.state.type === "waiting_approval";

	const currentStep =
		workflowState.workflowDefinition.steps[workflowState.currentStepIndex];
	const isWaitingApproval = workflowState.state.type === "waiting_approval";
	const isInteractiveRunning =
		currentStep?.mode === "interactive" &&
		workflowState.state.type === "running";

	const handleAbort = useCallback(() => {
		invoke("abort_workflow", { worktreePath }).catch((e) =>
			console.warn("[WorkflowPanel] abort_workflow failed", e),
		);
	}, [worktreePath]);

	const handleApprove = useCallback(() => {
		invoke("approve_workflow_step", {
			worktreePath,
			decision: "approve",
		}).catch((e) =>
			console.warn("[WorkflowPanel] approve_workflow_step failed", e),
		);
	}, [worktreePath]);

	const handleReject = useCallback(() => {
		invoke("approve_workflow_step", {
			worktreePath,
			decision: "reject",
		}).catch((e) =>
			console.warn("[WorkflowPanel] approve_workflow_step failed", e),
		);
	}, [worktreePath]);

	const handleCompleteInteractive = useCallback(() => {
		invoke("complete_interactive_step", {
			worktreePath,
			abort: false,
		}).catch((e) =>
			console.warn("[WorkflowPanel] complete_interactive_step failed", e),
		);
	}, [worktreePath]);

	return (
		<div className="flex flex-col h-full overflow-hidden">
			{/* Action bar */}
			<div className="flex items-center justify-end px-3 py-1.5 border-b shrink-0">
				<div className="flex items-center gap-2">
					{isWaitingApproval && (
						<>
							<button
								type="button"
								onClick={handleApprove}
								className="flex items-center gap-1 px-2 py-0.5 text-xs rounded bg-green-500/20 text-green-700 dark:text-green-300 hover:bg-green-500/30 transition-colors"
								aria-label="Approve step"
							>
								<Check className="size-3" />
								Approve
							</button>
							<button
								type="button"
								onClick={handleReject}
								className="flex items-center gap-1 px-2 py-0.5 text-xs rounded bg-yellow-500/20 text-yellow-700 dark:text-yellow-300 hover:bg-yellow-500/30 transition-colors"
								aria-label="Reject step"
							>
								<X className="size-3" />
								Reject
							</button>
						</>
					)}
					{isInteractiveRunning && (
						<button
							type="button"
							onClick={handleCompleteInteractive}
							className="flex items-center gap-1 px-2 py-0.5 text-xs rounded bg-green-500/20 text-green-700 dark:text-green-300 hover:bg-green-500/30 transition-colors"
							aria-label="Complete step"
						>
							<Check className="size-3" />
							Complete
						</button>
					)}
					{isRunning && (
						<button
							type="button"
							onClick={handleAbort}
							className="flex items-center gap-1 px-2 py-0.5 text-xs rounded bg-red-500/20 text-red-700 dark:text-red-300 hover:bg-red-500/30 transition-colors"
							aria-label="Stop workflow"
						>
							<Square className="size-3" />
							Stop
						</button>
					)}
				</div>
			</div>

			{/* Trace */}
			<div className="flex-1 overflow-auto min-h-0">
				<WorkflowTrace
					workflowState={workflowState}
					onSessionClick={onSessionClick}
				/>
			</div>
		</div>
	);
}

function StatusBadge({ state }: { state: string }) {
	const colors: Record<string, string> = {
		running: "bg-blue-500/20 text-blue-700 dark:text-blue-300",
		completed: "bg-green-500/20 text-green-700 dark:text-green-300",
		failed: "bg-red-500/20 text-red-700 dark:text-red-300",
		waiting_approval: "bg-yellow-500/20 text-yellow-700 dark:text-yellow-300",
		aborted: "bg-muted text-muted-foreground",
	};
	return (
		<span
			className={`px-1.5 py-0.5 rounded text-xs ${colors[state] ?? "bg-muted text-muted-foreground"}`}
		>
			{state}
		</span>
	);
}
