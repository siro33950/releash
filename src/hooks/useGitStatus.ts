import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import type { FileStatus } from "@/types/file-tree";
import type { GitFileStatus } from "@/types/git";

function toFileStatus(entry: GitFileStatus): FileStatus {
	if (entry.worktree_status === "new") return "untracked";
	if (entry.worktree_status === "modified") return "modified";
	if (entry.worktree_status === "deleted") return "deleted";
	if (entry.index_status === "new") return "added";
	if (entry.index_status === "modified") return "modified";
	if (entry.index_status === "deleted") return "deleted";
	if (entry.index_status === "renamed") return "modified";
	return null;
}

export function useGitStatus(rootPath: string | null) {
	const [statusMap, setStatusMap] = useState<Map<string, FileStatus>>(
		new Map(),
	);
	const [stagedFiles, setStagedFiles] = useState<GitFileStatus[]>([]);
	const [changedFiles, setChangedFiles] = useState<GitFileStatus[]>([]);
	const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	const fetchStatus = useCallback(async () => {
		if (!rootPath) {
			setStatusMap(new Map());
			setStagedFiles([]);
			setChangedFiles([]);
			return;
		}
		try {
			const entries = await invoke<GitFileStatus[]>("get_git_status", {
				repoPath: rootPath,
			});
			const map = new Map<string, FileStatus>();
			const staged: GitFileStatus[] = [];
			const changed: GitFileStatus[] = [];

			for (const entry of entries) {
				const absPath = `${rootPath}/${entry.path}`;
				map.set(absPath, toFileStatus(entry));
				if (entry.index_status !== "none") staged.push(entry);
				if (entry.worktree_status !== "none") changed.push(entry);
			}

			setStatusMap(map);
			setStagedFiles(staged);
			setChangedFiles(changed);
		} catch {
			setStatusMap(new Map());
			setStagedFiles([]);
			setChangedFiles([]);
		}
	}, [rootPath]);

	const refresh = useCallback(() => {
		fetchStatus();
	}, [fetchStatus]);

	const debouncedRefresh = useCallback(() => {
		if (timerRef.current) clearTimeout(timerRef.current);
		timerRef.current = setTimeout(() => {
			fetchStatus();
		}, 300);
	}, [fetchStatus]);

	useEffect(() => {
		fetchStatus();
	}, [fetchStatus]);

	useEffect(() => {
		let unlisten: UnlistenFn | null = null;
		let mounted = true;

		const setup = async () => {
			unlisten = await listen("file-change", () => {
				if (mounted) debouncedRefresh();
			});
		};
		setup();

		return () => {
			mounted = false;
			unlisten?.();
			if (timerRef.current) clearTimeout(timerRef.current);
		};
	}, [debouncedRefresh]);

	return { statusMap, stagedFiles, changedFiles, refresh };
}
