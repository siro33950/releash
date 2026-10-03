import { useCallback, useEffect, useState } from "react";
import { invokeClient as invoke, subscribeState } from "@/lib/client";
import { showClientError } from "@/lib/clientErrorNotice";

export interface UseRepoListReturn {
	repoPaths: string[] | null;
	addRepo: (path: string) => void;
	removeRepo: (path: string) => void;
}

export function useRepoList(): UseRepoListReturn {
	const [repoPaths, setRepoPaths] = useState<string[] | null>(null);
	useEffect(
		() =>
			subscribeState("repository-paths", setRepoPaths, (error) => {
				setRepoPaths(null);
				showClientError(error);
			}),
		[],
	);
	const addRepo = useCallback((path: string) => {
		invoke("add_repo_path", { path }).catch(showClientError);
	}, []);

	const removeRepo = useCallback((path: string) => {
		invoke("remove_repo_path", { path }).catch(showClientError);
	}, []);

	return { repoPaths, addRepo, removeRepo };
}
