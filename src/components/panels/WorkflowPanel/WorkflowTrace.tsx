import {
	AlertTriangle,
	Ban,
	Check,
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	Circle,
	Clock,
	Eye,
	EyeOff,
	GitBranch,
	Loader2,
	X,
} from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { useStepApprovalAction } from "@/hooks/useStepApprovalAction";
import type {
	CliMutationRequestRecord,
	JsonValue,
	NodeDefinition,
	ParallelStepState,
	StepHistoryEntry,
	WorkflowEvent,
	WorkflowState,
} from "@/types/workflow";
import { workflowStateClasses } from "./workflowStateStyles";

/**
 * spec issues-1023: timeline 上で step を選択した時に親へ伝える metadata。
 * `WorkflowState` の内部形状を再走査せずに済むよう、Trace 側が知っている
 * `stepName` / `nodeType` / `runIndex` / `runId`（observation 経路を Rust 側
 * StepDetailView 取得に閉じるためのキー）を一緒に渡す。
 *
 * `sessionId` は step に chat session が紐づいている場合のみ設定される。
 * agent step は当該 step の chat session、approval step は被承認の agent step
 * の chat session（engine の current_session_id）が引き継がれ、いずれも
 * Workflow panel 内 ChatSessionView から閲覧・対話できる。
 * sessionId が無い step（bash / parallel parent / 未紐付け completed）も
 * timeline 上から選択可能で、その場合は detail のみが表示される。
 */
export interface WorkflowStepSelection {
	runId: string;
	sessionId?: string;
	stepName: string;
	nodeType: "agent" | "bash" | "approval" | "parallel" | "unknown";
	runIndex?: number;
}

interface WorkflowTraceProps {
	workflowState: WorkflowState;
	events?: WorkflowEvent[];
	/**
	 * step session の選択操作。
	 * spec issues-1023 以降は「Workflow panel 内で transcript を読む step を選ぶ」
	 * 経路として用いる（旧 chat tab を開く責務は本コンポーネントから取り除かれている）。
	 */
	onSessionClick?: (selection: WorkflowStepSelection) => void;
	/** 選択中 step の transcript 表示を閉じる経路（旧 chat tab を閉じる相当） */
	onCloseSession?: (sessionId: string) => void;
	/**
	 * Workflow panel 内で transcript を表示中の step session id。
	 * 指定された場合、Eye 切替の "open" 表示はこちらを正規の状態とする
	 * （旧 `runtimeStates[sessionId].tabOpen` の chat-tab ベース判定を上書き）。
	 */
	selectedStepSessionId?: string | null;
	/**
	 * spec issues-1023: timeline 上のどの step（runIndex 込み）が現在選択中か。
	 * sessionId を持たない step（bash / approval / parallel parent / sessionId 無し
	 * completed）でも timeline から選択 → detail 表示できるよう、selection キーは
	 * stepName + runIndex を持つ。
	 */
	selectedStep?: { stepName: string; runIndex?: number } | null;
	approvalAction?: WorkflowApprovalActionContext;
}

interface WorkflowApprovalActionContext {
	worktreePath: string;
	executionId: string;
}

export function WorkflowTrace({
	workflowState,
	events = [],
	onSessionClick,
	onCloseSession,
	selectedStepSessionId,
	selectedStep,
	approvalAction,
}: WorkflowTraceProps) {
	const traceItems = buildTraceItems(workflowState, selectedStepSessionId);
	const autoFollowVersion = buildAutoFollowVersion(workflowState, events);
	const { scrollRef, handleScroll } = useAutoFollowScroll(autoFollowVersion);
	const runId = workflowState.executionId;

	return (
		<div
			ref={scrollRef}
			onScroll={handleScroll}
			data-testid="workflow-trace-scroll"
			className="h-full overflow-auto"
		>
			<div className="flex flex-col gap-3 p-3">
				<div className="flex flex-col">
					{traceItems.map((item, index) => (
						<TraceItemRow
							key={traceItemKey(item, index)}
							item={item}
							isLast={index === traceItems.length - 1}
							workflowState={workflowState}
							runId={runId}
							onSessionClick={onSessionClick}
							onCloseSession={onCloseSession}
							selectedStep={selectedStep ?? null}
							approvalAction={approvalAction}
						/>
					))}
				</div>
				{events.length > 0 && <EventTrace events={events} />}
			</div>
		</div>
	);
}

interface ScrollMetrics {
	scrollHeight: number;
	scrollTop: number;
	clientHeight: number;
}

const bottomThresholdPx = 24;

export function isAtBottom(
	metrics: ScrollMetrics,
	threshold = bottomThresholdPx,
) {
	return (
		metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= threshold
	);
}

