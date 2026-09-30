import { useCallback } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { logClientError } from "@/lib/clientErrorNotice";
import { useStateSubscription } from "./useStateSubscription";

export function useIssues(repoPath: string) {
	const issues = useStateSubscription({ kind: "issues", args: [repoPath] });
	const refresh = useCallback(
		() =>
			invoke("fetch_issues", { repoPath }).catch((error) =>
				logClientError("Failed to fetch issues:", error),
			),
		[repoPath],
	);
	return { issues: issues ?? [], loading: issues === undefined, refresh };
}
