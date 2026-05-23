import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, History, Square, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useWorkflowConfig } from "@/hooks/useWorkflowConfig";
import { useWorkflowRunLog } from "@/hooks/useWorkflowRunLog";
import type { PermissionMode } from "@/types/session";
import type { WorkflowRunSummary, WorkflowState } from "@/types/workflow";
import { WorkflowStatusSummary } from "./WorkflowStatusSummary";
import { type WorkflowStepSelection, WorkflowTrace } from "./WorkflowTrace";

export type { WorkflowStepSelection } from "./WorkflowTrace";

interface WorkflowPanelProps {
	workflowState: WorkflowState | null;
	worktreePath: string;
	permissionMode?: PermissionMode;
	onSessionClick?: (selection: WorkflowStepSelection) => void;
	onCloseSession?: (sessionId: string) => void;
	/**
	 * spec issues-1023: Workflow panel 内で transcript を表示中の step session id。
	 * 親の WorkflowSidebarPanel が保持する選択状態を子の trace 表示に伝播する。
	 */
	selectedStepSessionId?: string | null;
	/**
	 * spec issues-1023: selectedStep は session を持たない step を含めた現在選択
	 * 状態（stepName + runIndex）を表す。trace 上で非 session step も選択可能。
	 */
	selectedStep?: { stepName: string; runIndex?: number } | null;
	/**
	 * spec issues-1023: 新規 run 起動 UI は本 issue のスコープ外（別 milestone）。
	 * Workflow 観測 mode（右パネル）からは kicker を表示しない。
	 */
	showNewWorkflowButton?: boolean;
}

