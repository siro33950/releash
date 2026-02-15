import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { arrayMove } from "@/lib/arrayMove";
import { normalizePath } from "@/lib/normalizePath";
import type { AgentStateSync } from "@/types/protocol";
import type { WorkspaceTab, WorktreeTab } from "@/types/workspace-tab";

const KANBAN_TAB: WorkspaceTab = { type: "kanban", id: "kanban" };

function fallbackBranchName(rootPath: string): string {
	return rootPath.split("/").filter(Boolean).pop() ?? rootPath;
}

export interface UseWorkspaceTabsReturn {
	tabs: WorkspaceTab[];
	activeTabId: string;
	openWorktreeTab: (
		rootPath: string,
		branchName?: string,
		repoName?: string,
	) => void;
	closeWorktreeTab: (id: string) => void;
	setActiveTab: (id: string) => void;
	switchToKanban: () => void;
	reorderTabs: (fromId: string, toId: string) => void;
}

export function useWorkspaceTabs(): UseWorkspaceTabsReturn {
	const [tabs, setTabs] = useState<WorkspaceTab[]>([KANBAN_TAB]);
	const [activeTabId, setActiveTabId] = useState<string>("kanban");

	const openWorktreeTab = useCallback(
		(rootPath: string, branchName?: string, repoName?: string) => {
			const normalized = normalizePath(rootPath);
			setTabs((prev) => {
				const existing = prev.find(
					(t) => t.type === "worktree" && t.rootPath === normalized,
				);
				if (existing) {
					setActiveTabId(existing.id);
					return prev;
				}
				const newTab: WorktreeTab = {
					type: "worktree",
					id: normalized,
					rootPath: normalized,
					branchName: branchName ?? fallbackBranchName(normalized),
					repoName,
				};
				setActiveTabId(newTab.id);
				return [...prev, newTab];
			});
		},
		[],
	);

	const closeWorktreeTab = useCallback((id: string) => {
		if (id === "kanban") return;
		setTabs((prev) => {
			const idx = prev.findIndex((t) => t.id === id);
			if (idx === -1) return prev;
			const next = prev.filter((t) => t.id !== id);
			setActiveTabId((currentActive) => {
				if (currentActive !== id) return currentActive;
				const fallback = next[Math.min(idx, next.length - 1)];
				return fallback?.id ?? "kanban";
			});
			return next;
		});
	}, []);

	const setActiveTab = useCallback((id: string) => {
		setActiveTabId(id);
	}, []);

	const switchToKanban = useCallback(() => {
		setActiveTabId("kanban");
	}, []);

	const reorderTabs = useCallback((fromId: string, toId: string) => {
		if (fromId === toId) return;
		setTabs((prev) => {
			const fromIndex = prev.findIndex((t) => t.id === fromId);
			const toIndex = prev.findIndex((t) => t.id === toId);
			if (fromIndex === -1 || toIndex === -1) return prev;
			return arrayMove(prev, fromIndex, toIndex);
		});
	}, []);

	useEffect(() => {
		const unlisten = listen<AgentStateSync>("agent-state-changed", (event) => {
			const worktreePath = normalizePath(event.payload.worktree_path);
			const { state } = event.payload;
			setTabs((prev) =>
				prev.map((t) =>
					t.type === "worktree" && t.rootPath === worktreePath
						? { ...t, agentState: state }
						: t,
				),
			);
		});
		return () => {
			unlisten.then((f) => f());
		};
	}, []);

	return {
		tabs,
		activeTabId,
		openWorktreeTab,
		closeWorktreeTab,
		setActiveTab,
		switchToKanban,
		reorderTabs,
	};
}
