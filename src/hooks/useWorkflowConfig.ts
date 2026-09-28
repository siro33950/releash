import { useCallback, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import { useStateSubscriptionResult } from "./useStateSubscription";

export function useWorkflowConfig(open: boolean) {
	const subscription = useStateSubscriptionResult(open ? "workflows" : null);
	const [operationError, setOperationError] = useState<string | null>(null);

	const deleteWorkflow = useCallback(async (name: string) => {
		setOperationError(null);
		try {
			await invoke("delete_workflow", { name });
		} catch (e) {
			setOperationError(getErrorMessage(e));
		}
	}, []);

	const openInEditor = useCallback(async (name: string) => {
		setOperationError(null);
		try {
			await invoke("open_workflow_in_editor", { name });
		} catch (e) {
			setOperationError(getErrorMessage(e));
		}
	}, []);

	return {
		workflows: subscription.value ?? [],
		loading: open && subscription.value === undefined && !subscription.error,
		error: operationError ?? subscription.error,
		deleteWorkflow,
		openInEditor,
	};
}
