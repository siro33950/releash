import { useCallback } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { logClientError } from "@/lib/clientErrorNotice";
import { useStateSubscriptionResult } from "./useStateSubscription";

export function useBaseBranch(
	rootPath: string | null,
	branchName: string | null,
) {
	const baseBranch = useStateSubscriptionResult(
		rootPath && branchName
			? { kind: "branch-base", args: [rootPath, branchName] }
			: null,
	);
	const branches = useStateSubscriptionResult(
		rootPath
			? {
					kind: "branches",
					args: branchName ? [rootPath, branchName] : [rootPath],
				}
			: null,
	);
	const setBaseBranch = useCallback(
		(base: string | null) => {
			if (!rootPath || !branchName) return;
			return invoke("set_branch_base", {
				repoPath: rootPath,
				branchName,
				base: base || null,
			}).catch((error) => logClientError("Failed to set base branch:", error));
		},
		[rootPath, branchName],
	);
	return {
		baseBranch: baseBranch.value ?? null,
		error: baseBranch.error ?? branches.error,
		setBaseBranch,
		localBranches: (branches.value ?? []).map((branch) => branch.name),
	};
}
