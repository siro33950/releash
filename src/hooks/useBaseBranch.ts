import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

interface BranchInfo {
	name: string;
	is_remote: boolean;
}

export function useBaseBranch(
	rootPath: string | null,
	branchName: string | null,
) {
	const [baseBranch, setBaseBranchState] = useState<string | null>(null);
	const [localBranches, setLocalBranches] = useState<string[]>([]);

	const fetch = useCallback(async () => {
		if (!rootPath || !branchName) {
			setBaseBranchState(null);
			setLocalBranches([]);
			return;
		}
		try {
			const [base, branches] = await Promise.all([
				invoke<string | null>("get_branch_base", {
					repoPath: rootPath,
					branchName,
				}),
				invoke<BranchInfo[]>("list_branches", { repoPath: rootPath }),
			]);
			setBaseBranchState(base);
			setLocalBranches(
				branches
					.filter((b) => !b.is_remote && b.name !== branchName)
					.map((b) => b.name),
			);
		} catch {
			setBaseBranchState(null);
			setLocalBranches([]);
		}
	}, [rootPath, branchName]);

	useEffect(() => {
		fetch();
	}, [fetch]);

	const setBaseBranch = useCallback(
		(base: string) => {
			if (!rootPath || !branchName) return;
			setBaseBranchState(base);
			invoke("set_branch_base", {
				repoPath: rootPath,
				branchName,
				base,
			}).catch(() => {
				fetch();
			});
		},
		[rootPath, branchName, fetch],
	);

	return { baseBranch, setBaseBranch, localBranches };
}
