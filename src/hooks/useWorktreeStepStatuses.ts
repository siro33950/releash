import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type {
	WorkflowStepStatusChange,
	WorkspaceStepStatus,
} from "@/types/workspace-tree";

export interface WorktreeStepStatuses {
	steps: Map<string, WorkspaceStepStatus>;
	workflows: Map<string, WorkspaceStepStatus>;
}

export function workflowStepStatusKey(
	executionId: string,
	stepName: string,
	runIndex?: number | null,
): string {
	return `${executionId}:${stepName}:${runIndex ?? 1}`;
}

function applyStatusChange(
	prev: WorktreeStepStatuses,
	change: WorkflowStepStatusChange,
	stepVersions: Map<string, number>,
	workflowVersions: Map<string, number>,
): WorktreeStepStatuses {
	const steps = new Map(prev.steps);
	const workflows = new Map(prev.workflows);
	const key = workflowStepStatusKey(
		change.executionId,
		change.stepName,
		change.runIndex,
	);
	const previousStepVersion = stepVersions.get(key) ?? -1;
	if (change.version >= previousStepVersion) {
		stepVersions.set(key, change.version);
		if (change.representative) {
			steps.set(key, change.representative);
		} else {
			steps.delete(key);
		}
	}
	const previousWorkflowVersion =
		workflowVersions.get(change.executionId) ?? -1;
	if (change.version >= previousWorkflowVersion) {
		workflowVersions.set(change.executionId, change.version);
		if (change.workflowRepresentative) {
			workflows.set(change.executionId, change.workflowRepresentative);
		} else {
			workflows.delete(change.executionId);
		}
	}
	return { steps, workflows };
}

export function useWorktreeStepStatuses(
	worktreePath: string | null,
): WorktreeStepStatuses {
	const [statuses, setStatuses] = useState<WorktreeStepStatuses>({
		steps: new Map(),
		workflows: new Map(),
	});
	const stepVersions = useRef(new Map<string, number>());
	const workflowVersions = useRef(new Map<string, number>());

	useEffect(() => {
		if (!worktreePath) {
			stepVersions.current = new Map();
			workflowVersions.current = new Map();
			setStatuses({ steps: new Map(), workflows: new Map() });
			return;
		}

		let mounted = true;
		let unlisten: UnlistenFn | null = null;
		stepVersions.current = new Map();
		workflowVersions.current = new Map();
		setStatuses({ steps: new Map(), workflows: new Map() });

		const subscribe = async () => {
			let subscribed = false;
			try {
				unlisten = await listen<WorkflowStepStatusChange>(
					"workflow-step-status-changed",
					(event) => {
						if (!mounted) return;
						if (event.payload.worktreePath !== worktreePath) return;
						setStatuses((prev) =>
							applyStatusChange(
								prev,
								event.payload,
								stepVersions.current,
								workflowVersions.current,
							),
						);
					},
				);
				subscribed = true;

				if (!mounted) {
					unlisten?.();
					return;
				}

				const initial = await invoke<WorkflowStepStatusChange[]>(
					"list_workflow_step_statuses",
				);
				if (!mounted) return;
				setStatuses((prev) => {
					let merged = prev;
					for (const status of Array.isArray(initial) ? initial : []) {
						if (status.worktreePath !== worktreePath) continue;
						merged = applyStatusChange(
							merged,
							status,
							stepVersions.current,
							workflowVersions.current,
						);
					}
					return merged;
				});
			} catch {
				if (mounted && !subscribed) {
					setStatuses({ steps: new Map(), workflows: new Map() });
				}
			}
		};

		subscribe();

		return () => {
			mounted = false;
			unlisten?.();
		};
	}, [worktreePath]);

	return statuses;
}
