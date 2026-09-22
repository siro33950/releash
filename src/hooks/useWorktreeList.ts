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
	const prevBranchesRef = useRef({ repoPath, cards: branches });

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
			const applyBranches = (
				cards: WorktreeBranch[],
				preservePrDisplay = false,
			) => {
				if (seq !== refreshSeqRef.current) return;
				const previous = prevBranchesRef.current;
				if (preservePrDisplay && previous.repoPath === repoPath) {
					const displayed = new Map(previous.cards.map((b) => [b.name, b]));
					cards = cards.map((card) => {
						const prior = displayed.get(card.name);
						if (!prior || prior.worktree_path !== card.worktree_path)
							return card;
						return {
							...card,
							has_pr: prior.has_pr,
							pr_number: prior.pr_number,
							pr_url: prior.pr_url,
						};
					});
				}
				prevBranchesRef.current = { repoPath, cards };
				if (JSON.stringify(cards) !== JSON.stringify(previous.cards)) {
					setBranches(cards);
				}
				setLoading(false);
			};
			try {
				const snapshot = await invoke("list_branches_with_status_snapshot", {
					repoPath,
				});
				// 表示先の振り分けは backend が確定済み。ここでは PR 情報を重ねるだけ。
				const groups = snapshot.worktree_display_groups;
				applyBranches(groups.working_areas, true);
				applyBranches(await enrichWithPrStatus(groups.working_areas));
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

	const pollInterval = branches.some((branch) => branch.is_deleting)
		? 1_000
		: POLL_INTERVAL;
	useEffect(() => {
		const id = setInterval(() => {
			if (document.visibilityState === "visible") {
				refresh({ silent: true });
			}
		}, pollInterval);
		return () => clearInterval(id);
	}, [refresh, pollInterval]);

	return { branches, loading, refresh };
}
