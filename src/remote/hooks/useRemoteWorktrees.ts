import { useCallback, useEffect, useMemo, useState } from "react";
import type {
	WorktreeEntryMsg,
	WorktreePrEntry,
	WsMessage,
} from "@/types/protocol";

interface UseRemoteWorktreesParams {
	subscribe: (cb: (msg: WsMessage) => void) => () => void;
	send: (msg: WsMessage) => void;
	connected: boolean;
}

/// PR ステータスは worktree 一覧の後に別メッセージで届くため、worktree 行に
/// 後付けでマージした表示用の型。
export type RemoteWorktree = WorktreeEntryMsg & {
	has_pr?: boolean;
	pr_number?: number;
	pr_url?: string;
};

export function useRemoteWorktrees({
	subscribe,
	send,
	connected,
}: UseRemoteWorktreesParams) {
	const [baseWorktrees, setBaseWorktrees] = useState<WorktreeEntryMsg[]>([]);
	// path → PR ステータス。worktree 一覧の後追い配信で更新する。
	const [prByPath, setPrByPath] = useState<Record<string, WorktreePrEntry>>({});
	const [loading, setLoading] = useState(false);

	const refresh = useCallback(() => {
		if (!connected) return;
		setLoading(true);
		send({
			type: "worktree_list_request",
			payload: {},
		});
	}, [send, connected]);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "worktree_list_response") {
				setBaseWorktrees(msg.payload.worktrees);
				// 新しい一覧に存在しなくなった worktree の PR のみ整理する。
				// 既存 PR は保持して後追いの worktree_pr_status_sync で差し替えるため、
				// 一覧返却 → PR 同期の間にバッジが一時的に消えるちらつきを防ぐ。
				const livePaths = new Set(msg.payload.worktrees.map((wt) => wt.path));
				setPrByPath((prev) => {
					const next: Record<string, WorktreePrEntry> = {};
					for (const [path, entry] of Object.entries(prev)) {
						if (livePaths.has(path)) {
							next[path] = entry;
						}
					}
					return next;
				});
				setLoading(false);
			}
			if (msg.type === "worktree_pr_status_sync") {
				const next: Record<string, WorktreePrEntry> = {};
				for (const e of msg.payload.entries) {
					next[e.path] = e;
				}
				setPrByPath(next);
			}
			if (msg.type === "branch_list_sync") {
				refresh();
			}
		});
	}, [subscribe, refresh]);

	const worktrees = useMemo<RemoteWorktree[]>(
		() =>
			baseWorktrees.map((wt) => {
				const pr = prByPath[wt.path];
				return pr
					? {
							...wt,
							has_pr: true,
							pr_number: pr.pr_number,
							pr_url: pr.pr_url,
						}
					: wt;
			}),
		[baseWorktrees, prByPath],
	);

	useEffect(() => {
		if (connected) {
			refresh();
		} else {
			setLoading(false);
		}
	}, [connected, refresh]);

	useEffect(() => {
		if (!connected) return;
		const id = setInterval(refresh, 30000);
		return () => clearInterval(id);
	}, [connected, refresh]);

	const select = useCallback(
		(worktreePath: string) => {
			send({
				type: "worktree_select_request",
				payload: { path: worktreePath },
			});
		},
		[send],
	);

	return { worktrees, loading, refresh, select };
}
