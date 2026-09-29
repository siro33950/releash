import { useCallback } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { showClientError } from "@/lib/clientErrorNotice";

export function useGitActions() {
	const stage = useCallback(async (repoPath: string, paths: string[]) => {
		await invoke("git_stage", { repoPath, paths }).catch((error) => {
			showClientError(error);
			throw error;
		});
	}, []);

	const unstage = useCallback(async (repoPath: string, paths: string[]) => {
		await invoke("git_unstage", { repoPath, paths }).catch((error) => {
			showClientError(error);
			throw error;
		});
	}, []);

	const createBranch = useCallback(
		async (repoPath: string, branchName: string) => {
			await invoke("git_create_branch", { repoPath, branchName }).catch(
				(error) => {
					showClientError(error);
					throw error;
				},
			);
		},
		[],
	);

	return {
		stage,
		unstage,
		createBranch,
	};
}
