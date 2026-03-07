import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { formatGitError } from "@/lib/errorHandler";
import { useGitEventRefresh } from "./useGitEventRefresh";

export interface ReviewFileStats {
	additions: number;
	deletions: number;
}

export interface ReviewChangedFile {
	path: string;
	old_path: string | null;
	status: string;
	binary: boolean;
	stats: ReviewFileStats;
}

interface ReviewDiffResponse {
	base_ref: string;
	changed_files: ReviewChangedFile[];
	stats: {
		files_changed: number;
		insertions: number;
		deletions: number;
	};
}

export function useReviewDiffFiles(
	rootPath: string | null,
	enabled: boolean,
	baseBranch: string | null,
) {
	const [files, setFiles] = useState<ReviewChangedFile[]>([]);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const fetchFiles = useCallback(async () => {
		if (!rootPath || !enabled) {
			setFiles([]);
			setError(null);
			return;
		}
		setLoading(true);
		setError(null);
		try {
			const result = await invoke<ReviewDiffResponse>(
				"get_review_diff_summary",
				{
					repoPath: rootPath,
					baseBranch: baseBranch ?? undefined,
				},
			);
			setFiles(result.changed_files);
		} catch (e) {
			setFiles([]);
			setError(formatGitError(e));
		} finally {
			setLoading(false);
		}
	}, [rootPath, enabled, baseBranch]);

	useEffect(() => {
		fetchFiles();
	}, [fetchFiles]);

	useGitEventRefresh(rootPath, fetchFiles, enabled);

	return { files, loading, error, refresh: fetchFiles };
}
