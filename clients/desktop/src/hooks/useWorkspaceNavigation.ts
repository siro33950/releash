import { useCallback, useState } from "react";
import type { WorktreeTab } from "@/types/workspace-tab";

function fallbackBranchName(rootPath: string): string {
	return rootPath.split("/").filter(Boolean).pop() ?? rootPath;
}

export interface UseWorkspaceNavigationReturn {
	worktrees: WorktreeTab[];
	selectedWorktreeId: string | null;
	openWorktreeTab: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
	) => void;
	closeWorktreeTab: (id: string) => void;
	setSelectedWorktree: (id: string | null) => void;
}

export function useWorkspaceNavigation(): UseWorkspaceNavigationReturn {
	const [worktrees, setWorktrees] = useState<WorktreeTab[]>([]);
	const [selectedWorktreeId, setSelectedWorktreeId] = useState<string | null>(
		null,
	);

	const openWorktreeTab = useCallback(
		(rootPath: string, branchName?: string, repoName?: string) => {
			setWorktrees((prev) => {
				const existing = prev.find((t) => t.rootPath === rootPath);
				if (existing) {
					setSelectedWorktreeId(existing.id);
					return prev;
				}
				const newTab: WorktreeTab = {
					type: "worktree",
					id: rootPath,
					rootPath,
					branchName: branchName ?? fallbackBranchName(rootPath),
					repoName,
				};
				setSelectedWorktreeId(newTab.id);
				return [...prev, newTab];
			});
		},
		[],
	);

	const closeWorktreeTab = useCallback((id: string) => {
		setWorktrees((prev) => {
			const idx = prev.findIndex((t) => t.id === id);
			if (idx === -1) return prev;
			const next = prev.filter((t) => t.id !== id);
			setSelectedWorktreeId((currentSelected) => {
				if (currentSelected !== id) return currentSelected;
				const fallback = next[Math.min(idx, next.length - 1)];
				return fallback?.id ?? null;
			});
			return next;
		});
	}, []);

	const setSelectedWorktree = useCallback((id: string | null) => {
		setSelectedWorktreeId(id);
	}, []);

	return {
		worktrees,
		selectedWorktreeId,
		openWorktreeTab,
		closeWorktreeTab,
		setSelectedWorktree,
	};
}
