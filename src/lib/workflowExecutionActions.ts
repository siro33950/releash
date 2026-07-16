import { invoke } from "@tauri-apps/api/core";

export type WorkflowExecutionAction = "stop" | "resume" | "abort";

const actionCommand: Record<
	WorkflowExecutionAction,
	"stop_workflow" | "resume_workflow" | "abort_workflow"
> = {
	stop: "stop_workflow",
	resume: "resume_workflow",
	abort: "abort_workflow",
};

const actionLabel: Record<WorkflowExecutionAction, string> = {
	stop: "Stop",
	resume: "Resume",
	abort: "Abort",
};

export async function executeWorkflowAction(
	action: WorkflowExecutionAction,
	executionId: string,
): Promise<void> {
	try {
		await invoke(actionCommand[action], { executionId });
	} catch (error) {
		throw new Error(`${actionLabel[action]} workflow failed: ${String(error)}`);
	}
}
