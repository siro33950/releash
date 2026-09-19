import { useCallback, useEffect, useState } from "react";
import { invokeClient as invoke, listenClient as listen } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";

export interface UseRepoListReturn {
	repoPaths: string[];
	loaded: boolean;
	loadError: string | null;
	addRepo: (path: string) => void;
	removeRepo: (path: string) => void;
	initFromCwd: (cwdRepoPath: string) => void;
}

export function useRepoList(): UseRepoListReturn {
	const [loaded, setLoaded] = useState(false);
	const [loadError, setLoadError] = useState<string | null>(null);
	const [repoPaths, setRepoPaths] = useState<string[]>([]);

	useEffect(() => {
		let cancelled = false;
		invoke("get_repo_paths")
			.then((paths) => {
				if (cancelled) return;
				setRepoPaths(paths);
				setLoaded(true);
			})
			.catch((error) => {
				if (!cancelled) setLoadError(getErrorMessage(error));
			});
		return () => {
			cancelled = true;
		};
	}, []);

	useEffect(() => {
		const unlisten = listen(
			"repo-paths-changed",
			(event) => {
				setRepoPaths(event.payload);
			},
			() => {
				void invoke("get_repo_paths")
					.then(setRepoPaths)
					.catch((err) => console.warn("[useRepoList] refresh failed", err));
			},
		);
		return () => {
			unlisten.then((fn) => fn());
		};
	}, []);

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

	return { repoPaths, loaded, loadError, addRepo, removeRepo, initFromCwd };
}
