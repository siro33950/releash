import { useEffect, useState } from "react";
import type { Subscribe } from "@/remote/hooks/useMessageBus";
import type { WorkflowState } from "@/types/workflow";

interface UseRemoteWorkflowStateOptions {
	subscribe: Subscribe;
	selectedWorktree: string | null;
}

export function useRemoteWorkflowState({
	subscribe,
	selectedWorktree,
}: UseRemoteWorkflowStateOptions) {
	const [workflowState, setWorkflowState] = useState<WorkflowState | null>(
		null,
	);

	useEffect(() => {
		setWorkflowState(null);
		return subscribe((msg) => {
			if (
				msg.type === "workflow_state_sync" &&
				selectedWorktree !== null &&
				msg.payload.worktreePath === selectedWorktree
			) {
				setWorkflowState(msg.payload.workflowState);
			}
		});
	}, [subscribe, selectedWorktree]);

	return { workflowState };
}
