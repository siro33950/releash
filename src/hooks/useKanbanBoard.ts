import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { BranchCard, PrStatus } from "@/types/git";
import type { AgentStateSync } from "@/types/protocol";

export function useKanbanBoard(repoPath: string | null) {
	const [branches, setBranches] = useState<BranchCard[]>([]);
	const [loading, setLoading] = useState(true);
	const [baseBranchLabel, setBaseBranchLabel] = useState<string>("");

	const { todo, inProgress, review, done } = useMemo(() => {
		const todo: BranchCard[] = [];
		const inProgress: BranchCard[] = [];
		const review: BranchCard[] = [];
		const done: BranchCard[] = [];
		for (const b of branches) {
			if (b.is_merged) {
				done.push(b);
			} else if (b.has_pr) {
				review.push(b);
			} else if (b.worktree_path != null) {
				inProgress.push(b);
			} else {
				todo.push(b);
			}
		}
		return { todo, inProgress, review, done };
	}, [branches]);

	const refreshBaseBranch = useCallback(async () => {
		if (!repoPath) {
			setBaseBranchLabel("");
			return;
		}
		try {
			const base = await invoke<string | null>("get_releash_base", {
				repoPath,
			});
			if (base) {
				setBaseBranchLabel(base);
			} else {
				const detected = await invoke<string>("get_default_branch", {
					repoPath,
				});
				setBaseBranchLabel(`${detected} (auto)`);
			}
		} catch {
			setBaseBranchLabel("");
		}
	}, [repoPath]);

	const enrichWithPrStatus = useCallback(
		async (cards: BranchCard[]): Promise<BranchCard[]> => {
			if (!repoPath) return cards;
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
		if (!repoPath) {
			setBranches([]);
			setLoading(false);
			return;
		}
		try {
			const cards = await invoke<BranchCard[]>("list_branches_with_status", {
				repoPath,
			});
			const enriched = await enrichWithPrStatus(cards);
			const agentStates = await invoke<Record<string, AgentStateSync>>(
				"get_agent_states",
			).catch((): Record<string, AgentStateSync> => ({}));
			setBranches(
				enriched.map((b) => {
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
				}),
			);
		} catch (e) {
			console.error("Failed to list branches:", e);
		} finally {
			setLoading(false);
		}
		refreshBaseBranch();
	}, [repoPath, refreshBaseBranch, enrichWithPrStatus]);

	useEffect(() => {
		setLoading(true);
		refresh();
	}, [refresh]);

	useEffect(() => {
		if (!repoPath) return;
		const unlisten = listen("branch-list-sync", () => {
			refresh();
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, [repoPath, refresh]);

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
		if (!repoPath) return;
		const id = setInterval(() => {
			if (document.visibilityState === "visible") {
				refresh();
			}
		}, 30000);
		return () => clearInterval(id);
	}, [repoPath, refresh]);

	return {
		branches,
		loading,
		baseBranchLabel,
		todo,
		inProgress,
		review,
		done,
		refresh,
		refreshBaseBranch,
	};
}
