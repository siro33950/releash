import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type { PrStatus } from "@/types/git";

const POLL_INTERVAL = 30_000;

export function useBranchPr(rootPath: string, branchName: string | null) {
	const [prNumber, setPrNumber] = useState<number | null>(null);
	const [prUrl, setPrUrl] = useState<string | null>(null);
	const [loading, setLoading] = useState(true);
	const hasFetched = useRef(false);

	const fetchBranchPr = useCallback(async () => {
		if (!branchName) {
			setPrNumber(null);
			setPrUrl(null);
			setLoading(false);
			return;
		}
		if (!hasFetched.current) {
			setLoading(true);
		}
		try {
			const status = await invoke<PrStatus>("get_cached_pr_status", {
				repoPath: rootPath,
			});
			const prInfo = status.open_prs[branchName];
			if (prInfo) {
				setPrNumber(prInfo.number);
				setPrUrl(prInfo.url);
			} else {
				setPrNumber(null);
				setPrUrl(null);
			}
		} catch {
			setPrNumber(null);
			setPrUrl(null);
		} finally {
			hasFetched.current = true;
			setLoading(false);
		}
	}, [rootPath, branchName]);

	useEffect(() => {
		hasFetched.current = false;
		setLoading(true);
		fetchBranchPr();
		const id = setInterval(fetchBranchPr, POLL_INTERVAL);
		return () => clearInterval(id);
	}, [fetchBranchPr]);

	return { prNumber, prUrl, loading, refresh: fetchBranchPr };
}
