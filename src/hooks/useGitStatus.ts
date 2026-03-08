import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type { FileStatus } from "@/types/file-tree";
import type { GitFileStatus } from "@/types/git";
import { useGitEventRefresh } from "./useGitEventRefresh";

function toFileStatus(entry: GitFileStatus): FileStatus {
	if (entry.worktree_status === "ignored") return "ignored";
	if (entry.worktree_status === "new") return "untracked";
	if (entry.worktree_status === "modified") return "modified";
	if (entry.worktree_status === "deleted") return "deleted";
	if (entry.index_status === "new") return "added";
	if (entry.index_status === "modified") return "modified";
	if (entry.index_status === "deleted") return "deleted";
	if (entry.index_status === "renamed") return "modified";
	return null;
}

export function useGitStatus(
	rootPath: string | null,
	externalRefreshKey?: number,
) {
	const [statusMap, setStatusMap] = useState<Map<string, FileStatus>>(
		new Map(),
	);
	const [stagedFiles, setStagedFiles] = useState<GitFileStatus[]>([]);
	const [changedFiles, setChangedFiles] = useState<GitFileStatus[]>([]);
	const prevEntriesRef = useRef<string>("");

	const fetchStatus = useCallback(async () => {
		if (!rootPath) {
			setStatusMap(new Map());
			setStagedFiles([]);
			setChangedFiles([]);
			prevEntriesRef.current = "";
			return;
		}
		try {
			const entries = await invoke<GitFileStatus[]>("get_git_status", {
				repoPath: rootPath,
			});

			const serialized = JSON.stringify(entries);
			if (serialized === prevEntriesRef.current) return;
			prevEntriesRef.current = serialized;

			const map = new Map<string, FileStatus>();
			const staged: GitFileStatus[] = [];
			const changed: GitFileStatus[] = [];

			for (const entry of entries) {
				const absPath = `${rootPath}/${entry.path}`;
				map.set(absPath, toFileStatus(entry));
				if (entry.index_status !== "none") staged.push(entry);
				if (
					entry.worktree_status !== "none" &&
					entry.worktree_status !== "ignored"
				)
					changed.push(entry);
			}

			setStatusMap(map);
			setStagedFiles(staged);
			setChangedFiles(changed);
		} catch {
			setStatusMap(new Map());
			setStagedFiles([]);
			setChangedFiles([]);
			prevEntriesRef.current = "";
		}
	}, [rootPath]);

	const refresh = useCallback(() => {
		fetchStatus();
	}, [fetchStatus]);

	useEffect(() => {
		fetchStatus();
	}, [fetchStatus]);

	useEffect(() => {
		if (externalRefreshKey != null && externalRefreshKey > 0) {
			fetchStatus();
		}
	}, [externalRefreshKey, fetchStatus]);

	useGitEventRefresh(rootPath, fetchStatus);

	return { statusMap, stagedFiles, changedFiles, refresh };
}
