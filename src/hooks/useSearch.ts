import { invoke } from "@tauri-apps/api/core";
import { useCallback, useRef, useState } from "react";
import type { SearchOptions, SearchResult } from "@/types/search";

export function useSearch(rootPath: string | null) {
	const [result, setResult] = useState<SearchResult | null>(null);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const requestIdRef = useRef(0);

	const search = useCallback(
		async (pattern: string, options?: SearchOptions) => {
			if (!rootPath || !pattern.trim()) {
				setResult(null);
				return;
			}

			const requestId = ++requestIdRef.current;
			setLoading(true);
			setError(null);

			try {
				const res = await invoke<SearchResult>("search_files", {
					rootPath,
					pattern,
					caseSensitive: options?.caseSensitive ?? false,
					isRegex: options?.isRegex ?? false,
					maxResults: options?.maxResults ?? 1000,
				});

				if (requestId === requestIdRef.current) {
					setResult(res);
					setLoading(false);
				}
			} catch (e) {
				if (requestId === requestIdRef.current) {
					setError(String(e));
					setResult(null);
					setLoading(false);
				}
			}
		},
		[rootPath],
	);

	const clear = useCallback(() => {
		requestIdRef.current++;
		setResult(null);
		setError(null);
		setLoading(false);
	}, []);

	return { result, loading, error, search, clear };
}
