import { invoke } from "@tauri-apps/api/core";
import { useCallback } from "react";
import type { DiffBase, DiffMode } from "@/types/settings";

// Re-export reducer types used by this hook
export type GitAction =
	| { type: "SET_DIFF_BASE"; value: DiffBase }
	| { type: "SET_DIFF_MODE"; value: DiffMode }
	| { type: "SET_GIT_ERROR"; error: string | null }
	| { type: "REFRESH" };

export interface GitState {
	diffBase: DiffBase;
	diffMode: DiffMode;
	gitError: string | null;
	refreshKey: number;
}

export function gitReducer(state: GitState, action: GitAction): GitState {
	switch (action.type) {
		case "SET_DIFF_BASE":
			return { ...state, diffBase: action.value };
		case "SET_DIFF_MODE":
			return { ...state, diffMode: action.value };
		case "SET_GIT_ERROR":
			return { ...state, gitError: action.error };
		case "REFRESH":
			return { ...state, refreshKey: state.refreshKey + 1 };
	}
}

export type UIAction =
	| { type: "SET_SETTINGS_OPEN"; open: boolean }
	| { type: "SET_CLOSING_TAB"; path: string | null }
	| { type: "SET_SAVING_CONFLICT"; path: string | null }
	| { type: "SET_DISCARD_CONFIRM"; show: boolean }
	| { type: "OPEN_CREATE_BRANCH" }
	| { type: "CLOSE_CREATE_BRANCH" }
	| { type: "SET_NEW_BRANCH_NAME"; name: string }
	| { type: "SET_EDITOR_DRAG_OVER"; value: boolean };

export interface UIState {
	isSettingsOpen: boolean;
	closingTabPath: string | null;
	savingConflictPath: string | null;
	showDiscardConfirm: boolean;
	showCreateBranch: boolean;
	newBranchName: string;
	editorDragOver: boolean;
}

export const initialUIState: UIState = {
	isSettingsOpen: false,
	closingTabPath: null,
	savingConflictPath: null,
	showDiscardConfirm: false,
	showCreateBranch: false,
	newBranchName: "",
	editorDragOver: false,
};

export function uiReducer(state: UIState, action: UIAction): UIState {
	switch (action.type) {
		case "SET_SETTINGS_OPEN":
			return { ...state, isSettingsOpen: action.open };
		case "SET_CLOSING_TAB":
			return { ...state, closingTabPath: action.path };
		case "SET_SAVING_CONFLICT":
			return { ...state, savingConflictPath: action.path };
		case "SET_DISCARD_CONFIRM":
			return { ...state, showDiscardConfirm: action.show };
		case "OPEN_CREATE_BRANCH":
			return { ...state, showCreateBranch: true, newBranchName: "" };
		case "CLOSE_CREATE_BRANCH":
			return { ...state, showCreateBranch: false };
		case "SET_NEW_BRANCH_NAME":
			return { ...state, newBranchName: action.name };
		case "SET_EDITOR_DRAG_OVER":
			return { ...state, editorDragOver: action.value };
	}
}

export type EditorAction =
	| { type: "SET_ACTIVE_VIEW"; view: string }
	| { type: "TRIGGER_SEARCH"; query: string }
	| {
			type: "SET_PENDING_REVEAL";
			reveal: { path: string; line: number; openThread?: boolean } | null;
	  }
	| { type: "INCREMENT_NEW_FOLDER" };

export interface EditorState {
	activeView: string;
	searchFocusKey: number;
	searchInitialQuery: string;
	pendingReveal: { path: string; line: number; openThread?: boolean } | null;
	newFolderKey: number;
}

const defaultEditorState: EditorState = {
	activeView: "git",
	searchFocusKey: 0,
	searchInitialQuery: "",
	pendingReveal: null,
	newFolderKey: 0,
};

export const initialEditorState: EditorState = defaultEditorState;

export function createEditorState(
	overrides?: Partial<Pick<EditorState, "activeView">>,
): EditorState {
	return { ...defaultEditorState, ...overrides };
}

export function editorReducer(
	state: EditorState,
	action: EditorAction,
): EditorState {
	switch (action.type) {
		case "SET_ACTIVE_VIEW":
			return { ...state, activeView: action.view };
		case "TRIGGER_SEARCH":
			return {
				...state,
				activeView: "search",
				searchInitialQuery: action.query,
				searchFocusKey: state.searchFocusKey + 1,
			};
		case "SET_PENDING_REVEAL":
			return { ...state, pendingReveal: action.reveal };
		case "INCREMENT_NEW_FOLDER":
			return { ...state, newFolderKey: state.newFolderKey + 1 };
	}
}

