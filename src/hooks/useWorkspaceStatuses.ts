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
			let subscribed = false;
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
				subscribed = true;

				if (!mounted) {
					unlisten?.();
					return;
				}

				const initial = await invoke<WorkspaceStatus[]>(
					"list_workspace_statuses",
				);
				if (mounted) {
					// listen 登録後・invoke await 中に届いた最新イベントを尊重するため、
					// 既存 state と last_activity_at で比較し、エントリごとに新しい方を採用する。
					const initialList = Array.isArray(initial) ? initial : [];
					setStatuses((prev) => {
						const next: Record<string, WorkspaceStatus> = { ...prev };
						for (const ws of initialList) {
							const current = next[ws.worktree_id];
							if (!current || current.last_activity_at <= ws.last_activity_at) {
								next[ws.worktree_id] = ws;
							}
						}
						return next;
					});
				}
			} catch {
				// listen が成功した後の invoke 失敗では listener が入れた最新 state を
				// 消さないために state を触らない。listen そのものが失敗した場合のみ
				// 空にリセットする。
				if (mounted && !subscribed) {
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
