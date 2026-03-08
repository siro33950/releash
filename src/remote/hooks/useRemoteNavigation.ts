import { useCallback, useEffect, useState } from "react";
import type { Subscribe } from "./useMessageBus";
import type { DiffBase } from "./useRemoteFileContent";

export type Tab =
	| "changes"
	| "diff"
	| "terminal"
	| "agent"
	| "comments"
	| "threads";

interface UseRemoteNavigationOptions {
	subscribe: Subscribe;
}

export function useRemoteNavigation({ subscribe }: UseRemoteNavigationOptions) {
	const [selectedPath, setSelectedPath] = useState<string | null>(null);
	const [selectedWorktree, setSelectedWorktree] = useState<string | null>(null);
	const [activeTab, setActiveTab] = useState<Tab>("agent");
	const [diffBase, setDiffBase] = useState<DiffBase>("branch-base");
	const [worktreeLoading, setWorktreeLoading] = useState(false);

	const selectWorktreeOptimistic = useCallback((path: string) => {
		setSelectedWorktree(path);
		setWorktreeLoading(true);
	}, []);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "worktree_select_response") {
				if (msg.payload.success) {
					setWorktreeLoading(false);
				} else {
					setSelectedWorktree(null);
					setWorktreeLoading(false);
				}
			}
		});
	}, [subscribe]);

	return {
		selectedPath,
		selectedWorktree,
		worktreeLoading,
		activeTab,
		diffBase,
		setSelectedPath,
		setSelectedWorktree,
		setActiveTab,
		setDiffBase,
		selectWorktreeOptimistic,
	};
}