function readScrollMetrics(element: HTMLElement): ScrollMetrics {
	return {
		scrollHeight: element.scrollHeight,
		scrollTop: element.scrollTop,
		clientHeight: element.clientHeight,
	};
}

function buildAutoFollowVersion(
	workflowState: WorkflowState,
	events: WorkflowEvent[],
) {
	const activeParallelVersion =
		workflowState.activeParallelSteps
			?.map(
				(step) =>
					`${step.stepName}:${step.state}:${step.runIndex}:${step.completedAt ?? ""}`,
			)
			.join("|") ?? "";
	const historyVersion = workflowState.stepHistory
		.map((entry) => `${entry.stepName}:${entry.completedAt}`)
		.join("|");
	const eventVersion = events
		.map((event) => `${event.event}:${event.timestampMs}`)
		.join("|");

	return [
		workflowState.executionId,
		workflowState.updatedAt,
		workflowState.state.type,
		workflowState.currentStepName,
		historyVersion,
		activeParallelVersion,
		eventVersion,
	].join(";");
}

function useAutoFollowScroll(autoFollowVersion: string) {
	const scrollRef = useRef<HTMLDivElement | null>(null);
	const [isAutoFollowing, setIsAutoFollowing] = useState(true);

	const handleScroll = () => {
		const element = scrollRef.current;
		if (!element) return;
		setIsAutoFollowing(isAtBottom(readScrollMetrics(element)));
	};

	useLayoutEffect(() => {
		if (!isAutoFollowing) return;
		const element = scrollRef.current;
		if (!element) return;
		// Consume the content version so this effect reruns after trace updates.
		void autoFollowVersion;
		element.scrollTop = element.scrollHeight;
	}, [isAutoFollowing, autoFollowVersion]);

	return { scrollRef, handleScroll };
}

type TraceItem =
	| {
			kind: "completed";
			step: NodeDefinition | undefined;
			stepName: string;
			occurrence: number;
			entry: StepHistoryEntry;
			sessionId?: string;
			runtimeActive: boolean;
			tabOpen: boolean;
			state: "completed";
	  }
	| {
			kind: "current";
			step: NodeDefinition | undefined;
			stepName: string;
			occurrence: number;
			sessionId?: string;
			runtimeActive?: boolean;
			tabOpen?: boolean;
			state: "running" | "waiting_approval" | "failed";
	  }
	| {
			kind: "parallel";
			step: NodeDefinition | undefined;
			stepName: string;
			occurrence: number;
			childSteps: TraceParallelStepState[];
			state: "running" | "completed" | "failed";
			entry?: StepHistoryEntry;
	  };

type TraceParallelStepState = ParallelStepState & {
	runtimeActive: boolean;
	tabOpen: boolean;
};

function runtimeFor(
	workflowState: WorkflowState,
	sessionId?: string,
	selectedStepSessionId?: string | null,
) {
	if (!sessionId) return { runtimeActive: false, tabOpen: false };
	const base = workflowState.runtimeStates?.[sessionId] ?? {
		runtimeActive: false,
		tabOpen: false,
	};
	// spec issues-1023: Workflow panel 内 transcript 経路に移行後は、selected step
	// が "open" の真実源（chat tab open は廃止）。selectedStepSessionId が
	// undefined のときのみ旧来の runtimeStates 由来の表示にフォールバックする。
	if (selectedStepSessionId !== undefined) {
		return {
			runtimeActive: base.runtimeActive,
			tabOpen: selectedStepSessionId === sessionId,
		};
	}
	return base;
}

