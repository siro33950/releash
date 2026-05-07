import {
	AlertTriangle,
	Ban,
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	Circle,
	Clock,
	GitBranch,
	Loader2,
} from "lucide-react";
import { useState } from "react";
import type {
	JsonValue,
	ParallelStepState,
	Step,
	StepHistoryEntry,
	WorkflowLogEvent,
	WorkflowState,
} from "@/types/workflow";

interface WorkflowTraceProps {
	workflowState: WorkflowState;
	events?: WorkflowLogEvent[];
	onSessionClick?: (sessionId: string) => void;
	onFileClick?: (path: string) => void;
}

const stateClasses: Record<string, string> = {
	running: "border-blue-500/50 bg-blue-500/10 text-blue-700 dark:text-blue-300",
	completed:
		"border-green-500/50 bg-green-500/10 text-green-700 dark:text-green-300",
	failed: "border-red-500/50 bg-red-500/10 text-red-700 dark:text-red-300",
	waiting_approval:
		"border-yellow-500/50 bg-yellow-500/10 text-yellow-700 dark:text-yellow-300",
	aborted: "border-muted-foreground/40 bg-muted text-muted-foreground",
	pending: "border-border bg-background text-muted-foreground",
};

export function WorkflowTrace({
	workflowState,
	events = [],
	onSessionClick,
	onFileClick,
}: WorkflowTraceProps) {
	const totalTokens =
		workflowState.totalTokenUsage.inputTokens +
		workflowState.totalTokenUsage.outputTokens;
	const traceItems = buildTraceItems(workflowState);

	return (
		<div className="h-full overflow-auto">
			<div className="flex flex-col gap-3 p-3">
				<CurrentAction
					workflowState={workflowState}
					totalTokens={totalTokens}
				/>
				<div className="flex flex-col">
					{traceItems.map((item, index) => (
						<TraceItemRow
							key={traceItemKey(item, index)}
							item={item}
							index={index}
							isLast={index === traceItems.length - 1}
							onSessionClick={onSessionClick}
							onFileClick={onFileClick}
						/>
					))}
				</div>
				{events.length > 0 && <EventTrace events={events} />}
			</div>
		</div>
	);
}

function CurrentAction({
	workflowState,
	totalTokens,
}: {
	workflowState: WorkflowState;
	totalTokens: number;
}) {
	const state = workflowState.state.type;
	const currentStep = workflowState.workflowDefinition.steps.find(
		(step) => step.name === workflowState.currentStepName,
	);

	let label = "Workflow completed";
	if (state === "running") label = `Running ${workflowState.currentStepName}`;
	if (state === "waiting_approval") {
		label = `Waiting for approval: ${workflowState.currentStepName}`;
	}
	if (state === "failed") label = "Workflow failed";
	if (state === "aborted") label = "Workflow aborted";

	const details =
		state === "failed" && "reason" in workflowState.state
			? workflowState.state.reason
			: currentStep
				? `${currentStep.parallel ? "parallel" : (currentStep.mode ?? "auto")} step`
				: `${workflowState.stepHistory.length} recorded steps`;

	return (
		<div
			className={`overflow-hidden rounded-md border px-3 py-2 ${stateClasses[state] ?? stateClasses.pending}`}
		>
			<div className="flex items-center justify-between gap-3">
				<div className="min-w-0">
					<div className="text-sm font-medium truncate">{label}</div>
					<div className="text-xs opacity-80 truncate">{details}</div>
				</div>
				<div className="text-xs whitespace-nowrap opacity-80">
					{totalTokens} tokens
				</div>
			</div>
		</div>
	);
}

type TraceItem =
	| {
			kind: "completed";
			step: Step | undefined;
			stepName: string;
			occurrence: number;
			entry: StepHistoryEntry;
			sessionId?: string;
			state: "completed";
	  }
	| {
			kind: "current";
			step: Step | undefined;
			stepName: string;
			occurrence: number;
			sessionId?: string;
			state: "running" | "waiting_approval" | "failed";
	  }
	| {
			kind: "parallel";
			step: Step | undefined;
			stepName: string;
			occurrence: number;
			childSteps: ParallelStepState[];
			state: "running" | "completed" | "failed";
			entry?: StepHistoryEntry;
	  };

