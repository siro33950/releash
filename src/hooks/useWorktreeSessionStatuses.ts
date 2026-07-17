import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { SessionNotice, SessionStatus } from "@/types/session";

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
		const unlisteners: UnlistenFn[] = [];
		const pendingNotices = new Map<string, SessionNotice>();
		const knownSessionIds = new Set<string>();
		let loadingInitial = true;

		const mergePendingNotice = (
			status: SessionStatus,
			pendingNotice: SessionNotice | undefined,
		): SessionStatus => {
			if (!pendingNotice) return status;
			if (status.notice && status.notice.createdAt > pendingNotice.createdAt) {
				return status;
			}
			return { ...status, notice: pendingNotice };
		};

		const subscribe = async () => {
			let subscribed = false;
			try {
				const unlistenStatus = await listen<SessionStatus>(
					"session-status-changed",
					(event) => {
						if (!mounted) return;
						if (event.payload.worktree_id !== worktreePath) return;
						knownSessionIds.add(event.payload.chat_session_id);
						setStatuses((prev) => {
							const next = new Map(prev);
							next.set(event.payload.chat_session_id, event.payload);
							return next;
						});
						// A live backend status push is newer than any queued notice push and
						// can explicitly clear notice with `null`.
						pendingNotices.delete(event.payload.chat_session_id);
					},
				);
				unlisteners.push(unlistenStatus);
				subscribed = true;

				try {
					const unlistenNotice = await listen<SessionNotice>(
						"agent-session-notice",
						(event) => {
							if (!mounted) return;
							pendingNotices.set(event.payload.sessionId, event.payload);
							setStatuses((prev) => {
								const current = prev.get(event.payload.sessionId);
								if (!current) return prev;
								const next = new Map(prev);
								next.set(event.payload.sessionId, {
									...current,
									notice: event.payload,
								});
								return next;
							});
							if (
								!loadingInitial &&
								knownSessionIds.has(event.payload.sessionId)
							) {
								pendingNotices.delete(event.payload.sessionId);
							}
						},
					);
					unlisteners.push(unlistenNotice);
				} catch {
					// Snapshot remains authoritative if the transient push channel is unavailable.
				}

				if (!mounted) {
					for (const unlisten of unlisteners) unlisten();
					return;
				}

				const initial = await invoke<SessionStatus[]>("list_session_statuses");
				if (mounted) {
					// listen 登録後・invoke await 中に届いた最新イベントを尊重するため、
					// 既存 state と last_activity_at で比較し、エントリごとに新しい方を採用する。
					const initialList = Array.isArray(initial) ? initial : [];
					const pendingAtBootstrap = new Map(pendingNotices);
					for (const status of initialList) {
						if (status.worktree_id === worktreePath) {
							knownSessionIds.add(status.chat_session_id);
						}
					}
					setStatuses((prev) => {
						const next = new Map(prev);
						for (const s of initialList) {
							if (s.worktree_id !== worktreePath) continue;
							const current = next.get(s.chat_session_id);
							const newest =
								!current || current.last_activity_at < s.last_activity_at
									? s
									: current;
							next.set(
								s.chat_session_id,
								mergePendingNotice(
									newest,
									pendingAtBootstrap.get(s.chat_session_id),
								),
							);
						}
						return next;
					});
					loadingInitial = false;
					for (const sessionId of knownSessionIds) {
						pendingNotices.delete(sessionId);
					}
				}
			} catch {
				loadingInitial = false;
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
			for (const unlisten of unlisteners) unlisten();
		};
	}, [worktreePath]);

	return statuses;
}
