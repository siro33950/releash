import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import type { TerminalTabPanelHandle } from "@/components/panels/TerminalTabPanel";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitDirWatcher } from "@/hooks/useGitDirWatcher";
import { useThreads } from "@/hooks/useThreads";
import { useNativeFileDrop } from "@/hooks/useNativeFileDrop";
import { useWorkspaceStatus } from "@/hooks/useWorkspaceStatus";
import { useWorktreeThreads } from "@/screens/useWorktreeComments";
import {
	gitReducer,
	initialUIState,
	uiReducer,
	useWorktreeGitActions,
} from "@/screens/useWorktreeGitActions";
import { useWorktreeMenuHandlers } from "@/screens/useWorktreeMenuHandlers";
import type { AppSettings } from "@/types/settings";
import {
	type InternalWorktreeState,
	normalizeRightBottomActiveTab,
	type WorkspaceState,
} from "@/types/workspace-state";

export type { InternalWorktreeState } from "@/types/workspace-state";

interface UseWorktreeStateParams {
	rootPath: string;
	settings: AppSettings;
	onSettingsSave: (settings: AppSettings) => void;
	isActive: boolean;
	initialWorkspaceState?: WorkspaceState;
	internalStateMapRef?: React.MutableRefObject<
		Map<string, InternalWorktreeState>
	>;
}

export function useWorktreeState({
	rootPath,
	settings,
	onSettingsSave,
	isActive,
	initialWorkspaceState,
	internalStateMapRef,
}: UseWorktreeStateParams) {
	const [rightBottomCollapsed, setRightBottomCollapsed] = useState(
		initialWorkspaceState?.layout.rightBottomCollapsed ?? false,
	);

	const [rightBottomActiveTab, setRightBottomActiveTab] = useState<string>(
		normalizeRightBottomActiveTab(
			initialWorkspaceState?.layout.rightBottomActiveTab,
		),
	);

	const { branch } = useCurrentBranch(rootPath);
	const [ready, setReady] = useState(false);
	const {
		threads,
		removeThread,
		resolveThread,
		showResolvedThreads,
		toggleShowResolvedThreads,
	} = useThreads(rootPath);

	const { stage, unstage, createBranch } = useGitActions();
	const terminalRef = useRef<TerminalTabPanelHandle>(null);

	const [git, dispatchGit] = useReducer(gitReducer, {
		gitError: null,
		refreshKey: 0,
	});
	const { gitError, refreshKey: gitRefreshKey } = git;

	const refreshGit = useCallback(() => dispatchGit({ type: "REFRESH" }), []);

	const [ui, dispatchUI] = useReducer(uiReducer, initialUIState);
	const { isSettingsOpen, showCreateBranch, newBranchName } = ui;

	// --- Git dir watcher (index / refs / HEAD) ---
	useGitDirWatcher(rootPath);

	// --- Lifecycle effects ---
	useEffect(() => {
		if (branch != null) setReady(true);
	}, [branch]);

	// Rust 中央管理 (AgentStatusCenter) から worktree 集約状態を購読する。
	const workspaceStatus = useWorkspaceStatus(rootPath);
	const agentState = workspaceStatus?.aggregated_state;

	// --- Refs ---
	const settingsRef = useRef(settings);
	settingsRef.current = settings;

	const onSettingsSaveRef = useRef(onSettingsSave);
	onSettingsSaveRef.current = onSettingsSave;

	// --- Extracted hooks ---
	const gitActions = useWorktreeGitActions({
		rootPath,
		stage,
		unstage,
		createBranch,
		refreshGit,
		newBranchName,
		dispatchGit,
		dispatchUI,
	});

	useWorktreeMenuHandlers({
		dispatchUI,
		settingsRef,
		onSettingsSaveRef,
		gitActions,
		isActive,
	});

	const { handleSendToTerminal, handleThreadClick } = useWorktreeThreads({
		terminalRef,
		rootPath,
	});

	// --- Native file drop (image D&D to AgentChat) ---
	const { registerDropZone } = useNativeFileDrop({
		onDropToEditor: useCallback((_paths: string[]) => {}, []),
	});

	// --- Sync internal state for workspace state persistence ---
	useEffect(() => {
		if (!internalStateMapRef) return;
		internalStateMapRef.current.set(rootPath, {
			tabs: [],
			activeEditorPath: null,
			activeView: "agent",
			rightBottomCollapsed,
			rightBottomActiveTab,
		});
	}, [
		internalStateMapRef,
		rootPath,
		rightBottomCollapsed,
		rightBottomActiveTab,
	]);

	return {
		ready,
		threads,
		removeThread,
		resolveThread,
		showResolvedThreads,
		toggleShowResolvedThreads,
		gitError,
		isSettingsOpen,
		showCreateBranch,
		newBranchName,
		branch,
		agentState,
		terminalRef,
		dispatchUI,
		dispatchGit,
		gitActions,
		registerDropZone,
		handleSendToTerminal,
		handleThreadClick,
		refreshGit,
		gitRefreshKey,
		onSettingsSave,
		settings,
		rootPath,
		rightBottomCollapsed,
		setRightBottomCollapsed,
		rightBottomActiveTab,
		setRightBottomActiveTab,
	};
}