function buildTraceItems(
	workflowState: WorkflowState,
	selectedStepSessionId?: string | null,
): TraceItem[] {
	const stepsByName = new Map(
		workflowState.workflowDefinition.nodes.map((step) => [step.name, step]),
	);
	const seenCounts = new Map<string, number>();
	const items: TraceItem[] = workflowState.stepHistory.map((entry) => {
		const occurrence = (seenCounts.get(entry.stepName) ?? 0) + 1;
		seenCounts.set(entry.stepName, occurrence);
		const step = stepsByName.get(entry.stepName);

		if (step?.parallel_children && step.parallel_children.length > 0) {
			const childSteps: TraceParallelStepState[] = step.parallel_children.map(
				(child) => {
					// 履歴エントリにchild snapshotがあればそれを使用（run固有の情報）
					const childSnapshot = entry.childOutputs?.find(
						(co) => co.stepName === child.name,
					);
					if (childSnapshot) {
						const runtime = runtimeFor(
							workflowState,
							childSnapshot.sessionId,
							selectedStepSessionId,
						);
						return {
							stepName: child.name,
							state: "completed" as const,
							sessionId: childSnapshot.sessionId,
							result: childSnapshot.result,
							runIndex: childSnapshot.runIndex,
							completedAt: childSnapshot.completedAt,
							structuredOutput: childSnapshot.structuredOutput,
							outputContract: childSnapshot.outputContract,
							runtimeActive: runtime.runtimeActive,
							tabOpen: runtime.tabOpen,
						};
					}
					// フォールバック: グローバルstepOutputsを参照
					const childOutput = workflowState.stepOutputs[child.name];
					const runtime = runtimeFor(
						workflowState,
						childOutput?.sessionId,
						selectedStepSessionId,
					);
					return {
						stepName: child.name,
						state: childOutput ? "completed" : "pending",
						sessionId: childOutput?.sessionId,
						result: childOutput?.result,
						runIndex: childOutput?.runIndex ?? 1,
						completedAt: childOutput?.completedAt,
						structuredOutput: childOutput?.structuredOutput,
						outputContract: childOutput?.outputContract,
						runtimeActive: runtime.runtimeActive,
						tabOpen: runtime.tabOpen,
					};
				},
			);
			return {
				kind: "parallel" as const,
				step,
				stepName: entry.stepName,
				occurrence,
				childSteps,
				state: "completed" as const,
				entry,
			};
		}

		const runtime = runtimeFor(
			workflowState,
			entry.sessionId,
			selectedStepSessionId,
		);
		return {
			kind: "completed",
			step,
			stepName: entry.stepName,
			occurrence,
			entry,
			sessionId: entry.sessionId,
			runtimeActive: runtime.runtimeActive,
			tabOpen: runtime.tabOpen,
			state: "completed",
		};
	});

	const state = workflowState.state.type;
	// ワークフロー失敗時、current stepが既にstep_historyに完了記録があれば
	// ghostエントリを追加しない（cycle_guard超過等のワークフローレベル失敗）
	const currentAlreadyCompleted =
		state === "failed" &&
		workflowState.currentStepName &&
		seenCounts.has(workflowState.currentStepName);
	if (
		(state === "running" ||
			state === "waiting_approval" ||
			(state === "failed" && !currentAlreadyCompleted)) &&
		workflowState.currentStepName
	) {
		const completedCount = seenCounts.get(workflowState.currentStepName) ?? 0;
		const startedCount =
			workflowState.stepExecutionCounts[workflowState.currentStepName] ??
			completedCount + 1;
		const currentStep = stepsByName.get(workflowState.currentStepName);
		const activeParallel = workflowState.activeParallelSteps;
		if (
			currentStep?.parallel_children &&
			activeParallel &&
			activeParallel.length > 0
		) {
			const childSteps: TraceParallelStepState[] = activeParallel.map(
				(step) => {
					const runtime = runtimeFor(
						workflowState,
						step.sessionId,
						selectedStepSessionId,
					);
					return {
						...step,
						runtimeActive: runtime.runtimeActive,
						tabOpen: runtime.tabOpen,
					};
				},
			);
			const hasFailed = activeParallel.some((ps) => ps.state === "failed");
			const allFinished = activeParallel.every(
				(ps) => ps.state === "completed" || ps.state === "failed",
			);
			items.push({
				kind: "parallel",
				step: currentStep,
				stepName: workflowState.currentStepName,
				occurrence: Math.max(startedCount, completedCount + 1),
				childSteps,
				state: hasFailed ? "failed" : allFinished ? "completed" : "running",
			});
		} else {
			const runtime = runtimeFor(
				workflowState,
				workflowState.currentSessionId,
				selectedStepSessionId,
			);
			items.push({
				kind: "current",
				step: currentStep,
				stepName: workflowState.currentStepName,
				occurrence: Math.max(startedCount, completedCount + 1),
				sessionId: workflowState.currentSessionId,
				runtimeActive: runtime.runtimeActive,
				tabOpen: runtime.tabOpen,
				state,
			});
		}
	}

	return items;
}

function traceItemKey(item: TraceItem, index: number) {
	if (item.kind === "completed") {
		return `${item.stepName}-${item.occurrence}-${item.entry.completedAt}-${item.entry.sessionId ?? item.entry.result ?? "done"}`;
	}
	if (item.kind === "parallel") {
		return `parallel-${item.stepName}-${item.occurrence}-${item.state}`;
	}
	return `${item.stepName}-${item.occurrence}-${item.state}-${index}`;
}

