import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type {
	WorkspaceStepStatus,
	WorktreeStepStatusView,
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

function viewToStatuses(view: WorktreeStepStatusView): WorktreeStepStatuses {
	const steps = new Map<string, WorkspaceStepStatus>();
	for (const step of view.steps) {
		steps.set(
			workflowStepStatusKey(step.executionId, step.stepName, step.runIndex),
			step.representative,
		);
	}
	const workflows = new Map<string, WorkspaceStepStatus>();
	for (const workflow of view.workflows) {
		workflows.set(workflow.executionId, workflow.representative);
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

	useEffect(() => {
		if (!worktreePath) {
			setStatuses({ steps: new Map(), workflows: new Map() });
			return;
		}

		let mounted = true;
		let unlisten: UnlistenFn | null = null;
		setStatuses({ steps: new Map(), workflows: new Map() });

		const applyView = (view: WorktreeStepStatusView) => {
			if (view.worktreePath !== worktreePath) return;
			setStatuses(viewToStatuses(view));
		};

		const subscribe = async () => {
			let subscribed = false;
			try {
				unlisten = await listen<WorktreeStepStatusView>(
					"workflow-step-status-changed",
					(event) => {
						if (!mounted) return;
						applyView(event.payload);
					},
				);
				subscribed = true;

				if (!mounted) {
					unlisten?.();
					return;
				}

				await invoke("sync_worktree_step_statuses", { worktreePath });
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
