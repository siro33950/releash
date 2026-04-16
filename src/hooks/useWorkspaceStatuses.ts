import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { WorkspaceStatus } from "@/types/session";

/**
 * 全 worktree の WorkspaceStatus マップを Rust 中央管理から購読する。
 * - マウント時に `list_workspace_statuses` で初期値取得
 * - `workspace-status-changed` イベントで該当エントリだけ更新
 *
 * 戻り値は `worktree_id` をキーにした Record。
 */
export function useWorkspaceStatuses(): Record<string, WorkspaceStatus> {
	const [statuses, setStatuses] = useState<Record<string, WorkspaceStatus>>({});

	useEffect(() => {
		let mounted = true;
		let unlisten: UnlistenFn | null = null;

		const subscribe = async () => {
			try {
				unlisten = await listen<WorkspaceStatus>(
					"workspace-status-changed",
					(event) => {
						if (!mounted) return;
						setStatuses((prev) => ({
							...prev,
							[event.payload.worktree_id]: event.payload,
						}));
					},
				);

				if (!mounted) {
					unlisten?.();
					return;
				}

				const initial = await invoke<WorkspaceStatus[]>(
					"list_workspace_statuses",
				);
				if (mounted) {
					const map: Record<string, WorkspaceStatus> = {};
					for (const ws of initial) {
						map[ws.worktree_id] = ws;
					}
					setStatuses(map);
				}
			} catch {
				if (mounted) {
					setStatuses({});
				}
			}
		};

		subscribe();

		return () => {
			mounted = false;
			unlisten?.();
		};
	}, []);

	return statuses;
}