function TraceItemRow({
	item,
	isLast,
	workflowState,
	runId,
	onSessionClick,
	onCloseSession,
	selectedStep,
	approvalAction,
}: {
	item: TraceItem;
	isLast: boolean;
	workflowState: WorkflowState;
	runId: string;
	onSessionClick?: (selection: WorkflowStepSelection) => void;
	onCloseSession?: (sessionId: string) => void;
	selectedStep: { stepName: string; runIndex?: number } | null;
	approvalAction?: WorkflowApprovalActionContext;
}) {
	if (item.kind === "parallel") {
		return (
			<ParallelBlockRow
				item={item}
				isLast={isLast}
				runId={runId}
				onSessionClick={onSessionClick}
				onCloseSession={onCloseSession}
				selectedStep={selectedStep}
			/>
		);
	}

	const stepMode = item.step?.type ?? "unknown";
	const approvalTarget =
		workflowState.state.type === "waiting_approval" &&
		item.kind === "current" &&
		!item.step?.parallel_children &&
		item.stepName === workflowState.currentStepName
			? approvalAction
			: undefined;
	const canReject = workflowState.approvalOperations?.canReject === true;
	const runIndex =
		item.kind === "completed" ? item.entry.runIndex : item.occurrence;
	const nodeType: WorkflowStepSelection["nodeType"] =
		item.step?.type ?? "unknown";
	const handleSelectRow = onSessionClick
		? () =>
				onSessionClick({
					runId,
					sessionId: item.sessionId,
					stepName: item.stepName,
					nodeType,
					runIndex,
				})
		: undefined;
	const isSelected =
		selectedStep != null &&
		selectedStep.stepName === item.stepName &&
		(selectedStep.runIndex === undefined || selectedStep.runIndex === runIndex);

	return (
		<div className="grid grid-cols-[24px_1fr] gap-2">
			<div className="flex flex-col items-center">
				<div
					className={`mt-2 flex size-5 items-center justify-center rounded-full border ${workflowStateClasses[item.state] ?? workflowStateClasses.pending}`}
				>
					<StateIcon state={item.state} />
				</div>
				{!isLast && <div className="w-px flex-1 min-h-4 bg-border" />}
			</div>
			<div
				data-testid={`trace-item-${item.stepName}-${item.occurrence}`}
				className={`mb-2 overflow-hidden rounded-md border px-3 py-2 transition-colors ${
					item.kind === "current"
						? "border-primary/60 bg-primary/5"
						: isSelected
							? "border-primary/40 bg-primary/5"
							: "border-border hover:bg-muted/30"
				}`}
			>
				{handleSelectRow ? (
					<button
						type="button"
						data-testid={`select-step-${item.stepName}-${item.occurrence}`}
						aria-label={`Select step ${item.stepName}`}
						onClick={handleSelectRow}
						className="flex w-full min-w-0 flex-wrap items-center gap-2 text-left"
					>
						<span className="min-w-0 flex-1 text-sm font-medium truncate">
							{item.stepName}
						</span>
						<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
							{stepMode}
						</span>
					</button>
				) : (
					<div className="flex min-w-0 flex-wrap items-center gap-2">
						<span className="min-w-0 flex-1 text-sm font-medium truncate">
							{item.stepName}
						</span>
						<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
							{stepMode}
						</span>
					</div>
				)}
				<TraceItemSummary
					item={item}
					runId={runId}
					onSessionClick={onSessionClick}
					onCloseSession={onCloseSession}
				/>
				{approvalTarget && (
					<StepApprovalActions
						approvalAction={approvalTarget}
						stepName={item.stepName}
						canReject={canReject}
					/>
				)}
			</div>
		</div>
	);
}

