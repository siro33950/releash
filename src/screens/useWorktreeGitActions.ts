import { useCallback } from "react";
import { getErrorMessage } from "@/lib/errorMessage";

// Re-export reducer types used by this hook
export type GitAction = { type: "SET_GIT_ERROR"; error: string | null };

export interface GitState {
	gitError: string | null;
}

export function gitReducer(state: GitState, action: GitAction): GitState {
	switch (action.type) {
		case "SET_GIT_ERROR":
			return { ...state, gitError: action.error };
	}
}

export type UIAction =
	| { type: "SET_SETTINGS_OPEN"; open: boolean }
	| { type: "OPEN_CREATE_BRANCH" }
	| { type: "CLOSE_CREATE_BRANCH" }
	| { type: "SET_NEW_BRANCH_NAME"; name: string };

export interface UIState {
	isSettingsOpen: boolean;
	showCreateBranch: boolean;
	newBranchName: string;
}

export const initialUIState: UIState = {
	isSettingsOpen: false,
	showCreateBranch: false,
	newBranchName: "",
};

export function uiReducer(state: UIState, action: UIAction): UIState {
	switch (action.type) {
		case "SET_SETTINGS_OPEN":
			return { ...state, isSettingsOpen: action.open };
		case "OPEN_CREATE_BRANCH":
			return { ...state, showCreateBranch: true, newBranchName: "" };
		case "CLOSE_CREATE_BRANCH":
			return { ...state, showCreateBranch: false };
		case "SET_NEW_BRANCH_NAME":
			return { ...state, newBranchName: action.name };
	}
}

interface UseWorktreeGitActionsParams {
	rootPath: string;
	stage: (repoPath: string, paths: string[]) => Promise<void>;
	unstage: (repoPath: string, paths: string[]) => Promise<void>;
	createBranch: (repoPath: string, branchName: string) => Promise<void>;
	newBranchName: string;
	dispatchGit: React.Dispatch<GitAction>;
	dispatchUI: React.Dispatch<UIAction>;
}

export interface WorktreeGitActions {
	handleGitStageAll: () => Promise<void>;
	handleGitUnstageAll: () => Promise<void>;
	handleGitCreateBranch: () => void;
	executeCreateBranch: () => Promise<void>;
}

export function useWorktreeGitActions({
	rootPath,
	stage,
	unstage,
	createBranch,
	newBranchName,
	dispatchGit,
	dispatchUI,
}: UseWorktreeGitActionsParams): WorktreeGitActions {
	const handleGitStageAll = useCallback(async () => {
		try {
			await stage(rootPath, []);
		} catch (e) {
			dispatchGit({ type: "SET_GIT_ERROR", error: getErrorMessage(e) });
		}
	}, [rootPath, stage, dispatchGit]);

	const handleGitUnstageAll = useCallback(async () => {
		try {
			await unstage(rootPath, []);
		} catch (e) {
			dispatchGit({ type: "SET_GIT_ERROR", error: getErrorMessage(e) });
		}
	}, [rootPath, unstage, dispatchGit]);

	const handleGitCreateBranch = useCallback(() => {
		dispatchUI({ type: "OPEN_CREATE_BRANCH" });
	}, [dispatchUI]);

	const executeCreateBranch = useCallback(async () => {
		const name = newBranchName.trim();
		if (!name) return;
		try {
			await createBranch(rootPath, name);
			dispatchUI({ type: "CLOSE_CREATE_BRANCH" });
		} catch (e) {
			dispatchGit({ type: "SET_GIT_ERROR", error: getErrorMessage(e) });
		}
	}, [rootPath, createBranch, newBranchName, dispatchGit, dispatchUI]);

	return {
		handleGitStageAll,
		handleGitUnstageAll,
		handleGitCreateBranch,
		executeCreateBranch,
	};
}
