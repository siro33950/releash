import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";

export type WorkflowExecutionAction = "abort";

export async function executeWorkflowAction(
	_action: WorkflowExecutionAction,
	executionId: string,
): Promise<void> {
	try {
		await invoke("abort_workflow", { executionId });
	} catch (error) {
		throw new Error(getErrorMessage(error));
	}
}
