import type { StepHistoryEntry, WorkflowState } from "@/types/workflow";

interface StepDetailProps {
	stepName: string;
	workflowState: WorkflowState;
	onSessionClick?: (sessionId: string) => void;
}

export function StepDetail({
	stepName,
	workflowState,
	onSessionClick,
}: StepDetailProps) {
	const entries = workflowState.stepHistory.filter(
		(h) => h.stepName === stepName,
	);
	const stepState = workflowState.stepStates[stepName];

	if (entries.length === 0) {
		if (stepState === "running") {
			return (
				<div className="px-3 py-2 text-xs text-blue-600 dark:text-blue-400">
					実行中
				</div>
			);
		}
		if (stepState === "waiting_approval") {
			return (
				<div className="px-3 py-2 text-xs text-yellow-600 dark:text-yellow-400">
					承認待ち
				</div>
			);
		}
		return (
			<div className="px-3 py-2 text-xs text-muted-foreground">未実行</div>
		);
	}

	return (
		<div className="flex flex-col gap-1 px-3 py-2">
			{stepState === "running" && (
				<div className="text-xs text-blue-600 dark:text-blue-400">実行中</div>
			)}
			{stepState === "waiting_approval" && (
				<div className="text-xs text-yellow-600 dark:text-yellow-400">
					承認待ち
				</div>
			)}
			{entries.map((entry, i) => (
				<StepHistoryRow
					key={`${entry.stepName}-${entry.completedAt}`}
					entry={entry}
					index={i}
					onSessionClick={onSessionClick}
				/>
			))}
		</div>
	);
}

function StepHistoryRow({
	entry,
	index,
	onSessionClick,
}: {
	entry: StepHistoryEntry;
	index: number;
	onSessionClick?: (sessionId: string) => void;
}) {
	return (
		<div className="flex items-center justify-between text-xs gap-2">
			<span className="text-muted-foreground">#{index + 1}</span>
			{entry.result && <span className="truncate flex-1">{entry.result}</span>}
			{entry.tokenUsage && (
				<span className="text-muted-foreground whitespace-nowrap">
					{entry.tokenUsage.inputTokens + entry.tokenUsage.outputTokens} tokens
				</span>
			)}
			{entry.sessionId != null && onSessionClick && (
				<button
					type="button"
					className="text-primary hover:underline whitespace-nowrap"
					onClick={() => {
						if (entry.sessionId) onSessionClick(entry.sessionId);
					}}
				>
					View
				</button>
			)}
		</div>
	);
}
