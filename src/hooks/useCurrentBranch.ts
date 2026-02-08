import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

export function useCurrentBranch(rootPath: string | null) {
	const [branch, setBranch] = useState<string | null>(null);

	const fetch = useCallback(async () => {
		if (!rootPath) {
			setBranch(null);
			return;
		}
		try {
			const name = await invoke<string>("get_current_branch", {
				repoPath: rootPath,
			});
			setBranch(name);
		} catch {
			setBranch(null);
		}
	}, [rootPath]);

	useEffect(() => {
		fetch();
	}, [fetch]);

	return { branch, refresh: fetch };
}
