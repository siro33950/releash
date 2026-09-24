import { useCallback } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { useStateSubscription } from "./useStateSubscription";

export function useBaseBranch(
	rootPath: string | null,
	branchName: string | null,
) {
	const baseBranch = useStateSubscription(
		rootPath && branchName
			? { kind: "branch-base", args: [rootPath, branchName] }
			: null,
	);
	const branches = useStateSubscription(
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
			}).catch((error) => {
				console.error("Failed to set base branch:", error);
			});
		},
		[rootPath, branchName],
	);
	return {
		baseBranch: baseBranch ?? null,
		setBaseBranch,
		localBranches: (branches ?? []).map((branch) => branch.name),
	};
}
