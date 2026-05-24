import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { WorkflowEvent } from "@/types/workflow";

/**
 * spec issues-1023: 現 worktree に紐づく run の event log を observation 経路から
 * 取得するための薄い invoke ラッパー。active run / history run の両方で同じ経路を
 * 共有するため、`refreshKey` を変えると再 fetch する（active 側は state.updatedAt
 * を渡し、history 側は固定値で 1 回読む）。
 *
 * worktree_path は必須。engine 側の worktree 認可境界
 * （`canonicalize_managed_worktree_path` + run metadata の `worktree_path` 一致）を
 * 通過させる（spec L132/L150 観測 invoke 認可境界）。
 */
export interface UseWorkflowRunLogResult {
	events: WorkflowEvent[];
	error: string | null;
}

export function useWorkflowRunLog(
	worktreePath: string | null,
	runId: string | null,
	refreshKey: string | number,
): UseWorkflowRunLogResult {
	const [events, setEvents] = useState<WorkflowEvent[]>([]);
	const [error, setError] = useState<string | null>(null);

	// biome-ignore lint/correctness/useExhaustiveDependencies: refreshKey は明示的な re-fetch trigger（active run の updatedAt 等）
	useEffect(() => {
		if (!worktreePath || !runId) {
			setEvents([]);
			setError(null);
			return;
		}
		let cancelled = false;
		invoke<WorkflowEvent[] | null>("get_workflow_run_log", {
			worktreePath,
			runId,
		})
			.then((log) => {
				if (cancelled) return;
				setEvents(log ?? []);
				setError(null);
			})
			.catch((e) => {
				console.warn("[useWorkflowRunLog] get_workflow_run_log failed", e);
				if (cancelled) return;
				setEvents([]);
				setError(String(e));
			});
		return () => {
			cancelled = true;
		};
	}, [worktreePath, runId, refreshKey]);

	return { events, error };
}
