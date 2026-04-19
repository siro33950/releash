import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type { BranchDiffChangedFile } from "@/hooks/useBranchDiffFiles";
import type { GitFileStatus } from "@/types/git";
import type { DiffBase } from "@/types/settings";

export interface DiffFileEntry {
	path: string;
	status: string;
	additions: number;
	deletions: number;
}

export interface DiffTreeNode {
	id: string;
	name: string;
	path: string;
	node_type: "file" | "folder";
	status: string | null;
	additions: number | null;
	deletions: number | null;
	children: DiffTreeNode[];
}

interface StatusFileStat {
	path: string;
	index_additions: number;
	index_deletions: number;
	wt_additions: number;
	wt_deletions: number;
}

export interface UseDiffFileTreeResult {
	stagedTree: DiffTreeNode[];
	changesTree: DiffTreeNode[];
	stagedFileCount: number;
	changesFileCount: number;
	branchBaseTree: DiffTreeNode[];
	branchBaseFileCount: number;
	loading: boolean;
}

function branchDiffToEntries(files: BranchDiffChangedFile[]): DiffFileEntry[] {
	return files.map((f) => ({
		path: f.path,
		status: f.status,
		additions: f.stats.additions,
		deletions: f.stats.deletions,
	}));
}

function stagedToIndexEntries(
	stagedFiles: GitFileStatus[],
	statsMap: Map<string, StatusFileStat>,
): DiffFileEntry[] {
	return stagedFiles.map((f) => {
		const stat = statsMap.get(f.path);
		return {
			path: f.path,
			status: f.index_status,
			additions: stat?.index_additions ?? 0,
			deletions: stat?.index_deletions ?? 0,
		};
	});
}

function changedToWtEntries(
	changedFiles: GitFileStatus[],
	statsMap: Map<string, StatusFileStat>,
): DiffFileEntry[] {
	return changedFiles.map((f) => {
		const stat = statsMap.get(f.path);
		return {
			path: f.path,
			status: f.worktree_status,
			additions: stat?.wt_additions ?? 0,
			deletions: stat?.wt_deletions ?? 0,
		};
	});
}

async function buildTreeFromEntries(
	entries: DiffFileEntry[],
): Promise<DiffTreeNode[]> {
	if (entries.length === 0) return [];
	return invoke<DiffTreeNode[]>("build_diff_file_tree", { entries });
}

export function useDiffFileTree(
	diffBase: DiffBase,
	branchDiffFiles: BranchDiffChangedFile[],
	stagedFiles: GitFileStatus[],
	changedFiles: GitFileStatus[],
	rootPath?: string,
): UseDiffFileTreeResult {
	const [stagedTree, setStagedTree] = useState<DiffTreeNode[]>([]);
	const [changesTree, setChangesTree] = useState<DiffTreeNode[]>([]);
	const [stagedFileCount, setStagedFileCount] = useState(0);
	const [changesFileCount, setChangesFileCount] = useState(0);
	const [branchBaseTree, setBranchBaseTree] = useState<DiffTreeNode[]>([]);
	const [branchBaseFileCount, setBranchBaseFileCount] = useState(0);
	const [loading, setLoading] = useState(false);
	const requestIdRef = useRef(0);

	const buildTree = useCallback(async () => {
		const requestId = ++requestIdRef.current;

		if (diffBase === "branch-base") {
			// Branch Base mode: single tree
			setStagedTree([]);
			setChangesTree([]);
			setStagedFileCount(0);
			setChangesFileCount(0);

			const entries = branchDiffToEntries(branchDiffFiles);
			setBranchBaseFileCount(entries.length);

			if (entries.length === 0) {
				setBranchBaseTree([]);
				setLoading(false);
				return;
			}

			setLoading(true);
			try {
				const result = await buildTreeFromEntries(entries);
				if (requestId !== requestIdRef.current) return;
				setBranchBaseTree(result);
			} catch {
				if (requestId !== requestIdRef.current) return;
				setBranchBaseTree([]);
			} finally {
				if (requestId === requestIdRef.current) {
					setLoading(false);
				}
			}
		} else {
			// HEAD mode: two trees (staged + changes)
			setBranchBaseTree([]);
			setBranchBaseFileCount(0);

			let statsMap = new Map<string, StatusFileStat>();
			if (rootPath && (stagedFiles.length > 0 || changedFiles.length > 0)) {
				try {
					const stats = await invoke<StatusFileStat[]>(
						"get_status_diff_stats",
						{ repoPath: rootPath },
					);
					if (requestId !== requestIdRef.current) return;
					statsMap = new Map(stats.map((s) => [s.path, s]));
				} catch {
					// Fall back to 0 stats
				}
				if (requestId !== requestIdRef.current) return;
			}

			const stagedEntries = stagedToIndexEntries(stagedFiles, statsMap);
			const changesEntries = changedToWtEntries(changedFiles, statsMap);
			setStagedFileCount(stagedEntries.length);
			setChangesFileCount(changesEntries.length);

			if (stagedEntries.length === 0 && changesEntries.length === 0) {
				setStagedTree([]);
				setChangesTree([]);
				setLoading(false);
				return;
			}

			setLoading(true);
			try {
				const [sTree, cTree] = await Promise.all([
					buildTreeFromEntries(stagedEntries),
					buildTreeFromEntries(changesEntries),
				]);
				if (requestId !== requestIdRef.current) return;
				setStagedTree(sTree);
				setChangesTree(cTree);
			} catch {
				if (requestId !== requestIdRef.current) return;
				setStagedTree([]);
				setChangesTree([]);
			} finally {
				if (requestId === requestIdRef.current) {
					setLoading(false);
				}
			}
		}
	}, [diffBase, branchDiffFiles, stagedFiles, changedFiles, rootPath]);

	useEffect(() => {
		buildTree();
	}, [buildTree]);

	return {
		stagedTree,
		changesTree,
		stagedFileCount,
		changesFileCount,
		branchBaseTree,
		branchBaseFileCount,
		loading,
	};
}
