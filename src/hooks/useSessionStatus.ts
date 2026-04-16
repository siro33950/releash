import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { SessionStatus } from "@/types/session";

/**
 * 指定 ChatSession の SessionStatus を Rust 中央管理から購読する。
 * - マウント時に `get_session_status` で初期値取得
 * - `session-status-changed` イベントを listen し、該当 chat_session_id だけ反映
 *
 * フロント側で状態の派生・加工は一切行わず、Rust が算出した SessionStatus を
 * そのまま表示する。
 */
export function useSessionStatus(
	chatSessionId: string | null,
): SessionStatus | null {
	const [status, setStatus] = useState<SessionStatus | null>(null);

	useEffect(() => {
		if (!chatSessionId) {
			setStatus(null);
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
						if (event.payload.chat_session_id === chatSessionId) {
							setStatus(event.payload);
						}
					},
				);
				subscribed = true;

				if (!mounted) {
					unlisten?.();
					return;
				}

				const initial = await invoke<SessionStatus | null>(
					"get_session_status",
					{ chatSessionId },
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
	}, [chatSessionId]);

	return status;
}
