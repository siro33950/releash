import {
	AlertTriangle,
	Ban,
	CheckCircle2,
	Circle,
	Clock,
	Loader2,
} from "lucide-react";
import type {
	Step,
	StepHistoryEntry,
	WorkflowLogEvent,
	WorkflowState,
} from "@/types/workflow";

interface WorkflowTraceProps {
	workflowState: WorkflowState;
	events?: WorkflowLogEvent[];
	onSessionClick?: (sessionId: string) => void;
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
				? `${currentStep.mode} step`
				: `${workflowState.stepHistory.length} recorded steps`;

	return (
		<div
			className={`rounded-md border px-3 py-2 ${stateClasses[state] ?? stateClasses.pending}`}
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
	  };

function buildTraceItems(workflowState: WorkflowState): TraceItem[] {
	const stepsByName = new Map(
		workflowState.workflowDefinition.steps.map((step) => [step.name, step]),
	);
	const seenCounts = new Map<string, number>();
	const items: TraceItem[] = workflowState.stepHistory.map((entry) => {
		const occurrence = (seenCounts.get(entry.stepName) ?? 0) + 1;
		seenCounts.set(entry.stepName, occurrence);
		return {
			kind: "completed",
			step: stepsByName.get(entry.stepName),
			stepName: entry.stepName,
			occurrence,
			entry,
			sessionId: entry.sessionId ?? workflowState.chatSessionId,
			state: "completed",
		};
	});

	const state = workflowState.state.type;
	if (
		(state === "running" ||
			state === "waiting_approval" ||
			state === "failed") &&
		workflowState.currentStepName
	) {
		const completedCount = seenCounts.get(workflowState.currentStepName) ?? 0;
		const startedCount =
			workflowState.stepExecutionCounts[workflowState.currentStepName] ??
			completedCount + 1;
		items.push({
			kind: "current",
			step: stepsByName.get(workflowState.currentStepName),
			stepName: workflowState.currentStepName,
			occurrence: Math.max(startedCount, completedCount + 1),
			sessionId: workflowState.currentSessionId ?? workflowState.chatSessionId,
			state,
		});
	}

	return items;
}

function traceItemKey(item: TraceItem, index: number) {
	if (item.kind === "completed") {
		return `${item.stepName}-${item.occurrence}-${item.entry.completedAt}-${item.entry.sessionId ?? item.entry.result ?? "done"}`;
	}
	return `${item.stepName}-${item.occurrence}-${item.state}-${index}`;
}

function TraceItemRow({
	item,
	index,
	isLast,
	onSessionClick,
}: {
	item: TraceItem;
	index: number;
	isLast: boolean;
	onSessionClick?: (sessionId: string) => void;
}) {
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
				className={`mb-2 rounded-md border px-3 py-2 ${
					item.kind === "current"
						? "border-primary/60 bg-primary/5"
						: "border-border"
				}`}
			>
				<div className="flex items-start justify-between gap-3">
					<div className="min-w-0">
						<div className="flex items-center gap-2">
							<span className="text-sm font-medium truncate">
								{item.stepName}
							</span>
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
						<TraceItemSummary item={item} onSessionClick={onSessionClick} />
					</div>
					<span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
						{item.state}
					</span>
				</div>
			</div>
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
}: {
	item: TraceItem;
	onSessionClick?: (sessionId: string) => void;
}) {
	if (item.kind === "completed") {
		const tokenTotal = item.entry.tokenUsage
			? item.entry.tokenUsage.inputTokens + item.entry.tokenUsage.outputTokens
			: null;

		return (
			<div className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
				<span className="min-w-0 flex-1 truncate">
					Result: {item.entry.result ?? "completed"}
				</span>
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
					</div>
				))}
			</div>
		</div>
	);
}
