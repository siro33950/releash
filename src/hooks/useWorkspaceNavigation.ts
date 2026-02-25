import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { normalizePath } from "@/lib/normalizePath";
import type { AgentStateSync } from "@/types/protocol";
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
			const normalized = normalizePath(rootPath);
			setWorktrees((prev) => {
				const existing = prev.find((t) => t.rootPath === normalized);
				if (existing) {
					setSelectedWorktreeId(existing.id);
					return prev;
				}
				const newTab: WorktreeTab = {
					type: "worktree",
					id: normalized,
					rootPath: normalized,
					branchName: branchName ?? fallbackBranchName(normalized),
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

	useEffect(() => {
		let unlisten: UnlistenFn | null = null;

		const setupListener = async () => {
			try {
				unlisten = await listen<AgentStateSync>(
					"agent-state-changed",
					(event) => {
						const worktreePath = normalizePath(event.payload.worktree_path);
						const { state } = event.payload;
						setWorktrees((prev) =>
							prev.map((t) =>
								t.rootPath === worktreePath ? { ...t, agentState: state } : t,
							),
						);
					},
				);
			} catch (err) {
				console.warn("agent-state-changed listener setup failed:", err);
			}
		};

		setupListener();

		return () => {
			unlisten?.();
		};
	}, []);

	return {
		worktrees,
		selectedWorktreeId,
		openWorktreeTab,
		closeWorktreeTab,
		setSelectedWorktree,
	};
}
