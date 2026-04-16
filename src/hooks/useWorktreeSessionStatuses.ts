import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { SessionStatus } from "@/types/session";

/**
 * 指定 worktree に属する全 ChatSession の SessionStatus を Map で取得する。
 * Rust 中央管理から購読し、追加・更新・削除をリアルタイムに反映する。
 *
 * フロント側で AgentState を導出することは禁止。SessionStatus.agent_state を
 * そのまま消費すること。
 */
export function useWorktreeSessionStatuses(
	worktreePath: string | null,
): Map<string, SessionStatus> {
	const [statuses, setStatuses] = useState<Map<string, SessionStatus>>(
		new Map(),
	);

	useEffect(() => {
		if (!worktreePath) {
			setStatuses(new Map());
			return;
		}

		let mounted = true;
		let unlisten: UnlistenFn | null = null;

		const subscribe = async () => {
			try {
				unlisten = await listen<SessionStatus>(
					"session-status-changed",
					(event) => {
						if (!mounted) return;
						if (event.payload.worktree_id !== worktreePath) return;
						setStatuses((prev) => {
							const next = new Map(prev);
							next.set(event.payload.chat_session_id, event.payload);
							return next;
						});
					},
				);

				if (!mounted) {
					unlisten?.();
					return;
				}

				const initial = await invoke<SessionStatus[]>("list_session_statuses");
				if (mounted) {
					const map = new Map<string, SessionStatus>();
					for (const s of initial) {
						if (s.worktree_id === worktreePath) {
							map.set(s.chat_session_id, s);
						}
					}
					setStatuses(map);
				}
			} catch {
				if (mounted) {
					setStatuses(new Map());
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
