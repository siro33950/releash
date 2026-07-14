import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type {
	WorkflowExecution,
	WorkflowExecutionChangedPayload,
} from "@/types/workflow";

export function useWorkflowState(worktreePath: string | undefined) {
	const [workflowExecution, setWorkflowExecution] =
		useState<WorkflowExecution | null>(null);

	// Tauriイベントをリッスン
	useEffect(() => {
		if (!worktreePath) {
			setWorkflowExecution(null);
			return;
		}

		// 別 worktree へ切り替わった直後は一旦クリアして古い表示を防ぐ
		setWorkflowExecution(null);

		let cancelled = false;

		// 初期状態を取得（null の場合も反映し、前の表示をリセットする）。
		invoke<string | null>("resolve_active_execution_by_worktree", {
			worktreePath,
		})
			.then((executionId) => {
				if (cancelled) return null;
				if (!executionId) {
					setWorkflowExecution(null);
					return null;
				}
				return invoke<WorkflowExecution | null>(
					"get_workflow_execution_state",
					{ worktreePath, executionId },
				).then((execution) => {
					if (!cancelled) {
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

		let unlisten: UnlistenFn | null = null;
		const setup = listen<WorkflowExecutionChangedPayload>(
			"workflow-execution-changed",
			(event) => {
				if (!cancelled && event.payload.worktreePath === worktreePath) {
					setWorkflowExecution(event.payload.workflowExecution);
				}
			},
		).then((fn) => {
			if (cancelled) {
				fn();
			} else {
				unlisten = fn;
			}
		});

		return () => {
			cancelled = true;
			setup.then(() => {
				unlisten?.();
			});
		};
	}, [worktreePath]);

	return { workflowExecution };
}
