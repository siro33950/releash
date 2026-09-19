import { useCallback, useEffect, useRef, useState } from "react";
import {
	invokeClient as invoke,
	listenClient as listen,
	watchClient,
} from "@/lib/client";
import type { WorktreeBranch } from "@/types/git";

const POLL_INTERVAL = 120_000;

export function useWorktreeList(repoPath: string) {
	const [branches, setBranches] = useState<WorktreeBranch[]>([]);
	const [loading, setLoading] = useState(true);
	const refreshSeqRef = useRef(0);
	const prevBranchesRef = useRef("");

	const enrichWithPrStatus = useCallback(
		async (cards: WorktreeBranch[]): Promise<WorktreeBranch[]> => {
			try {
				const prStatus = await invoke("get_cached_pr_status", {
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
				const snapshot = await invoke("list_branches_with_status_snapshot", {
					repoPath,
				});
				// 表示先の振り分けは backend が確定済み。ここでは PR 情報を重ねるだけ。
				const groups = snapshot.worktree_display_groups;
				const filtered = await enrichWithPrStatus(groups.working_areas);
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

	useEffect(
		() =>
			watchClient(
				"start_git_dir_watching",
				{ repoPath },
				() => {},
				(error) => console.error("Failed to start git dir watcher:", error),
			),
		[repoPath],
	);

	useEffect(() => {
		const reload = () => {
			void refresh({ silent: true });
		};
		const unlisten = listen("branch-list-sync", reload, reload);
		window.addEventListener("branch-list-refresh", reload);
		return () => {
			window.removeEventListener("branch-list-refresh", reload);
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
