import { invoke } from "@tauri-apps/api/core";
import { Check, Play, Square, X } from "lucide-react";
import { useCallback, useState } from "react";
import { useWorkflowConfig } from "@/hooks/useWorkflowConfig";
import type { WorkflowState } from "@/types/workflow";
import { StepDetail } from "./StepDetail";
import { WorkflowGraph } from "./WorkflowGraph";
import { WorkflowHistory } from "./WorkflowHistory";

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
	if (!workflowState || !chatSessionId) {
		return (
			<WorkflowEmptyState
				chatSessionId={chatSessionId}
				worktreePath={worktreePath}
			/>
		);
	}

	return (
		<WorkflowActivePanel
			workflowState={workflowState}
			worktreePath={worktreePath}
			onSessionClick={onSessionClick}
		/>
	);
}

function WorkflowEmptyState({
	chatSessionId,
	worktreePath,
}: {
	chatSessionId: string | null;
	worktreePath: string;
}) {
	const [open, setOpen] = useState(false);
	const { workflows } = useWorkflowConfig(open);

	const handleStart = useCallback(
		(workflowName: string) => {
			if (!chatSessionId) return;
			setOpen(false);
			invoke("start_workflow", {
				workflowName,
				chatSessionId,
			}).catch((e) => console.warn("[WorkflowPanel] start_workflow failed", e));
		},
		[chatSessionId],
	);

	return (
		<div className="flex flex-col h-full overflow-hidden">
			<div className="flex items-center px-3 py-2 border-b shrink-0">
				<span className="text-sm font-medium">Workflow</span>
			</div>
			<div className="flex-1 flex flex-col items-center justify-center gap-3 text-muted-foreground">
				{open ? (
					<div className="w-full max-w-[200px]">
						{workflows.length > 0 ? (
							<ul className="border rounded overflow-hidden">
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
							<p className="text-sm text-center">No workflows</p>
						)}
						<button
							type="button"
							className="mt-2 w-full text-xs text-center text-muted-foreground hover:text-foreground transition-colors"
							onClick={() => setOpen(false)}
						>
							Cancel
						</button>
					</div>
				) : (
					<>
						<p className="text-sm">No workflow running</p>
						{chatSessionId && (
							<button
								type="button"
								onClick={() => setOpen(true)}
								className="flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-md border hover:bg-muted transition-colors text-foreground"
							>
								<Play className="size-3.5" />
								Start workflow
							</button>
						)}
					</>
				)}
			</div>
			<div className="border-t shrink-0 max-h-[30%] overflow-auto">
				<WorkflowHistory worktreePath={worktreePath} workflowState={null} />
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
	const [selectedStep, setSelectedStep] = useState<string | null>(null);

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
			{/* Header */}
			<div className="flex items-center justify-between px-3 py-2 border-b shrink-0">
				<div className="flex items-center gap-2">
					<span className="text-sm font-medium">
						{workflowState.workflowName}
					</span>
					<StatusBadge state={workflowState.state.type} />
				</div>
				<div className="flex items-center gap-2">
					<span className="text-xs text-muted-foreground">
						{workflowState.totalTokenUsage.inputTokens +
							workflowState.totalTokenUsage.outputTokens}{" "}
						tokens
					</span>
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

			{/* Graph */}
			<div className="flex-1 overflow-auto min-h-0">
				<WorkflowGraph
					workflowState={workflowState}
					onStepClick={setSelectedStep}
				/>
			</div>

			{/* Step Detail */}
			{selectedStep && (
				<div className="border-t shrink-0 max-h-[30%] overflow-auto">
					<div className="flex items-center justify-between px-3 py-1 border-b">
						<span className="text-xs font-medium">{selectedStep}</span>
						<button
							type="button"
							className="text-xs text-muted-foreground hover:text-foreground"
							onClick={() => setSelectedStep(null)}
						>
							Close
						</button>
					</div>
					<StepDetail
						stepName={selectedStep}
						workflowState={workflowState}
						onSessionClick={onSessionClick}
					/>
				</div>
			)}

			{/* History */}
			<div className="border-t shrink-0 max-h-[30%] overflow-auto">
				<WorkflowHistory
					worktreePath={worktreePath}
					workflowState={workflowState}
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
