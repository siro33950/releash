import { useCallback, useEffect, useState } from "react";
import type { Subscribe } from "./useMessageBus";

export type Tab = "terminal" | "agent" | "comments" | "threads";

interface UseRemoteNavigationOptions {
	subscribe: Subscribe;
}

export function useRemoteNavigation({ subscribe }: UseRemoteNavigationOptions) {
	const [selectedWorktree, setSelectedWorktree] = useState<string | null>(null);
	const [activeTab, setActiveTab] = useState<Tab>("terminal");
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
		selectedWorktree,
		worktreeLoading,
		activeTab,
		setSelectedWorktree,
		setActiveTab,
		selectWorktreeOptimistic,
	};
}
