import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { agentStateKey, aggregateAgentState } from "@/lib/agentStateUtils";
import { normalizePath } from "@/lib/normalizePath";
import type { PrStatus, WorktreeBranch } from "@/types/git";
import type { AgentStateSync } from "@/types/protocol";

const POLL_INTERVAL = 120_000;

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
	const refreshSeqRef = useRef(0);
	const prevBranchesRef = useRef("");
	const agentStatesRef = useRef<Map<string, AgentStateSync>>(new Map());

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

	const refresh = useCallback(
		async (options?: { silent?: boolean }) => {
			const seq = ++refreshSeqRef.current;
			if (!options?.silent) setLoading(true);
			try {
				const cards = await invoke<WorktreeBranch[]>(
					"list_branches_with_status",
					{
						repoPath,
					},
				);
				const enriched = await enrichWithPrStatus(cards);
				const agentStatesRecord = await invoke<Record<string, AgentStateSync>>(
					"get_agent_states",
				).catch((): Record<string, AgentStateSync> => ({}));
				const agentStatesMap = new Map(Object.entries(agentStatesRecord));
				agentStatesRef.current = agentStatesMap;

				const withAgentState = enriched.map((b) => {
					if (!b.worktree_path) return b;
					const bestState = aggregateAgentState(
						agentStatesMap,
						b.worktree_path,
					);
					return bestState ? { ...b, agent_state: bestState } : b;
				});
				const filtered = withAgentState.filter(
					(b) => b.worktree_path != null && !b.is_default,
				);
				if (seq === refreshSeqRef.current) {
					const serialized = JSON.stringify(filtered);
					if (serialized !== prevBranchesRef.current) {
						prevBranchesRef.current = serialized;
						setBranches(filtered);
					}
				}
			} catch (e) {
				console.error("Failed to list worktrees:", e);
			} finally {
				if (seq === refreshSeqRef.current) {
					setLoading(false);
				}
			}
		},
		[repoPath, enrichWithPrStatus],
	);

	useEffect(() => {
		refresh();
	}, [refresh]);

	const watcherIdRef = useRef<number | null>(null);

	useEffect(() => {
		let isMounted = true;
		const start = async () => {
			try {
				const id = await invoke<number>("start_git_dir_watching", {
					repoPath,
				});
				if (!isMounted) {
					invoke("stop_watching", { watcherId: id }).catch(() => {});
					return;
				}
				watcherIdRef.current = id;
			} catch (e) {
				console.error("Failed to start git dir watcher:", e);
			}
		};
		start();
		return () => {
			isMounted = false;
			if (watcherIdRef.current !== null) {
				invoke("stop_watching", { watcherId: watcherIdRef.current }).catch(
					() => {},
				);
				watcherIdRef.current = null;
			}
		};
	}, [repoPath]);

	useEffect(() => {
		const unlisten = listen("branch-list-sync", () => {
			refresh({ silent: true });
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, [refresh]);

	useEffect(() => {
		const unlisten = listen<AgentStateSync>("agent-state-changed", (event) => {
			const payload = event.payload;
			const key = agentStateKey(payload.worktree_path, payload.pty_id);
			agentStatesRef.current.set(key, payload);

			setBranches((prev) =>
				prev.map((b) => {
					if (!b.worktree_path) return b;
					if (
						normalizePath(b.worktree_path) !==
						normalizePath(payload.worktree_path)
					)
						return b;
					const bestState = aggregateAgentState(
						agentStatesRef.current,
						b.worktree_path,
					);
					return bestState ? { ...b, agent_state: bestState } : b;
				}),
			);
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, []);

	useEffect(() => {
		const id = setInterval(() => {
			if (document.visibilityState === "visible") {
				refresh({ silent: true });
			}
		}, POLL_INTERVAL);
		return () => clearInterval(id);
	}, [refresh]);

	return { branches, loading, refresh };
}
