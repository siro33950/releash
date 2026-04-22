import { useCallback } from "react";
import type { Tab } from "./useRemoteNavigation";

interface UseRemoteAppActionsParams {
	disconnect: () => void;
	setConnection: (value: { url: string; token: string } | null) => void;

	setSelectedWorktree: (worktree: string | null) => void;
	setActiveTab: (tab: Tab) => void;
	setTerminalMounted: (mounted: boolean) => void;
	setBranchName: (name: string | null) => void;

	selectWorktreeOptimistic: (path: string) => void;
	selectWorktree: (path: string) => void;
	resetPty: () => void;
}

export function useRemoteAppActions({
	disconnect,
	setConnection,
	setSelectedWorktree,
	setActiveTab,
	setTerminalMounted,
	setBranchName,
	selectWorktreeOptimistic,
	selectWorktree,
	resetPty,
}: UseRemoteAppActionsParams) {
	const handleSelectWorktree = useCallback(
		(worktreePath: string) => {
			selectWorktreeOptimistic(worktreePath);
			selectWorktree(worktreePath);
			setBranchName(null);
			resetPty();
			setActiveTab("terminal");
			setTerminalMounted(true);
		},
		[
			selectWorktreeOptimistic,
			selectWorktree,
			setBranchName,
			resetPty,
			setActiveTab,
			setTerminalMounted,
		],
	);

	const handleBackToWorktreesAction = useCallback(() => {
		setSelectedWorktree(null);
		setBranchName(null);
	}, [setSelectedWorktree, setBranchName]);

	const handleConnect = useCallback(
		(wsUrl: string, token: string) => {
			setConnection({ url: wsUrl, token });
		},
		[setConnection],
	);

	const handleDisconnect = useCallback(() => {
		disconnect();
		setConnection(null);
		resetPty();
	}, [disconnect, setConnection, resetPty]);

	const handleTabChange = useCallback(
		(tab: Tab) => {
			setActiveTab(tab);
			if (tab === "terminal") setTerminalMounted(true);
		},
		[setActiveTab, setTerminalMounted],
	);

	return {
		handleSelectWorktree,
		handleBackToWorktreesAction,
		handleConnect,
		handleDisconnect,
		handleTabChange,
	};
}
