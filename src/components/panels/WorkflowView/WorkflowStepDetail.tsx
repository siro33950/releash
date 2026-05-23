import { X } from "lucide-react";
import type { WorkflowStepSelection } from "@/components/panels/WorkflowPanel";
import {
	useWorkflowStepDetail,
	type WorkflowStepDetailView,
	type WorkflowStepInputView,
} from "@/hooks/useWorkflowStepDetail";

interface WorkflowStepDetailProps {
	selection: WorkflowStepSelection;
	worktreePath: string;
	onClose: () => void;
}

/**
 * spec issues-1023: timeline 上で選択した step の入出力・遷移結果・所要時間を
 * inline で観測するための pane。
 *
 * 表示用整形のみを担う。事実列の整序・所要時間計算・入出力の引き当ては engine 側
 * `get_workflow_step_detail` Tauri command が一次的に提供し、frontend は受け取った
 * View を整形して描画する。
 */
export function WorkflowStepDetail({
	selection,
	worktreePath,
	onClose,
}: WorkflowStepDetailProps) {
	const { detail, isLoading, error } = useWorkflowStepDetail({
		worktreePath,
		runId: selection.runId,
		nodeName: selection.stepName,
		runIndex: selection.runIndex,
	});

	return (
		<div className="flex shrink-0 flex-col overflow-hidden">
			<div className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
				<div className="min-w-0 flex-1">
					<div className="truncate text-xs text-muted-foreground">
						Step detail
					</div>
					<div className="truncate text-sm font-medium">
						{selection.stepName}
					</div>
				</div>
				<span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
					{detail?.nodeType ?? selection.nodeType}
				</span>
				<button
					type="button"
					onClick={onClose}
					aria-label="Close step detail"
					className="rounded p-1 text-muted-foreground transition-colors hover:bg-muted-foreground/20 hover:text-foreground"
				>
					<X className="size-3.5" />
				</button>
			</div>
			<div
				data-testid="workflow-step-detail"
				className="flex flex-col gap-1 px-3 py-2 text-xs"
			>
				{error && (
					<div
						role="alert"
						className="rounded border border-red-500/40 bg-red-500/10 px-2 py-1 text-red-700 dark:text-red-300"
					>
						{error}
					</div>
				)}
				{isLoading && !detail && (
					<div className="text-muted-foreground">Loading...</div>
				)}
				{detail && <DetailBody detail={detail} />}
			</div>
		</div>
	);
}

function DetailBody({ detail }: { detail: WorkflowStepDetailView }) {
	return (
		<>
			<DetailRow label="State" value={detail.state} />
			<DetailRow label="Transition result" value={detail.result ?? "—"} />
			<DetailRow label="Started" value={formatTimestamp(detail.startedAtMs)} />
			<DetailRow
				label="Completed"
				value={formatTimestamp(detail.completedAtMs)}
			/>
			<DetailRow label="Duration" value={formatDuration(detail.durationMs)} />
			{detail.tokenUsage && (
				<DetailRow
					label="Tokens"
					value={`${detail.tokenUsage.inputTokens} in / ${detail.tokenUsage.outputTokens} out`}
				/>
			)}
			<InputSection input={detail.input ?? {}} />
			{detail.structuredOutput !== undefined && (
				<div className="mt-1">
					<div className="text-muted-foreground">Output</div>
					<pre className="mt-1 max-h-40 overflow-auto rounded bg-muted p-2 whitespace-pre-wrap break-words">
						{JSON.stringify(detail.structuredOutput, null, 2)}
					</pre>
				</div>
			)}
		</>
	);
}

function InputSection({ input }: { input: WorkflowStepInputView }) {
	const hasAny =
		input.instruction != null ||
		input.policy != null ||
		input.knowledge != null ||
		input.outputContract != null ||
		(input.inputContracts && input.inputContracts.length > 0) ||
		input.previousStepName != null ||
		input.previousStepStructuredOutput !== undefined;
	if (!hasAny) return null;
	return (
		<div
			data-testid="workflow-step-detail-input"
			className="mt-2 flex flex-col gap-1 border-t pt-2"
		>
			<div className="text-muted-foreground">Input</div>
			{input.instruction && (
				<DetailRow label="Instruction" value={input.instruction} />
			)}
			{input.policy && <DetailRow label="Policy" value={input.policy} />}
			{input.knowledge && (
				<DetailRow label="Knowledge" value={input.knowledge} />
			)}
			{input.outputContract && (
				<DetailRow label="Output contract" value={input.outputContract} />
			)}
			{input.inputContracts && input.inputContracts.length > 0 && (
				<DetailRow
					label="Input contracts"
					value={input.inputContracts.join(", ")}
				/>
			)}
			{input.previousStepName && (
				<DetailRow label="Previous step" value={input.previousStepName} />
			)}
			{input.previousStepStructuredOutput !== undefined && (
				<div className="mt-1">
					<div className="text-muted-foreground">Previous output</div>
					<pre className="mt-1 max-h-40 overflow-auto rounded bg-muted p-2 whitespace-pre-wrap break-words">
						{JSON.stringify(input.previousStepStructuredOutput, null, 2)}
					</pre>
				</div>
			)}
		</div>
	);
}

function DetailRow({ label, value }: { label: string; value: string }) {
	return (
		<div className="flex min-w-0 items-baseline gap-2">
			<span className="shrink-0 text-muted-foreground">{label}</span>
			<span className="min-w-0 flex-1 truncate font-mono">{value}</span>
		</div>
	);
}

function formatTimestamp(ms: number | undefined): string {
	if (ms === undefined || Number.isNaN(ms)) return "—";
	const d = new Date(ms);
	if (Number.isNaN(d.getTime())) return "—";
	const hh = String(d.getHours()).padStart(2, "0");
	const mm = String(d.getMinutes()).padStart(2, "0");
	const ss = String(d.getSeconds()).padStart(2, "0");
	return `${hh}:${mm}:${ss}`;
}

function formatDuration(ms: number | undefined): string {
	if (ms === undefined || Number.isNaN(ms)) return "—";
	if (ms < 1000) return `${Math.round(ms)} ms`;
	const seconds = ms / 1000;
	if (seconds < 60) return `${seconds.toFixed(1)} s`;
	const minutes = Math.floor(seconds / 60);
	const rem = Math.round(seconds % 60);
	return `${minutes}m ${rem}s`;
}
