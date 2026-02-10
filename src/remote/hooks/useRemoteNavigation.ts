import { useEffect, useState } from "react";
import type { Subscribe } from "./useMessageBus";
import type { DiffBase } from "./useRemoteFileContent";

export type Tab = "changes" | "diff" | "terminal" | "comments";

interface UseRemoteNavigationOptions {
	subscribe: Subscribe;
}

export function useRemoteNavigation({ subscribe }: UseRemoteNavigationOptions) {
	const [selectedPath, setSelectedPath] = useState<string | null>(null);
	const [selectedWorktree, setSelectedWorktree] = useState<string | null>(null);
	const [activeTab, setActiveTab] = useState<Tab>("changes");
	const [diffBase, setDiffBase] = useState<DiffBase>("HEAD");

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "worktree_select_response" && msg.payload.success) {
				setSelectedWorktree(msg.payload.path);
			}
		});
	}, [subscribe]);

	return {
		selectedPath,
		selectedWorktree,
		activeTab,
		diffBase,
		setSelectedPath,
		setSelectedWorktree,
		setActiveTab,
		setDiffBase,
	};
}
