import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type { IssueInfo } from "@/types/git";

const POLL_INTERVAL = 30_000;

export function useIssues(repoPath: string) {
	const [issues, setIssues] = useState<IssueInfo[]>([]);
	const [loading, setLoading] = useState(true);
	const hasFetched = useRef(false);

	const fetchIssues = useCallback(
		async (
			command: "get_cached_issues" | "fetch_issues" = "get_cached_issues",
		) => {
			if (!hasFetched.current) {
				setLoading(true);
			}
			try {
				const result = await invoke<IssueInfo[]>(command, {
					repoPath,
				});
				setIssues(result);
			} catch (e) {
				console.error(`[useIssues] ${command} failed for ${repoPath}:`, e);
				setIssues([]);
			} finally {
				hasFetched.current = true;
				setLoading(false);
			}
		},
		[repoPath],
	);

	const refresh = useCallback(() => fetchIssues("fetch_issues"), [fetchIssues]);

	useEffect(() => {
		hasFetched.current = false;
		setLoading(true);
		fetchIssues();
		const id = setInterval(fetchIssues, POLL_INTERVAL);
		return () => clearInterval(id);
	}, [fetchIssues]);

	return { issues, loading, refresh };
}
