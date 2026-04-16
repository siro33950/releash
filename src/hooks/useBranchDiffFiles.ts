import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";

export interface BranchDiffStats {
	additions: number;
	deletions: number;
}

export interface BranchDiffChangedFile {
	path: string;
	old_path: string | null;
	status: string;
	binary: boolean;
	stats: BranchDiffStats;
}

interface BranchDiffSummary {
	base_branch: string;
	changed_files: BranchDiffChangedFile[];
	stats: BranchDiffStats;
}

interface UseBranchDiffFilesResult {
	files: BranchDiffChangedFile[];
	loading: boolean;
	error: string | null;
	refresh: () => void;
}

/**
 * Fetches the list of files that differ between the current branch's merge-base
 * and the working tree (including staged changes). Used by the Source Control
 * panel's "branch-base" diff mode.
 */
export function useBranchDiffFiles(
	rootPath: string | null,
	enabled: boolean,
	baseBranch: string | null,
): UseBranchDiffFilesResult {
	const [files, setFiles] = useState<BranchDiffChangedFile[]>([]);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const requestIdRef = useRef(0);

	const fetch = useCallback(async () => {
		const requestId = ++requestIdRef.current;
		if (!rootPath || !enabled) {
			setFiles([]);
			setError(null);
			setLoading(false);
			return;
		}
		setLoading(true);
		setError(null);
		try {
			const result = await invoke<BranchDiffSummary>(
				"get_branch_diff_summary",
				{
					repoPath: rootPath,
					baseBranch,
				},
			);
			if (requestId !== requestIdRef.current) return;
			setFiles(result.changed_files);
		} catch (e) {
			if (requestId !== requestIdRef.current) return;
			setFiles([]);
			setError(e instanceof Error ? e.message : String(e));
		} finally {
			if (requestId === requestIdRef.current) {
				setLoading(false);
			}
		}
	}, [rootPath, enabled, baseBranch]);

	useEffect(() => {
		fetch();
	}, [fetch]);

	return { files, loading, error, refresh: fetch };
}
