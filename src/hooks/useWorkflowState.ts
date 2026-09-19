import type { UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { invokeClient as invoke, listenClient } from "@/lib/client";
import type { WorkflowExecution } from "@/types/workflow";

export function useWorkflowState(worktreePath: string | undefined) {
	const [workflowExecution, setWorkflowExecution] =
		useState<WorkflowExecution | null>(null);

	useEffect(() => {
		if (!worktreePath) {
			setWorkflowExecution(null);
			return;
		}

		// 別 worktree へ切り替わった直後は一旦クリアして古い表示を防ぐ
		setWorkflowExecution(null);

		let cancelled = false;

		let loadSequence = 0;
		const load = () => {
			const sequence = ++loadSequence;
			invoke("resolve_active_execution_by_worktree", {
				worktreePath,
			})
				.then((executionId) => {
					if (cancelled || sequence !== loadSequence) return null;
					if (!executionId) {
						setWorkflowExecution(null);
						return null;
					}
					return invoke("get_workflow_execution_state", {
						worktreePath,
						executionId,
					}).then((execution) => {
						if (!cancelled && sequence === loadSequence) {
							setWorkflowExecution(execution ?? null);
						}
						return null;
					});
				})
				.catch((e) =>
					console.warn(
						"[useWorkflowState] get_workflow_execution_state failed",
						e,
					),
				);
		};
		load();

		let unlisten: UnlistenFn | null = null;
		const setup = listenClient(
			"workflow-execution-changed",
			(event) => {
				if (!cancelled && event.payload.worktreePath === worktreePath) {
					loadSequence += 1;
					setWorkflowExecution(event.payload.workflowExecution);
				}
			},
			() => {
				if (!cancelled) load();
			},
		)
			.then((fn) => {
				if (cancelled) {
					fn();
				} else {
					unlisten = fn;
				}
			})
			.catch((error) =>
				console.warn("Workflow push subscription failed:", error),
			);

		return () => {
			cancelled = true;
			setup.then(() => {
				unlisten?.();
			});
		};
	}, [worktreePath]);

	return { workflowExecution };
}
