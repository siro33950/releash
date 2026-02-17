import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { PrDetail } from "@/types/git";

export function usePrDetail(rootPath: string, prNumber: number | null) {
	const [detail, setDetail] = useState<PrDetail | null>(null);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const fetchDetail = useCallback(
		async (ignore?: { current: boolean }) => {
			if (!prNumber) {
				setDetail(null);
				setLoading(false);
				setError(null);
				return;
			}
			setLoading(true);
			setError(null);
			try {
				const result = await invoke<PrDetail | null>("get_pr_detail", {
					repoPath: rootPath,
					prNumber,
				});
				if (ignore?.current) return;
				setDetail(result);
			} catch (e) {
				if (ignore?.current) return;
				setError(String(e));
				setDetail(null);
			} finally {
				if (!ignore?.current) {
					setLoading(false);
				}
			}
		},
		[rootPath, prNumber],
	);

	useEffect(() => {
		const ignore = { current: false };
		fetchDetail(ignore);
		return () => {
			ignore.current = true;
		};
	}, [fetchDetail]);

	const refresh = useCallback(() => fetchDetail(), [fetchDetail]);

	return { detail, loading, error, refresh };
}