interface UseWorktreeGitActionsParams {
	rootPath: string;
	stage: (repoPath: string, paths: string[]) => Promise<void>;
	unstage: (repoPath: string, paths: string[]) => Promise<void>;
	push: (repoPath: string) => Promise<string>;
	discard: (repoPath: string, paths: string[]) => Promise<void>;
	createBranch: (repoPath: string, branchName: string) => Promise<void>;
	refreshGit: () => void;
	newBranchName: string;
	dispatchGit: React.Dispatch<GitAction>;
	dispatchUI: React.Dispatch<UIAction>;
	dispatchEditor: React.Dispatch<EditorAction>;
}

export interface WorktreeGitActions {
	handleGitStageAll: () => Promise<void>;
	handleGitUnstageAll: () => Promise<void>;
	handleGitCommit: () => void;
	handleGitPush: () => Promise<void>;
	handleGitDiscardAll: () => void;
	executeDiscardAll: () => Promise<void>;
	handleGitCreateBranch: () => void;
	executeCreateBranch: () => Promise<void>;
}

export function useWorktreeGitActions({
	rootPath,
	stage,
	unstage,
	push,
	discard,
	createBranch,
	refreshGit,
	newBranchName,
	dispatchGit,
	dispatchUI,
	dispatchEditor,
}: UseWorktreeGitActionsParams): WorktreeGitActions {
	const handleGitStageAll = useCallback(async () => {
		try {
			const status = await invoke<{ changed: Array<{ path: string }> }>(
				"get_git_status",
				{ repoPath: rootPath },
			);
			if (status.changed.length > 0) {
				await stage(
					rootPath,
					status.changed.map((f) => f.path),
				);
				refreshGit();
			}
		} catch (e) {
			dispatchGit({ type: "SET_GIT_ERROR", error: String(e) });
		}
	}, [rootPath, stage, refreshGit, dispatchGit]);

	const handleGitUnstageAll = useCallback(async () => {
		try {
			const status = await invoke<{ staged: Array<{ path: string }> }>(
				"get_git_status",
				{ repoPath: rootPath },
			);
			if (status.staged.length > 0) {
				await unstage(
					rootPath,
					status.staged.map((f) => f.path),
				);
				refreshGit();
			}
		} catch (e) {
			dispatchGit({ type: "SET_GIT_ERROR", error: String(e) });
		}
	}, [rootPath, unstage, refreshGit, dispatchGit]);

	const handleGitCommit = useCallback(() => {
		dispatchEditor({ type: "SET_ACTIVE_VIEW", view: "git" });
	}, [dispatchEditor]);

	const handleGitPush = useCallback(async () => {
		try {
			await push(rootPath);
			refreshGit();
		} catch (e) {
			dispatchGit({ type: "SET_GIT_ERROR", error: String(e) });
		}
	}, [rootPath, push, refreshGit, dispatchGit]);

	const handleGitDiscardAll = useCallback(() => {
		dispatchUI({ type: "SET_DISCARD_CONFIRM", show: true });
	}, [dispatchUI]);

	const executeDiscardAll = useCallback(async () => {
		dispatchUI({ type: "SET_DISCARD_CONFIRM", show: false });
		try {
			const status = await invoke<{ changed: Array<{ path: string }> }>(
				"get_git_status",
				{ repoPath: rootPath },
			);
			if (status.changed.length > 0) {
				await discard(
					rootPath,
					status.changed.map((f) => f.path),
				);
				refreshGit();
			}
		} catch (e) {
			dispatchGit({ type: "SET_GIT_ERROR", error: String(e) });
		}
	}, [rootPath, discard, refreshGit, dispatchGit, dispatchUI]);

	const handleGitCreateBranch = useCallback(() => {
		dispatchUI({ type: "OPEN_CREATE_BRANCH" });
	}, [dispatchUI]);

	const executeCreateBranch = useCallback(async () => {
		const name = newBranchName.trim();
		if (!name) return;
		try {
			await createBranch(rootPath, name);
			dispatchUI({ type: "CLOSE_CREATE_BRANCH" });
			refreshGit();
		} catch (e) {
			dispatchGit({ type: "SET_GIT_ERROR", error: String(e) });
		}
	}, [
		rootPath,
		createBranch,
		newBranchName,
		refreshGit,
		dispatchGit,
		dispatchUI,
	]);

	return {
		handleGitStageAll,
		handleGitUnstageAll,
		handleGitCommit,
		handleGitPush,
		handleGitDiscardAll,
		executeDiscardAll,
		handleGitCreateBranch,
		executeCreateBranch,
	};
}
