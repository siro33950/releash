import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useGitActions } from "@/hooks/useGitActions";
import { useGitDirWatcher } from "@/hooks/useGitDirWatcher";
import { useNativeFileDrop } from "@/hooks/useNativeFileDrop";
import { useWorkspaceStatus } from "@/hooks/useWorkspaceStatus";
import {
	gitReducer,
	initialUIState,
	uiReducer,
	useWorktreeGitActions,
} from "@/screens/useWorktreeGitActions";
import { useWorktreeMenuHandlers } from "@/screens/useWorktreeMenuHandlers";
import type { AppSettings } from "@/types/settings";
import type {
	InternalWorktreeState,
	WorkspaceState,
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

	const [reviewCollapsed, setReviewCollapsed] = useState(
		initialWorkspaceState?.layout.reviewCollapsed ?? false,
	);

	const [diffOnlyMode, setDiffOnlyMode] = useState(
		initialWorkspaceState?.layout.diffOnlyMode ??
			settings.defaultDiffOnlyMode ??
			false,
	);

	const [selectedDiffFile, setSelectedDiffFile] = useState<string | null>(
		initialWorkspaceState?.layout.selectedDiffFile ?? null,
	);

	const { branch } = useCurrentBranch(rootPath);
	const [ready, setReady] = useState(false);

	const { stage, unstage, createBranch } = useGitActions();
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

	// --- Native file drop (image D&D to AgentChat) ---
	const { registerDropZone } = useNativeFileDrop({
		onDropToEditor: useCallback((_paths: string[]) => {}, []),
	});

	// --- Sync internal state for workspace state persistence ---
	useEffect(() => {
		if (!internalStateMapRef) return;
		const current = internalStateMapRef.current.get(rootPath);
		internalStateMapRef.current.set(rootPath, {
			tabs: current?.tabs ?? [],
			activeEditorPath: current?.activeEditorPath ?? null,
			activeView: current?.activeView ?? "agent",
			rightBottomCollapsed,
			reviewCollapsed,
			diffOnlyMode,
			selectedDiffFile,
		});
	}, [
		internalStateMapRef,
		rootPath,
		rightBottomCollapsed,
		reviewCollapsed,
		diffOnlyMode,
		selectedDiffFile,
	]);

	return {
		ready,
		gitError,
		isSettingsOpen,
		showCreateBranch,
		newBranchName,
		branch,
		agentState,
		dispatchUI,
		dispatchGit,
		gitActions,
		registerDropZone,
		refreshGit,
		gitRefreshKey,
		onSettingsSave,
		settings,
		rootPath,
		rightBottomCollapsed,
		setRightBottomCollapsed,
		reviewCollapsed,
		setReviewCollapsed,
		diffOnlyMode,
		setDiffOnlyMode,
		selectedDiffFile,
		setSelectedDiffFile,
	};
}
