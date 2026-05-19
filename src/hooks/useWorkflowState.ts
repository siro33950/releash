import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { WorkflowState, WorkflowStatePayload } from "@/types/workflow";

export function useWorkflowState(worktreePath: string | undefined) {
	const [workflowState, setWorkflowState] = useState<WorkflowState | null>(
		null,
	);

	// Tauriイベントをリッスン
	useEffect(() => {
		if (!worktreePath) {
			setWorkflowState(null);
			return;
		}

		// 別 worktree へ切り替わった直後は一旦クリアして古い表示を防ぐ
		setWorkflowState(null);

		let cancelled = false;

		// 初期状態を取得（nullの場合も反映し、前セッションの古い状態をリセット）。
		// run_id 主語 API へ移行: worktreePath -> active run_id を解決し、
		// run_id をキーに get_workflow_state を呼ぶ。
		invoke<string | null>("resolve_active_run_by_worktree", { worktreePath })
			.then((runId) => {
				if (cancelled) return null;
				if (!runId) {
					setWorkflowState(null);
					return null;
				}
				return invoke<WorkflowState | null>("get_workflow_state", {
					runId,
				}).then((state) => {
					if (!cancelled) {
						setWorkflowState(state ?? null);
					}
					return null;
				});
			})
			.catch((e) =>
				console.warn("[useWorkflowState] get_workflow_state failed", e),
			);

		let unlisten: UnlistenFn | null = null;
		const setup = listen<WorkflowStatePayload>(
			"workflow-state-changed",
			(event) => {
				if (!cancelled && event.payload.worktreePath === worktreePath) {
					setWorkflowState(event.payload.workflowState);
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

	return { workflowState };
}
