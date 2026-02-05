import { invoke } from "@tauri-apps/api/core";
import { useCallback } from "react";

export function useGitActions() {
	const stage = useCallback(
		async (repoPath: string, paths: string[]) => {
			await invoke("git_stage", { repoPath, paths });
		},
		[],
	);

	const unstage = useCallback(
		async (repoPath: string, paths: string[]) => {
			await invoke("git_unstage", { repoPath, paths });
		},
		[],
	);

	const commit = useCallback(
		async (repoPath: string, message: string): Promise<string> => {
			return await invoke<string>("git_commit", { repoPath, message });
		},
		[],
	);

	const push = useCallback(async (repoPath: string): Promise<string> => {
		return await invoke<string>("git_push", { repoPath });
	}, []);

	const createBranch = useCallback(
		async (repoPath: string, branchName: string) => {
			await invoke("git_create_branch", { repoPath, branchName });
		},
		[],
	);

	return { stage, unstage, commit, push, createBranch };
}
