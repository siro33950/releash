import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { subscribeAgentSessionChanged } from "@/lib/agentSessionEvents";
import type {
	AgentSessionItem,
	AgentSessionListPage,
} from "@/types/agent-session";

export function useAgentSessions(
	workspaceIdentity: string | null,
	lifecycle?: AgentSessionItem["lifecycle"],
) {
	const [items, setItems] = useState<AgentSessionItem[]>([]);
	const [loading, setLoading] = useState(workspaceIdentity != null);
	const [loadingMore, setLoadingMore] = useState(false);
	const [nextAfterSessionId, setNextAfterSessionId] = useState<string | null>(
		null,
	);
	const [error, setError] = useState<string | null>(null);

	const refresh = useCallback(async () => {
		if (!workspaceIdentity) {
			setItems([]);
			setNextAfterSessionId(null);
			setLoading(false);
			setError(null);
			return;
		}
		setLoading(true);
		try {
			const page = await invoke<AgentSessionListPage>("list_agent_sessions", {
				workspaceIdentity,
				...(lifecycle ? { lifecycle } : {}),
				origin: "standalone",
				limit: 100,
			});
			setItems(page?.items ?? []);
			setNextAfterSessionId(page?.nextAfterSessionId ?? null);
			setError(null);
		} catch (cause) {
			setNextAfterSessionId(null);
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			setLoading(false);
		}
	}, [lifecycle, workspaceIdentity]);

	const loadMore = useCallback(async () => {
		if (!workspaceIdentity || !nextAfterSessionId || loadingMore) return;
		setLoadingMore(true);
		try {
			const page = await invoke<AgentSessionListPage>("list_agent_sessions", {
				workspaceIdentity,
				...(lifecycle ? { lifecycle } : {}),
				origin: "standalone",
				limit: 100,
				afterSessionId: nextAfterSessionId,
			});
			setItems((current) => [...current, ...(page?.items ?? [])]);
			setNextAfterSessionId(page?.nextAfterSessionId ?? null);
			setError(null);
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			setLoadingMore(false);
		}
	}, [lifecycle, loadingMore, nextAfterSessionId, workspaceIdentity]);

	useEffect(() => {
		void refresh();
		return subscribeAgentSessionChanged(({ worktreePath }) => {
			if (!worktreePath || worktreePath === workspaceIdentity) void refresh();
		});
	}, [refresh, workspaceIdentity]);

	return {
		items,
		loading,
		loadingMore,
		hasMore: nextAfterSessionId != null,
		error,
		refresh,
		loadMore,
	};
}
