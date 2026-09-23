import { useCallback } from "react";
import { invokeClient as invoke } from "@/lib/client";

export interface UseRepoListReturn {
	addRepo: (path: string) => void;
	removeRepo: (path: string) => void;
	initFromCwd: (cwdRepoPath: string) => void;
}

export function useRepoList(): UseRepoListReturn {
	const addRepo = useCallback((path: string) => {
		invoke("add_repo_path", { path }).catch((err) =>
			console.warn("[useRepoList] add_repo_path failed", err),
		);
	}, []);

	const removeRepo = useCallback((path: string) => {
		invoke("remove_repo_path", { path }).catch((err) =>
			console.warn("[useRepoList] remove_repo_path failed", err),
		);
	}, []);

	const initFromCwd = useCallback((cwdRepoPath: string) => {
		invoke("add_repo_path", { path: cwdRepoPath }).catch((err) =>
			console.warn("[useRepoList] add_repo_path(initFromCwd) failed", err),
		);
	}, []);

	return { addRepo, removeRepo, initFromCwd };
}
