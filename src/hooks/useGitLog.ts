import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { CommitInfo } from "@/types/git";

export function useGitLog(rootPath: string | null, limit?: number) {
	const [commits, setCommits] = useState<CommitInfo[]>([]);
	const [loading, setLoading] = useState(false);

	const fetch = useCallback(async () => {
		if (!rootPath) {
			setCommits([]);
			return;
		}
		setLoading(true);
		try {
			const result = await invoke<CommitInfo[]>("get_git_log", {
				repoPath: rootPath,
				limit: limit ?? 50,
			});
			setCommits(result);
		} catch {
			setCommits([]);
		} finally {
			setLoading(false);
		}
	}, [rootPath, limit]);

	useEffect(() => {
		fetch();
	}, [fetch]);

	return { commits, loading };
}