function StepApprovalActions({
	approvalAction,
	stepName,
	canReject,
}: {
	approvalAction: WorkflowApprovalActionContext;
	stepName: string;
	canReject: boolean;
}) {
	const {
		rejectMode,
		rejectComment,
		setRejectComment,
		canSubmitReject,
		approvalError,
		approve,
		openReject,
		cancelReject,
		submitReject,
	} = useStepApprovalAction({
		worktreePath: approvalAction.worktreePath,
		executionId: approvalAction.executionId,
		stepName,
	});

	return (
		<div className="mt-2 flex min-w-0 flex-col gap-1.5 border-t pt-2">
			{approvalError && (
				<div
					role="alert"
					className="flex items-start gap-2 rounded border border-red-500/30 bg-red-500/10 px-2 py-1.5 text-xs text-red-700 dark:text-red-300"
				>
					<AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
					<span className="min-w-0 break-words">{approvalError}</span>
				</div>
			)}
			{rejectMode ? (
				<div className="flex min-w-0 flex-col gap-1.5">
					<textarea
						className="w-full min-w-0 rounded border bg-background px-2 py-1 text-xs resize-none"
						rows={3}
						placeholder="Reject comment..."
						value={rejectComment}
						onChange={(e) => setRejectComment(e.target.value)}
						aria-label="Reject comment"
					/>
					<div className="flex items-center justify-end gap-2">
						<button
							type="button"
							onClick={cancelReject}
							className="rounded bg-muted px-2 py-0.5 text-xs transition-colors hover:bg-muted/80"
						>
							Cancel
						</button>
						<button
							type="button"
							onClick={submitReject}
							disabled={!canSubmitReject}
							className="rounded bg-yellow-500/20 px-2 py-0.5 text-xs text-yellow-700 transition-colors hover:bg-yellow-500/30 disabled:cursor-not-allowed disabled:opacity-50 dark:text-yellow-300"
							aria-label="Submit reject"
						>
							Reject
						</button>
					</div>
				</div>
			) : (
				<div className="flex flex-wrap items-center justify-end gap-2">
					<button
						type="button"
						onClick={approve}
						className="flex items-center gap-1 rounded bg-green-500/20 px-2 py-0.5 text-xs text-green-700 transition-colors hover:bg-green-500/30 dark:text-green-300"
						aria-label="Approve step"
					>
						<Check className="size-3" />
						Approve
					</button>
					{canReject && (
						<button
							type="button"
							onClick={openReject}
							className="flex items-center gap-1 rounded bg-yellow-500/20 px-2 py-0.5 text-xs text-yellow-700 transition-colors hover:bg-yellow-500/30 dark:text-yellow-300"
							aria-label="Reject step"
						>
							<X className="size-3" />
							Reject
						</button>
					)}
				</div>
			)}
		</div>
	);
}

function ParallelBlockRow({
	item,
	isLast,
	runId,
	onSessionClick,
	onCloseSession,
	selectedStep,
}: {
	item: Extract<TraceItem, { kind: "parallel" }>;
	isLast: boolean;
	runId: string;
	onSessionClick?: (selection: WorkflowStepSelection) => void;
	onCloseSession?: (sessionId: string) => void;
	selectedStep: { stepName: string; runIndex?: number } | null;
}) {
	const completedCount = item.childSteps.filter(
		(cs) => cs.state === "completed",
	).length;
	const totalCount = item.childSteps.length;
	const tokenTotal = item.entry?.tokenUsage
		? item.entry.tokenUsage.inputTokens + item.entry.tokenUsage.outputTokens
		: null;

	// spec issues-1023: parallel parent header からも step detail を選択できるよう
	// onSessionClick / selectedStep を受け取る。parent 自体は session を持たないため
	// nodeType=parallel / sessionId 無し で WorkflowStepSelection を発火する。
	const isParentSelected =
		selectedStep != null &&
		selectedStep.stepName === item.stepName &&
		(selectedStep.runIndex === undefined ||
			selectedStep.runIndex === item.occurrence);
	const handleSelectParent = onSessionClick
		? () =>
				onSessionClick({
					runId,
					stepName: item.stepName,
					nodeType: "parallel",
					runIndex: item.occurrence,
				})
		: undefined;

	return (
		<div className="grid grid-cols-[24px_1fr] gap-2">
			<div className="flex flex-col items-center">
				<div
					className={`mt-2 flex size-5 items-center justify-center rounded-full border ${workflowStateClasses[item.state] ?? workflowStateClasses.pending}`}
				>
					<StateIcon state={item.state} />
				</div>
				{!isLast && <div className="w-px flex-1 min-h-4 bg-border" />}
			</div>
			<div
				data-testid={`trace-item-${item.stepName}-${item.occurrence}`}
				className={`mb-2 overflow-hidden rounded-md border px-3 py-2 ${
					item.state === "running"
						? "border-primary/60 bg-primary/5"
						: item.state === "failed"
							? "border-red-500/30 bg-red-500/5"
							: isParentSelected
								? "border-primary/40 bg-primary/5"
								: "border-border"
				}`}
			>
				{handleSelectParent ? (
					<button
						type="button"
						data-testid={`select-step-${item.stepName}-${item.occurrence}`}
						aria-label={`Select step ${item.stepName}`}
						onClick={handleSelectParent}
						className="flex w-full min-w-0 flex-wrap items-center gap-2 text-left"
					>
						<GitBranch className="size-3.5 text-muted-foreground" />
						<span className="min-w-0 flex-1 text-sm font-medium truncate">
							{item.stepName}
						</span>
						<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
							parallel
						</span>
					</button>
				) : (
					<div className="flex min-w-0 flex-wrap items-center gap-2">
						<GitBranch className="size-3.5 text-muted-foreground" />
						<span className="min-w-0 flex-1 text-sm font-medium truncate">
							{item.stepName}
						</span>
						<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
							parallel
						</span>
					</div>
				)}
				<div className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
					<span>
						{completedCount}/{totalCount} completed
					</span>
					{tokenTotal != null && (
						<span className="shrink-0">{tokenTotal} tokens</span>
					)}
				</div>
				<div className="mt-2 flex flex-col gap-1 pl-2 border-l-2 border-border">
					{item.childSteps.map((child) => {
						const childNodeType =
							item.step?.parallel_children?.find(
								(c) => c.name === child.stepName,
							)?.type ?? "agent";
						return (
							<ParallelChildRow
								key={`${child.stepName}-${child.runIndex}`}
								child={child}
								nodeType={childNodeType}
								runId={runId}
								onSessionClick={onSessionClick}
								onCloseSession={onCloseSession}
							/>
						);
					})}
				</div>
				{item.entry?.structuredOutput && (
					<div className="mt-2">
						<StructuredOutputToggle output={item.entry.structuredOutput} />
					</div>
				)}
			</div>
		</div>
	);
}

