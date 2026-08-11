import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import type { PrStatus, WorktreeBranch } from "@/types/git";

const POLL_INTERVAL = 120_000;

interface BranchCardsSnapshot {
	version: number;
	stale: boolean;
	loading: boolean;
	limited: boolean;
	branches: WorktreeBranch[];
}

export function useWorktreeList(repoPath: string) {
	const [branches, setBranches] = useState<WorktreeBranch[]>([]);
	const [loading, setLoading] = useState(true);
	const refreshSeqRef = useRef(0);
	const prevBranchesRef = useRef("");

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
				const snapshot = await invoke<BranchCardsSnapshot>(
					"list_branches_with_status_snapshot",
					{
						repoPath,
					},
				);
				const enriched = await enrichWithPrStatus(snapshot.branches);
				const filtered = enriched.filter((b) => b.worktree_path != null);
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
		const id = setInterval(() => {
			if (document.visibilityState === "visible") {
				refresh({ silent: true });
			}
		}, POLL_INTERVAL);
		return () => clearInterval(id);
	}, [refresh]);

	return { branches, loading, refresh };
}
