import { useCallback, useEffect, useState } from "react";
import { invokeClient } from "@/lib/client";

export function useCurrentBranch(rootPath: string | null) {
	const [branch, setBranch] = useState<string | null>(null);

	const fetch = useCallback(async () => {
		if (!rootPath) {
			setBranch(null);
			return;
		}
		try {
			const name = await invokeClient("get_current_branch", {
				repoPath: rootPath,
			});
			setBranch(name);
		} catch (err) {
			console.error("[useCurrentBranch] Failed to get branch:", err);
			setBranch(null);
		}
	}, [rootPath]);

	useEffect(() => {
		fetch();
	}, [fetch]);

	return { branch, refresh: fetch };
}