function ParallelChildRow({
	child,
	nodeType,
	runId,
	onSessionClick,
	onCloseSession,
}: {
	child: TraceParallelStepState;
	nodeType: WorkflowStepSelection["nodeType"];
	runId: string;
	onSessionClick?: (selection: WorkflowStepSelection) => void;
	onCloseSession?: (sessionId: string) => void;
}) {
	const so = child.structuredOutput;
	const verdict =
		so != null && typeof so === "object" && !Array.isArray(so)
			? ((so as Record<string, unknown>).verdict as string | undefined)
			: undefined;
	const sessionId = child.sessionId;
	const handleSelect = onSessionClick
		? () =>
				onSessionClick({
					runId,
					sessionId,
					stepName: child.stepName,
					nodeType,
					runIndex: child.runIndex,
				})
		: undefined;
	return (
		<div
			data-testid={`trace-child-item-${child.stepName}-${child.runIndex + 1}`}
			className="flex min-w-0 flex-col gap-1 rounded px-2 py-1 text-xs"
		>
			<div className="flex items-center gap-2">
				<div
					className={`flex size-4 items-center justify-center rounded-full border ${workflowStateClasses[child.state] ?? workflowStateClasses.pending}`}
				>
					<StateIcon state={child.state} />
				</div>
				{handleSelect ? (
					<button
						type="button"
						aria-label={`Select step ${child.stepName}`}
						onClick={handleSelect}
						className="min-w-0 flex-1 truncate text-left font-medium"
					>
						{child.stepName}
					</button>
				) : (
					<span className="min-w-0 flex-1 truncate font-medium">
						{child.stepName}
					</span>
				)}
				{verdict && <VerdictBadge verdict={verdict} />}
				{!verdict && child.result && (
					<span className="rounded bg-muted px-1.5 py-0.5 text-muted-foreground">
						{child.result}
					</span>
				)}
				{sessionId && (
					<SessionToggleButton
						tabOpen={child.tabOpen}
						onOpen={
							onSessionClick
								? () =>
										onSessionClick({
											runId,
											sessionId,
											stepName: child.stepName,
											nodeType,
											runIndex: child.runIndex,
										})
								: undefined
						}
						onClose={
							onCloseSession ? () => onCloseSession(sessionId) : undefined
						}
					/>
				)}
			</div>
			{child.structuredOutput && (
				<div className="ml-6">
					<StructuredOutputToggle output={child.structuredOutput} />
				</div>
			)}
		</div>
	);
}

/**
 * step session の tab open/close をトグルする小さなアイコンボタン。
 * - tab_open=true  → `Eye`、クリックで close（タブを閉じる）
 * - tab_open=false → `EyeOff`、クリックで open（タブを開いて履歴/現在の会話を表示）
 *
 * step 実行中（runtime busy）でも閉じる操作は可能。バックエンドが
 * `is_agent_step_runtime_busy` を見て runtime は残し tab だけ閉じる扱いになる。
 */
function SessionToggleButton({
	tabOpen,
	onOpen,
	onClose,
}: {
	tabOpen?: boolean;
	onOpen?: () => void;
	onClose?: () => void;
}) {
	const isOpen = tabOpen === true;
	const handler = isOpen ? onClose : onOpen;
	if (!handler) return null;
	const Icon = isOpen ? Eye : EyeOff;
	const label = isOpen ? "Close tab" : "Open tab";
	return (
		<button
			type="button"
			aria-label={label}
			title={label}
			className="shrink-0 text-muted-foreground hover:text-foreground"
			onClick={() => handler()}
		>
			<Icon className="size-3.5" />
		</button>
	);
}

