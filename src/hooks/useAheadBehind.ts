import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { AheadBehind } from "@/types/git";

export function useAheadBehind(
	rootPath: string | null,
	externalRefreshKey?: number,
): AheadBehind | null {
	const [data, setData] = useState<AheadBehind | null>(null);

	const fetch = useCallback(async () => {
		if (!rootPath) {
			setData(null);
			return;
		}
		try {
			const result = await invoke<AheadBehind>(
				"get_current_branch_ahead_behind",
				{ repoPath: rootPath },
			);
			setData(result);
		} catch {
			setData(null);
		}
	}, [rootPath]);

	useEffect(() => {
		fetch();
	}, [fetch]);

	useEffect(() => {
		if (externalRefreshKey != null && externalRefreshKey > 0) {
			fetch();
		}
	}, [externalRefreshKey, fetch]);

	return data;
}
