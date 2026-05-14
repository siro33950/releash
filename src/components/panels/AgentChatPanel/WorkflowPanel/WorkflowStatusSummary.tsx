import type { WorkflowState } from "@/types/workflow";
import { workflowStateClasses } from "./workflowStateStyles";

interface WorkflowStatusSummaryProps {
	workflowState: WorkflowState;
}

export function WorkflowStatusSummary({
	workflowState,
}: WorkflowStatusSummaryProps) {
	const totalTokens =
		workflowState.totalTokenUsage.inputTokens +
		workflowState.totalTokenUsage.outputTokens;
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
			data-testid="workflow-status-summary"
			className={`overflow-hidden rounded-md border px-3 py-2 ${workflowStateClasses[state] ?? workflowStateClasses.pending}`}
		>
			<div className="flex items-center justify-between gap-3">
				<div className="min-w-0">
					<div className="truncate text-sm font-medium">{label}</div>
					<div className="truncate text-xs opacity-80">{details}</div>
				</div>
				<div className="whitespace-nowrap text-xs opacity-80">
					{totalTokens} tokens
				</div>
			</div>
		</div>
	);
}
