import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { WorkspaceStatus } from "@/types/session";

/**
 * 指定 worktree の集約済み WorkspaceStatus を Rust 中央管理から購読する。
 * - マウント時に `get_workspace_status` で初期値取得
 * - `workspace-status-changed` イベントを listen し、該当 worktree_id だけ反映
 */
export function useWorkspaceStatus(
	worktreeId: string | null,
): WorkspaceStatus | null {
	const [status, setStatus] = useState<WorkspaceStatus | null>(null);

	useEffect(() => {
		if (!worktreeId) {
			setStatus(null);
			return;
		}

		let mounted = true;
		let unlisten: UnlistenFn | null = null;

		const subscribe = async () => {
			let subscribed = false;
			try {
				unlisten = await listen<WorkspaceStatus>(
					"workspace-status-changed",
					(event) => {
						if (!mounted) return;
						if (event.payload.worktree_id === worktreeId) {
							setStatus(event.payload);
						}
					},
				);
				subscribed = true;

				if (!mounted) {
					unlisten?.();
					return;
				}

				const initial = await invoke<WorkspaceStatus | null>(
					"get_workspace_status",
					{ worktreeId },
				);
				if (mounted) {
					// listen 登録後・invoke await 中に届いた最新イベントを尊重するため、
					// last_activity_at で新旧比較し、新しい方だけを state に反映する。
					setStatus((prev) => {
						if (!prev) return initial ?? null;
						if (!initial) return prev;
						return prev.last_activity_at > initial.last_activity_at
							? prev
							: initial;
					});
				}
			} catch {
				// listen が成功した後の invoke 失敗では listener が入れた最新 state を
				// 消さないために state を触らない。listen そのものが失敗した場合のみ
				// null に戻す。
				if (mounted && !subscribed) {
					setStatus(null);
				}
			}
		};

		subscribe();

		return () => {
			mounted = false;
			unlisten?.();
		};
	}, [worktreeId]);

	return status;
}
