import { useCallback } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { logClientError } from "@/lib/clientErrorNotice";
import { useStateSubscriptionResult } from "./useStateSubscription";

export function useIssues(repoPath: string) {
	const issues = useStateSubscriptionResult({
		kind: "issues",
		args: [repoPath],
	});
	const refresh = useCallback(
		() =>
			invoke("fetch_issues", { repoPath }).catch((error) =>
				logClientError("Failed to fetch issues:", error),
			),
		[repoPath],
	);
	return {
		issues: issues.value ?? [],
		error: issues.error,
		loading: issues.value === undefined && !issues.error,
		refresh,
	};
}
