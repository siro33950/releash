import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { PrStatus, WorktreeBranch } from "@/types/git";
import type { AgentStateSync } from "@/types/protocol";

const POLL_INTERVAL = 30_000;

export type WorktreeStatus = "backlog" | "in_progress" | "review" | "done";

export function computeStatus(branch: WorktreeBranch): WorktreeStatus {
	if (branch.is_merged) return "done";
	if (branch.has_pr) return "review";
	if (branch.base_ahead > 0 || branch.dirty_count > 0) return "in_progress";
	return "backlog";
}

export function useWorktreeList(repoPath: string) {
	const [branches, setBranches] = useState<WorktreeBranch[]>([]);
	const [loading, setLoading] = useState(true);

	const enrichWithPrStatus = useCallback(
		async (cards: WorktreeBranch[]): Promise<WorktreeBranch[]> => {
			try {
				const prStatus = await invoke<PrStatus>("get_cached_pr_status", {
					repoPath,
				});
				return cards.map((b) => {
					const pr = prStatus.open_prs[b.name];
					const isMergedViaPr = prStatus.merged_branches.includes(b.name);
					if (pr) {
						return {
							...b,
							has_pr: true,
							pr_number: pr.number,
							pr_url: pr.url,
						};
					}
					if (isMergedViaPr && !b.is_merged) {
						return { ...b, is_merged: true };
					}
					return b;
				});
			} catch {
				return cards;
			}
		},
		[repoPath],
	);

	const refresh = useCallback(async () => {
		try {
			const cards = await invoke<WorktreeBranch[]>(
				"list_branches_with_status",
				{
					repoPath,
				},
			);
			const enriched = await enrichWithPrStatus(cards);
			const agentStates = await invoke<Record<string, AgentStateSync>>(
				"get_agent_states",
			).catch((): Record<string, AgentStateSync> => ({}));

			const withAgentState = enriched.map((b) => {
				const agent = b.worktree_path
					? agentStates[b.worktree_path]
					: undefined;
				return agent
					? {
							...b,
							agent_state: agent.state,
							agent_state_timestamp: agent.timestamp,
						}
					: b;
			});
			const filtered = withAgentState.filter(
				(b) => b.worktree_path != null && !b.is_default,
			);
			setBranches(filtered);
		} catch (e) {
			console.error("Failed to list worktrees:", e);
		} finally {
			setLoading(false);
		}
	}, [repoPath, enrichWithPrStatus]);

	useEffect(() => {
		setLoading(true);
		refresh();
	}, [refresh]);

	useEffect(() => {
		const unlisten = listen("branch-list-sync", () => {
			refresh();
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, [refresh]);

	useEffect(() => {
		const unlisten = listen<AgentStateSync>("agent-state-changed", (event) => {
			const { worktree_path, state, timestamp } = event.payload;
			setBranches((prev) =>
				prev.map((b) =>
					b.worktree_path === worktree_path
						? { ...b, agent_state: state, agent_state_timestamp: timestamp }
						: b,
				),
			);
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, []);

	useEffect(() => {
		const id = setInterval(() => {
			if (document.visibilityState === "visible") {
				refresh();
			}
		}, POLL_INTERVAL);
		return () => clearInterval(id);
	}, [refresh]);

	return { branches, loading, refresh };
}
