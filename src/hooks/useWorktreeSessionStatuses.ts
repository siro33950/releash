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
			let subscribed = false;
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
				subscribed = true;

				if (!mounted) {
					unlisten?.();
					return;
				}

				const initial = await invoke<SessionStatus[]>("list_session_statuses");
				if (mounted) {
					// listen 登録後・invoke await 中に届いた最新イベントを尊重するため、
					// 既存 state と last_activity_at で比較し、エントリごとに新しい方を採用する。
					const initialList = Array.isArray(initial) ? initial : [];
					setStatuses((prev) => {
						const next = new Map(prev);
						for (const s of initialList) {
							if (s.worktree_id !== worktreePath) continue;
							const current = next.get(s.chat_session_id);
							if (!current || current.last_activity_at <= s.last_activity_at) {
								next.set(s.chat_session_id, s);
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