function StateIcon({ state }: { state: string }) {
	if (state === "running") return <Loader2 className="size-3 animate-spin" />;
	if (state === "completed") return <CheckCircle2 className="size-3" />;
	if (state === "failed") return <AlertTriangle className="size-3" />;
	if (state === "waiting_approval") return <Clock className="size-3" />;
	if (state === "aborted") return <Ban className="size-3" />;
	return <Circle className="size-2" />;
}

function TraceItemSummary({
	item,
	runId,
	onSessionClick,
	onCloseSession,
}: {
	item: Exclude<TraceItem, { kind: "parallel" }>;
	runId: string;
	onSessionClick?: (selection: WorkflowStepSelection) => void;
	onCloseSession?: (sessionId: string) => void;
}) {
	const nodeType: WorkflowStepSelection["nodeType"] =
		item.step?.type ?? "unknown";
	const runIndex =
		item.kind === "completed" ? item.entry.runIndex : item.occurrence;
	if (item.kind === "completed") {
		const tokenTotal = item.entry.tokenUsage
			? item.entry.tokenUsage.inputTokens + item.entry.tokenUsage.outputTokens
			: null;
		const structuredOutput = item.entry.structuredOutput;
		const hasSessionToggle =
			item.sessionId != null &&
			(onSessionClick != null || onCloseSession != null);
		const hasSummary =
			item.entry.result != null || tokenTotal != null || hasSessionToggle;
		const hasStructuredOutput = structuredOutput != null;

		if (!hasSummary && !hasStructuredOutput) {
			return null;
		}

		return (
			<div className="mt-1 space-y-1">
				{hasSummary && (
					<div className="flex items-center gap-2 text-xs text-muted-foreground">
						<div className="min-w-0 flex-1">
							{item.entry.result && (
								<VerdictBadge verdict={item.entry.result} />
							)}
						</div>
						{tokenTotal != null && (
							<span className="shrink-0">{tokenTotal} tokens</span>
						)}
						{item.sessionId && (
							<SessionToggleButton
								tabOpen={item.tabOpen}
								onOpen={
									onSessionClick
										? () =>
												onSessionClick({
													runId,
													sessionId: item.sessionId as string,
													stepName: item.stepName,
													nodeType,
													runIndex,
												})
										: undefined
								}
								onClose={
									onCloseSession
										? () => onCloseSession(item.sessionId as string)
										: undefined
								}
							/>
						)}
					</div>
				)}
				{hasStructuredOutput && (
					<StructuredOutputToggle output={structuredOutput} />
				)}
			</div>
		);
	}

	if (item.sessionId) {
		const sessionId = item.sessionId;
		return (
			<div className="mt-1 flex items-center gap-2 text-xs">
				<span
					className={`min-w-0 flex-1 truncate ${
						item.state === "waiting_approval"
							? "text-yellow-600"
							: item.state === "failed"
								? "text-red-600"
								: "text-blue-600"
					}`}
				>
					{item.state === "waiting_approval"
						? "Waiting for approval"
						: item.state === "failed"
							? "Failed"
							: "Running"}
				</span>
				<SessionToggleButton
					tabOpen={item.tabOpen}
					onOpen={
						onSessionClick
							? () =>
									onSessionClick({
										runId,
										sessionId,
										stepName: item.stepName,
										nodeType,
										runIndex,
									})
							: undefined
					}
					onClose={onCloseSession ? () => onCloseSession(sessionId) : undefined}
				/>
			</div>
		);
	}

	if (item.state === "running") {
		return (
			<div className="mt-1 flex items-center gap-2 text-xs text-blue-600">
				<span className="min-w-0 flex-1">Running</span>
			</div>
		);
	}
	if (item.state === "waiting_approval") {
		return (
			<div className="mt-1 flex items-center gap-2 text-xs text-yellow-600">
				<span className="min-w-0 flex-1">Waiting for approval</span>
			</div>
		);
	}
	if (item.state === "failed") {
		return (
			<div className="mt-1 flex items-center gap-2 text-xs text-red-600">
				<span className="min-w-0 flex-1">Failed</span>
			</div>
		);
	}
}

function StructuredOutputToggle({ output }: { output: JsonValue }) {
	const [expanded, setExpanded] = useState(false);
	const json = JSON.stringify(output, null, 2);

	return (
		<div className="text-xs">
			<button
				type="button"
				className="flex items-center gap-1 text-muted-foreground hover:text-foreground"
				onClick={() => setExpanded((v) => !v)}
			>
				{expanded ? (
					<ChevronDown className="size-3" />
				) : (
					<ChevronRight className="size-3" />
				)}
				Structured Output
			</button>
			{expanded && (
				<pre className="mt-1 max-h-40 overflow-auto rounded bg-muted p-2 text-xs whitespace-pre-wrap break-words">
					{json}
				</pre>
			)}
		</div>
	);
}

