import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";

export interface UseRepoListReturn {
	repoPaths: string[];
	addRepo: (path: string) => void;
	removeRepo: (path: string) => void;
	initFromCwd: (cwdRepoPath: string) => void;
}

export function useRepoList(): UseRepoListReturn {
	const [repoPaths, setRepoPaths] = useState<string[]>([]);

	useEffect(() => {
		invoke<string[]>("get_repo_paths")
			.then(setRepoPaths)
			.catch(() => {});
	}, []);

	useEffect(() => {
		const unlisten = listen<string[]>("repo-paths-changed", (event) => {
			setRepoPaths(event.payload);
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, []);

	const addRepo = useCallback((path: string) => {
		invoke("add_repo_path", { path }).catch(() => {});
	}, []);

	const removeRepo = useCallback((path: string) => {
		invoke("remove_repo_path", { path }).catch(() => {});
	}, []);

	const initFromCwd = useCallback((cwdRepoPath: string) => {
		invoke("add_repo_path", { path: cwdRepoPath }).catch(() => {});
	}, []);

	return { repoPaths, addRepo, removeRepo, initFromCwd };
}