export function WorkflowPanel({
	workflowState,
	worktreePath,
	permissionMode = "readonly",
	onSessionClick,
	onCloseSession,
	selectedStepSessionId,
	selectedStep,
	showNewWorkflowButton = true,
}: WorkflowPanelProps) {
	const [executionIds, setExecutionIds] = useState<string[]>([]);
	const [openPastIds, setOpenPastIds] = useState<string[]>([]);
	const [activeTab, setActiveTab] = useState<string>("current");
	const [historyOpen, setHistoryOpen] = useState(false);

	const fetchExecutionIds = useCallback(() => {
		invoke<WorkflowRunSummary[]>("list_workflow_runs", {
			worktreePath,
		})
			.then((runs: WorkflowRunSummary[]) =>
				setExecutionIds(runs.map((r) => r.runId)),
			)
			.catch((e) =>
				console.warn("[WorkflowPanel] list workflow run summaries failed", e),
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
				{showNewWorkflowButton && (
					<NewWorkflowButton
						worktreePath={worktreePath}
						permissionMode={permissionMode}
					/>
				)}
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
						onCloseSession={onCloseSession}
						selectedStepSessionId={selectedStepSessionId}
						selectedStep={selectedStep}
					/>
				</TabsContent>
			)}

			{visiblePastIds.map((id) => (
				<TabsContent
					key={`${worktreePath}:${id}`}
					value={id}
					className="flex-1 min-h-0 mt-0"
				>
					<ExecutionView
						executionId={id}
						worktreePath={worktreePath}
						onSessionClick={onSessionClick}
						onCloseSession={onCloseSession}
						selectedStepSessionId={selectedStepSessionId}
						selectedStep={selectedStep}
					/>
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

function NewWorkflowButton({
	worktreePath,
	permissionMode,
}: {
	worktreePath: string;
	permissionMode: PermissionMode;
}) {
	const [open, setOpen] = useState(false);
	const [selectedWorkflow, setSelectedWorkflow] = useState<string | null>(null);
	const [taskInput, setTaskInput] = useState("");
	const [isPending, setIsPending] = useState(false);
	const { workflows } = useWorkflowConfig(open);

	const handleSelect = useCallback((workflowName: string) => {
		setSelectedWorkflow(workflowName);
		setTaskInput("");
	}, []);

	const handleStart = useCallback(() => {
		if (!selectedWorkflow || isPending) return;
		setIsPending(true);
		invoke("start_workflow", {
			workflowName: selectedWorkflow,
			worktreePath,
			task: taskInput.trim() || null,
			permissionMode,
		})
			.then(() => {
				setOpen(false);
				setSelectedWorkflow(null);
				setTaskInput("");
			})
			.catch((e) => console.warn("[WorkflowPanel] start_workflow failed", e))
			.finally(() => setIsPending(false));
	}, [selectedWorkflow, worktreePath, taskInput, permissionMode, isPending]);

	const handleBack = useCallback(() => {
		setSelectedWorkflow(null);
		setTaskInput("");
	}, []);

	const handleOpenChange = useCallback((isOpen: boolean) => {
		setOpen(isOpen);
		if (!isOpen) {
			setSelectedWorkflow(null);
			setTaskInput("");
		}
	}, []);

	return (
		<Popover open={open} onOpenChange={handleOpenChange}>
			<PopoverTrigger asChild>
				<button
					type="button"
					aria-label="New workflow"
					className="px-2 h-full text-sm text-muted-foreground hover:text-foreground transition-colors shrink-0"
				>
					+
				</button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-64 p-0">
				{selectedWorkflow ? (
					<div className="p-3 flex flex-col gap-2">
						<div className="flex items-center gap-2">
							<button
								type="button"
								onClick={handleBack}
								className="text-xs text-muted-foreground hover:text-foreground transition-colors"
								aria-label="Back to workflow list"
							>
								←
							</button>
							<span className="text-sm font-medium truncate">
								{selectedWorkflow}
							</span>
						</div>
						<textarea
							value={taskInput}
							onChange={(e) => setTaskInput(e.target.value)}
							placeholder="Task description (optional)"
							className="w-full min-h-[60px] max-h-[120px] rounded border bg-background px-2 py-1.5 text-sm resize-y focus:outline-none focus:ring-1 focus:ring-ring"
							onKeyDown={(e) => {
								if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
									handleStart();
								}
							}}
						/>
						<button
							type="button"
							onClick={handleStart}
							disabled={isPending}
							className="w-full px-3 py-1.5 text-sm rounded bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
						>
							{isPending ? "Starting..." : "Start"}
						</button>
					</div>
				) : workflows.length > 0 ? (
					<ul>
						{workflows.map((wf) => (
							<li key={wf.name}>
								<button
									type="button"
									className="w-full text-left px-3 py-2 text-sm hover:bg-muted transition-colors"
									onClick={() => handleSelect(wf.name)}
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
	worktreePath,
	onSessionClick,
	onCloseSession,
	selectedStepSessionId,
	selectedStep,
}: {
	executionId: string;
	worktreePath: string;
	onSessionClick?: (selection: WorkflowStepSelection) => void;
	onCloseSession?: (sessionId: string) => void;
	selectedStepSessionId?: string | null;
	selectedStep?: { stepName: string; runIndex?: number } | null;
}) {
	const [historyState, setHistoryState] = useState<WorkflowState | null>(null);
	const [isLoading, setIsLoading] = useState(true);
	const [loadError, setLoadError] = useState<string | null>(null);
	const { events, error: logError } = useWorkflowRunLog(
		worktreePath,
		executionId,
		executionId,
	);

	useEffect(() => {
		let cancelled = false;
		setHistoryState(null);
		setLoadError(null);
		setIsLoading(true);

		invoke<WorkflowState | null>("get_workflow_run_state", {
			worktreePath,
			runId: executionId,
		})
			.then((state) => {
				if (cancelled) return;
				if (!state) {
					setLoadError("Failed to load execution history.");
					return;
				}
				setHistoryState(state);
			})
			.catch((e) => {
				console.warn("[ExecutionView] get workflow execution data failed", e);
				if (!cancelled) {
					setLoadError("Failed to load execution history.");
				}
			})
			.finally(() => {
				if (!cancelled) {
					setIsLoading(false);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [executionId, worktreePath]);

	// spec issues-1023: event log fetch エラーも history 読み込み失敗として扱う。
	useEffect(() => {
		if (logError) setLoadError("Failed to load execution history.");
	}, [logError]);

	if (loadError) {
		return (
			<div className="flex h-full flex-col overflow-hidden">
				<div className="min-h-0 flex-1 overflow-auto p-3">
					<div
						role="alert"
						data-testid="workflow-history-error"
						className="rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-700 dark:text-red-300"
					>
						{loadError}
					</div>
				</div>
			</div>
		);
	}

	if (isLoading || !historyState) {
		return (
			<div className="flex items-center justify-center h-full text-sm text-muted-foreground">
				Loading...
			</div>
		);
	}

	return (
		<div className="flex flex-col h-full overflow-hidden">
			{/* Header */}
			<div className="flex shrink-0 flex-col gap-2 border-b px-3 py-2">
				<div className="flex min-w-0 items-center gap-2">
					<span className="text-sm font-medium">
						{historyState.workflowName}
					</span>
					<StatusBadge state={historyState.state.type} />
				</div>
				<WorkflowStatusSummary workflowState={historyState} />
			</div>

			{/* Trace */}
			<div className="flex-1 min-h-0">
				<WorkflowTrace
					key={`history:${worktreePath}:${executionId}`}
					workflowState={historyState}
					events={events}
					onSessionClick={onSessionClick}
					onCloseSession={onCloseSession}
					selectedStepSessionId={selectedStepSessionId}
					selectedStep={selectedStep}
				/>
			</div>
		</div>
	);
}

function WorkflowActivePanel({
	workflowState,
	worktreePath,
	onSessionClick,
	onCloseSession,
	selectedStepSessionId,
	selectedStep,
}: {
	workflowState: WorkflowState;
	worktreePath: string;
	onSessionClick?: (selection: WorkflowStepSelection) => void;
	onCloseSession?: (sessionId: string) => void;
	selectedStepSessionId?: string | null;
	selectedStep?: { stepName: string; runIndex?: number } | null;
}) {
	const isRunning =
		workflowState.state.type === "running" ||
		workflowState.state.type === "waiting_approval";

	const [abortError, setAbortError] = useState<string | null>(null);

	const abortIdentityKey = `${worktreePath}|${workflowState.executionId}|${workflowState.state.type}`;
	const [prevAbortIdentityKey, setPrevAbortIdentityKey] =
		useState(abortIdentityKey);
	if (prevAbortIdentityKey !== abortIdentityKey) {
		setPrevAbortIdentityKey(abortIdentityKey);
		setAbortError(null);
	}

	// spec issues-1023: active run でも event timeline を表示するため、run_id を
	// 主語に event log を取得する。updatedAt を refresh trigger にし、event 列の
	// 追加に追従する。worktree 認可境界を観測 invoke で必ず通すため worktreePath も
	// 明示する。
	const { events: activeEvents } = useWorkflowRunLog(
		worktreePath,
		workflowState.executionId,
		workflowState.updatedAt,
	);

	const handleAbort = useCallback(() => {
		invoke("abort_workflow", { runId: workflowState.executionId })
			.then(() => setAbortError(null))
			.catch((e) => {
				console.warn("[WorkflowPanel] abort_workflow failed", e);
				setAbortError(String(e));
			});
	}, [workflowState.executionId]);

	return (
		<div className="flex flex-col h-full overflow-hidden">
			<div className="shrink-0 border-b p-3">
				<WorkflowStatusSummary workflowState={workflowState} />
			</div>
			{abortError && (
				<div
					role="alert"
					className="flex items-start gap-2 px-3 py-2 border-b bg-red-500/10 text-red-700 dark:text-red-300 text-xs shrink-0"
				>
					<AlertTriangle className="size-3.5 mt-0.5 shrink-0" />
					<span>{abortError}</span>
				</div>
			)}
			{/* Action bar */}
			<div className="flex items-center justify-end px-3 py-1.5 border-b shrink-0">
				<div className="flex items-center gap-2">
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
			<div className="flex-1 min-h-0">
				<WorkflowTrace
					key={`current:${worktreePath}:${workflowState.executionId}`}
					workflowState={workflowState}
					events={activeEvents}
					onSessionClick={onSessionClick}
					onCloseSession={onCloseSession}
					selectedStepSessionId={selectedStepSessionId}
					selectedStep={selectedStep}
					approvalAction={{
						worktreePath,
						executionId: workflowState.executionId,
					}}
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
