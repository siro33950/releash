import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { WorkflowDefinitionSummary } from "@/types/workflow";

export function useWorkflowConfig(open: boolean) {
	const [workflows, setWorkflows] = useState<WorkflowDefinitionSummary[]>([]);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const fetchWorkflows = useCallback(async () => {
		setLoading(true);
		setError(null);
		try {
			const list = await invoke<WorkflowDefinitionSummary[]>("list_workflows");
			setWorkflows(list);
		} catch (e) {
			setError(String(e));
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		if (open) {
			fetchWorkflows();
		}
	}, [open, fetchWorkflows]);

	const deleteWorkflow = useCallback(
		async (name: string) => {
			setError(null);
			try {
				await invoke("delete_workflow", { name });
				await fetchWorkflows();
			} catch (e) {
				setError(String(e));
			}
		},
		[fetchWorkflows],
	);

	const openInEditor = useCallback(async (name: string) => {
		setError(null);
		try {
			await invoke("open_workflow_in_editor", { name });
		} catch (e) {
			setError(String(e));
		}
	}, []);

	return {
		workflows,
		loading,
		error,
		deleteWorkflow,
		openInEditor,
	};
}
