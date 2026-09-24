import { useCallback, useEffect, useState } from "react";
import { invokeClient as invoke, subscribeState } from "@/lib/client";

export interface UseRepoListReturn {
	repoPaths: string[] | null;
	addRepo: (path: string) => void;
	removeRepo: (path: string) => void;
	initFromCwd: (cwdRepoPath: string) => void;
}

export function useRepoList(): UseRepoListReturn {
	const [repoPaths, setRepoPaths] = useState<string[] | null>(null);
	useEffect(() => subscribeState("repository-paths", setRepoPaths), []);
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

	return { repoPaths, addRepo, removeRepo, initFromCwd };
}
