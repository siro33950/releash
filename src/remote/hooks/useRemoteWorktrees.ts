import { useCallback, useEffect, useState } from "react";
import type { WorktreeEntryMsg, WsMessage } from "@/types/protocol";

interface UseRemoteWorktreesParams {
	subscribe: (cb: (msg: WsMessage) => void) => () => void;
	send: (msg: WsMessage) => void;
	connected: boolean;
}

export function useRemoteWorktrees({
	subscribe,
	send,
	connected,
}: UseRemoteWorktreesParams) {
	const [worktrees, setWorktrees] = useState<WorktreeEntryMsg[]>([]);
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
				setWorktrees(msg.payload.worktrees);
				setLoading(false);
			}
		});
	}, [subscribe]);

	useEffect(() => {
		if (connected) {
			refresh();
		}
	}, [connected, refresh]);

	useEffect(() => {
		if (!connected) return;
		const id = setInterval(refresh, 10000);
		return () => clearInterval(id);
	}, [connected, refresh]);

	return { worktrees, loading, refresh };
}