const verdictBadgeClasses: Record<string, string> = {
	LGTM: "bg-green-500/20 text-green-700 dark:text-green-300",
	NEEDS_FIX: "bg-red-500/20 text-red-700 dark:text-red-300",
	FIXED: "bg-green-500/20 text-green-700 dark:text-green-300",
	PARTIAL: "bg-yellow-500/20 text-yellow-700 dark:text-yellow-300",
	BLOCKED: "bg-red-500/20 text-red-700 dark:text-red-300",
	PASSED: "bg-green-500/20 text-green-700 dark:text-green-300",
	FAILED: "bg-red-500/20 text-red-700 dark:text-red-300",
};

function VerdictBadge({ verdict }: { verdict: string }) {
	const cls = verdictBadgeClasses[verdict] ?? "bg-muted text-muted-foreground";
	return (
		<span
			className={`shrink-0 rounded px-1.5 py-0.5 text-xs font-medium ${cls}`}
		>
			{verdict}
		</span>
	);
}

function EventTrace({ events }: { events: WorkflowEvent[] }) {
	return (
		<div className="rounded-md border">
			<div className="border-b px-3 py-1.5 text-xs font-medium text-muted-foreground">
				Event log
			</div>
			<div className="max-h-32 overflow-auto px-3 py-1">
				{events.map((event) => {
					const subject = describeEventSubject(event);
					return (
						<div
							key={`${event.event}-${subject ?? ""}-${event.timestampMs}`}
							className="flex items-baseline gap-2 py-0.5 text-xs text-muted-foreground"
						>
							<span
								className="shrink-0 font-mono tabular-nums opacity-70"
								title={new Date(event.timestampMs).toISOString()}
							>
								{formatEventTimestamp(event.timestampMs)}
							</span>
							<span className="min-w-0">
								<span className="font-mono">{event.event}</span>
								{subject && <span className="ml-1">({subject})</span>}
								{event.event === "contract_repair_requested" && (
									<span className="ml-1 text-yellow-600 dark:text-yellow-400">
										retry #{event.attempt}: {event.violation_reason}
									</span>
								)}
								{event.event === "cli_mutation_requested" && (
									<span className="ml-1 text-blue-600 dark:text-blue-400">
										{describeCliMutationRequest(event.request)}
									</span>
								)}
							</span>
						</div>
					);
				})}
			</div>
		</div>
	);
}

/**
 * spec issues-1023: event timeline 行で「どの node に紐づく事実か」を一目で観測
 * できるよう、event の種別に応じて主語 node の表示文字列を引き当てる。parallel 系
 * event は parent_node_name / child_node_name を併記する。
 */
function describeEventSubject(event: WorkflowEvent): string | undefined {
	switch (event.event) {
		case "parallel_started":
		case "parallel_completed":
			return event.parent_node_name;
		case "parallel_child_started":
		case "parallel_child_completed":
			return `${event.parent_node_name} → ${event.child_node_name}`;
		case "cli_mutation_requested":
			return event.request.node_name ?? undefined;
		default:
			return "node_name" in event ? event.node_name : undefined;
	}
}

/**
 * spec issues-1023 L13: cli_mutation_requested event の request payload
 * （approve / reject / abort + 自由記述）を観測者が経路非依存に読めるよう、
 * timeline 上に表示する短い summary 文字列を返す。
 */
function describeCliMutationRequest(request: CliMutationRequestRecord): string {
	switch (request.kind) {
		case "approve":
			return request.comment ? `approve: ${request.comment}` : "approve";
		case "reject":
			return `reject: ${request.reason}`;
		case "abort":
			return "abort";
	}
}

/**
 * spec issues-1023: event timeline 上で「いつ」事実が起きたかを観測するために
 * timestamp を `HH:MM:SS` 形式で表示する。秒未満は noisy なので削る。
 * 集約・整序は engine 側で済んでおり、本関数は表示用フォーマットのみを担う。
 */
function formatEventTimestamp(timestamp: number): string {
	const d = new Date(timestamp);
	if (Number.isNaN(d.getTime())) return "";
	const hh = String(d.getHours()).padStart(2, "0");
	const mm = String(d.getMinutes()).padStart(2, "0");
	const ss = String(d.getSeconds()).padStart(2, "0");
	return `${hh}:${mm}:${ss}`;
}