function buildTraceItems(workflowState: WorkflowState): TraceItem[] {
	const stepsByName = new Map(
		workflowState.workflowDefinition.steps.map((step) => [step.name, step]),
	);
	const seenCounts = new Map<string, number>();
	const items: TraceItem[] = workflowState.stepHistory.map((entry) => {
		const occurrence = (seenCounts.get(entry.stepName) ?? 0) + 1;
		seenCounts.set(entry.stepName, occurrence);
		const step = stepsByName.get(entry.stepName);

		if (step?.parallel && step.parallel.length > 0) {
			const childSteps: ParallelStepState[] = step.parallel.map((child) => {
				// 履歴エントリにchild snapshotがあればそれを使用（run固有の情報）
				const childSnapshot = entry.childOutputs?.find(
					(co) => co.stepName === child.name,
				);
				if (childSnapshot) {
					return {
						stepName: child.name,
						state: "completed" as const,
						sessionId: childSnapshot.sessionId,
						result: childSnapshot.result,
						runIndex: childSnapshot.runIndex,
						completedAt: childSnapshot.completedAt,
						structuredOutput: childSnapshot.structuredOutput,
						outputContract: childSnapshot.outputContract,
					};
				}
				// フォールバック: グローバルstepOutputsを参照
				const childOutput = workflowState.stepOutputs[child.name];
				return {
					stepName: child.name,
					state: childOutput ? "completed" : "pending",
					sessionId: childOutput?.sessionId,
					result: childOutput?.result,
					runIndex: childOutput?.runIndex ?? 1,
					completedAt: childOutput?.completedAt,
					structuredOutput: childOutput?.structuredOutput,
					outputContract: childOutput?.outputContract,
				};
			});
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

		return {
			kind: "completed",
			step,
			stepName: entry.stepName,
			occurrence,
			entry,
			sessionId: entry.sessionId ?? workflowState.chatSessionId,
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
		if (currentStep?.parallel && activeParallel && activeParallel.length > 0) {
			const hasFailed = activeParallel.some((ps) => ps.state === "failed");
			const allFinished = activeParallel.every(
				(ps) => ps.state === "completed" || ps.state === "failed",
			);
			items.push({
				kind: "parallel",
				step: currentStep,
				stepName: workflowState.currentStepName,
				occurrence: Math.max(startedCount, completedCount + 1),
				childSteps: activeParallel,
				state: hasFailed ? "failed" : allFinished ? "completed" : "running",
			});
		} else {
			items.push({
				kind: "current",
				step: currentStep,
				stepName: workflowState.currentStepName,
				occurrence: Math.max(startedCount, completedCount + 1),
				sessionId:
					workflowState.currentSessionId ?? workflowState.chatSessionId,
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
	index,
	isLast,
	onSessionClick,
	onFileClick,
}: {
	item: TraceItem;
	index: number;
	isLast: boolean;
	onSessionClick?: (sessionId: string) => void;
	onFileClick?: (path: string) => void;
}) {
	if (item.kind === "parallel") {
		return (
			<ParallelBlockRow
				item={item}
				index={index}
				isLast={isLast}
				onSessionClick={onSessionClick}
				onFileClick={onFileClick}
			/>
		);
	}

	const stepMode = item.step?.mode ?? "unknown";

	return (
		<div className="grid grid-cols-[24px_1fr] gap-2">
			<div className="flex flex-col items-center">
				<div
					className={`mt-2 flex size-5 items-center justify-center rounded-full border ${stateClasses[item.state] ?? stateClasses.pending}`}
				>
					<StateIcon state={item.state} />
				</div>
				{!isLast && <div className="w-px flex-1 min-h-4 bg-border" />}
			</div>
			<div
				className={`mb-2 overflow-hidden rounded-md border px-3 py-2 ${
					item.kind === "current"
						? "border-primary/60 bg-primary/5"
						: "border-border"
				}`}
			>
				<div className="flex min-w-0 flex-wrap items-center gap-2">
					<span className="text-sm font-medium truncate">{item.stepName}</span>
					<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
						{stepMode}
					</span>
					<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
						#{index + 1}
					</span>
					<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
						run {item.occurrence}
					</span>
				</div>
				<TraceItemSummary
					item={item}
					onSessionClick={onSessionClick}
					onFileClick={onFileClick}
				/>
			</div>
		</div>
	);
}

function ParallelBlockRow({
	item,
	index,
	isLast,
	onSessionClick,
	onFileClick,
}: {
	item: Extract<TraceItem, { kind: "parallel" }>;
	index: number;
	isLast: boolean;
	onSessionClick?: (sessionId: string) => void;
	onFileClick?: (path: string) => void;
}) {
	const completedCount = item.childSteps.filter(
		(cs) => cs.state === "completed",
	).length;
	const totalCount = item.childSteps.length;
	const tokenTotal = item.entry?.tokenUsage
		? item.entry.tokenUsage.inputTokens + item.entry.tokenUsage.outputTokens
		: null;

	return (
		<div className="grid grid-cols-[24px_1fr] gap-2">
			<div className="flex flex-col items-center">
				<div
					className={`mt-2 flex size-5 items-center justify-center rounded-full border ${stateClasses[item.state] ?? stateClasses.pending}`}
				>
					<StateIcon state={item.state} />
				</div>
				{!isLast && <div className="w-px flex-1 min-h-4 bg-border" />}
			</div>
			<div
				className={`mb-2 overflow-hidden rounded-md border px-3 py-2 ${
					item.state === "running"
						? "border-primary/60 bg-primary/5"
						: item.state === "failed"
							? "border-red-500/30 bg-red-500/5"
							: "border-border"
				}`}
			>
				<div className="flex min-w-0 flex-wrap items-center gap-2">
					<GitBranch className="size-3.5 text-muted-foreground" />
					<span className="text-sm font-medium truncate">{item.stepName}</span>
					<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
						parallel
					</span>
					<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
						#{index + 1}
					</span>
					<span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
						run {item.occurrence}
					</span>
				</div>
				<div className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
					<span>
						{completedCount}/{totalCount} completed
					</span>
					{item.entry?.result && <span>Result: {item.entry.result}</span>}
					{tokenTotal != null && (
						<span className="shrink-0">{tokenTotal} tokens</span>
					)}
				</div>
				<div className="mt-2 flex flex-col gap-1 pl-2 border-l-2 border-border">
					{item.childSteps.map((child) => (
						<ParallelChildRow
							key={`${child.stepName}-${child.runIndex}`}
							child={child}
							onSessionClick={onSessionClick}
							onFileClick={onFileClick}
						/>
					))}
				</div>
				{item.entry?.structuredOutput && (
					<div className="mt-2">
						<StructuredOutputToggle
							output={item.entry.structuredOutput}
							onFileClick={onFileClick}
						/>
					</div>
				)}
			</div>
		</div>
	);
}

function ParallelChildRow({
	child,
	onSessionClick,
	onFileClick,
}: {
	child: ParallelStepState;
	onSessionClick?: (sessionId: string) => void;
	onFileClick?: (path: string) => void;
}) {
	const so = child.structuredOutput;
	const verdict =
		so != null && typeof so === "object" && !Array.isArray(so)
			? ((so as Record<string, unknown>).verdict as string | undefined)
			: undefined;
	return (
		<div className="flex min-w-0 flex-col gap-1 rounded px-2 py-1 text-xs">
			<div className="flex items-center gap-2">
				<div
					className={`flex size-4 items-center justify-center rounded-full border ${stateClasses[child.state] ?? stateClasses.pending}`}
				>
					<StateIcon state={child.state} />
				</div>
				<span className="min-w-0 flex-1 truncate font-medium">
					{child.stepName}
				</span>
				{verdict && <VerdictBadge verdict={verdict} />}
				{!verdict && child.result && (
					<span className="rounded bg-muted px-1.5 py-0.5 text-muted-foreground">
						{child.result}
					</span>
				)}
				{child.sessionId && onSessionClick && (
					<button
						type="button"
						className="shrink-0 text-primary hover:underline"
						onClick={() => {
							if (child.sessionId) onSessionClick(child.sessionId);
						}}
					>
						View
					</button>
				)}
			</div>
			{child.structuredOutput && (
				<div className="ml-6">
					<StructuredOutputToggle
						output={child.structuredOutput}
						onFileClick={onFileClick}
					/>
				</div>
			)}
		</div>
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
	onSessionClick,
	onFileClick,
}: {
	item: Exclude<TraceItem, { kind: "parallel" }>;
	onSessionClick?: (sessionId: string) => void;
	onFileClick?: (path: string) => void;
}) {
	if (item.kind === "completed") {
		const tokenTotal = item.entry.tokenUsage
			? item.entry.tokenUsage.inputTokens + item.entry.tokenUsage.outputTokens
			: null;
		const isCollectStep = item.step?.collect != null;
		const reduceResult = isCollectStep ? item.entry.result : null;

		return (
			<div className="mt-1 space-y-1">
				<div className="flex items-center gap-2 text-xs text-muted-foreground">
					<span className="min-w-0 flex-1 truncate">
						Result: {item.entry.result ?? "completed"}
					</span>
					{item.entry.result && <VerdictBadge verdict={item.entry.result} />}
					{reduceResult && <VerdictBadge verdict={reduceResult} />}
					{tokenTotal != null && (
						<span className="shrink-0">{tokenTotal} tokens</span>
					)}
					{item.sessionId && onSessionClick && (
						<button
							type="button"
							className="shrink-0 text-primary hover:underline"
							onClick={() => {
								if (item.sessionId) onSessionClick(item.sessionId);
							}}
						>
							View
						</button>
					)}
				</div>
				{item.entry.structuredOutput && (
					<StructuredOutputToggle
						output={item.entry.structuredOutput}
						onFileClick={onFileClick}
					/>
				)}
			</div>
		);
	}

	if (item.sessionId && onSessionClick) {
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
				<button
					type="button"
					className="shrink-0 text-primary hover:underline"
					onClick={() => {
						if (item.sessionId) onSessionClick(item.sessionId);
					}}
				>
					View
				</button>
			</div>
		);
	}

	if (item.state === "running") {
		return <div className="mt-1 text-xs text-blue-600">Running</div>;
	}
	if (item.state === "waiting_approval") {
		return (
			<div className="mt-1 text-xs text-yellow-600">Waiting for approval</div>
		);
	}
	if (item.state === "failed") {
		return <div className="mt-1 text-xs text-red-600">Failed</div>;
	}
}

function StructuredOutputToggle({
	output,
	onFileClick,
}: {
	output: JsonValue;
	onFileClick?: (path: string) => void;
}) {
	const [expanded, setExpanded] = useState(false);
	const json = JSON.stringify(output, null, 2);
	const specFilePath =
		output != null &&
		typeof output === "object" &&
		!Array.isArray(output) &&
		typeof (output as Record<string, unknown>).spec_file_path === "string"
			? ((output as Record<string, unknown>).spec_file_path as string)
			: undefined;

	return (
		<div className="text-xs">
			{specFilePath && (
				<button
					type="button"
					className="text-primary hover:underline"
					onClick={() => onFileClick?.(specFilePath)}
				>
					{specFilePath}
				</button>
			)}
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

function EventTrace({ events }: { events: WorkflowLogEvent[] }) {
	return (
		<div className="rounded-md border">
			<div className="border-b px-3 py-1.5 text-xs font-medium text-muted-foreground">
				Event log
			</div>
			<div className="max-h-32 overflow-auto px-3 py-1">
				{events.map((event) => (
					<div
						key={`${event.event}-${"step_name" in event ? event.step_name : ""}-${event.timestamp}`}
						className="py-0.5 text-xs text-muted-foreground"
					>
						<span className="font-mono">{event.event}</span>
						{"step_name" in event && event.step_name && (
							<span className="ml-1">({event.step_name})</span>
						)}
						{event.event === "contract_repair_requested" &&
							"attempt" in event && (
								<span className="ml-1 text-yellow-600 dark:text-yellow-400">
									retry #{event.attempt as number}
									{"violation_reason" in event &&
										`: ${event.violation_reason as string}`}
								</span>
							)}
					</div>
				))}
			</div>
		</div>
	);
}
