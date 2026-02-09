import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { WorktreeEntry } from "@/types/git";

interface UseWorktreesReturn {
	worktrees: WorktreeEntry[];
	loading: boolean;
	refresh: () => Promise<void>;
	createWorktree: (params: {
		worktreePath: string;
		branch: string;
		createBranch: boolean;
		baseBranch?: string;
	}) => Promise<WorktreeEntry>;
	removeWorktree: (worktreePath: string, force: boolean) => Promise<void>;
}

export function useWorktrees(repoPath: string | null): UseWorktreesReturn {
	const [worktrees, setWorktrees] = useState<WorktreeEntry[]>([]);
	const [loading, setLoading] = useState(true);

	const refresh = useCallback(async () => {
		if (!repoPath) {
			setWorktrees([]);
			setLoading(false);
			return;
		}
		try {
			const entries = await invoke<WorktreeEntry[]>("list_worktrees", {
				repoPath,
			});
			setWorktrees(entries);
		} catch (e) {
			console.error("Failed to list worktrees:", e);
		} finally {
			setLoading(false);
		}
	}, [repoPath]);

	useEffect(() => {
		setLoading(true);
		refresh();
	}, [refresh]);

	useEffect(() => {
		if (!repoPath) return;
		const id = setInterval(() => {
			if (document.visibilityState === "visible") {
				refresh();
			}
		}, 5000);
		return () => clearInterval(id);
	}, [repoPath, refresh]);

	const createWorktree = useCallback(
		async (params: {
			worktreePath: string;
			branch: string;
			createBranch: boolean;
			baseBranch?: string;
		}) => {
			if (!repoPath) throw new Error("No repo path");
			const entry = await invoke<WorktreeEntry>("create_worktree", {
				repoPath,
				worktreePath: params.worktreePath,
				branch: params.branch,
				createBranch: params.createBranch,
				baseBranch: params.baseBranch ?? null,
			});
			await refresh();
			return entry;
		},
		[repoPath, refresh],
	);

	const removeWorktree = useCallback(
		async (worktreePath: string, force: boolean) => {
			if (!repoPath) throw new Error("No repo path");
			await invoke("remove_worktree", {
				repoPath,
				worktreePath,
				force,
			});
			await refresh();
		},
		[repoPath, refresh],
	);

	return { worktrees, loading, refresh, createWorktree, removeWorktree };
}
