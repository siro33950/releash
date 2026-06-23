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

export interface UseDiffFileTreeResult {
	stagedTree: DiffTreeNode[];
	changesTree: DiffTreeNode[];
	stagedFileCount: number;
	changesFileCount: number;
	branchBaseTree: DiffTreeNode[];
	branchBaseFileCount: number;
	loading: boolean;
}

interface HeadDiffFileTreeSnapshot {
	version: number;
	stale: boolean;
	loading: boolean;
	limited: boolean;
	combined_tree: DiffTreeNode[];
	staged_tree: DiffTreeNode[];
	changes_tree: DiffTreeNode[];
	staged_file_count: number;
	changes_file_count: number;
}

async function buildTreeFromEntries(
	entries: DiffFileEntry[],
): Promise<DiffTreeNode[]> {
	if (entries.length === 0) return [];
	return invoke<DiffTreeNode[]>("build_diff_file_tree", { entries });
}

function branchDiffToEntries(files: BranchDiffChangedFile[]): DiffFileEntry[] {
	return files.map((f) => ({
		path: f.path,
		status: f.status,
		additions: f.stats.additions,
		deletions: f.stats.deletions,
	}));
}

export function useDiffFileTree(
	diffBase: DiffBase,
	branchDiffFiles: BranchDiffChangedFile[],
	stagedFiles: GitFileStatus[],
	changedFiles: GitFileStatus[],
	rootPath?: string,
	statusVersion?: number,
): UseDiffFileTreeResult {
	const [stagedTree, setStagedTree] = useState<DiffTreeNode[]>([]);
	const [changesTree, setChangesTree] = useState<DiffTreeNode[]>([]);
	const [stagedFileCount, setStagedFileCount] = useState(0);
	const [changesFileCount, setChangesFileCount] = useState(0);
	const [branchBaseTree, setBranchBaseTree] = useState<DiffTreeNode[]>([]);
	const [branchBaseFileCount, setBranchBaseFileCount] = useState(0);
	const [loading, setLoading] = useState(false);
	const requestIdRef = useRef(0);
	const headStatusVersion = diffBase === "head" ? statusVersion : undefined;

	const buildTree = useCallback(async () => {
		const requestId = ++requestIdRef.current;
		const requestedStatusVersion = headStatusVersion;

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

			if (
				!rootPath ||
				(stagedFiles.length === 0 && changedFiles.length === 0)
			) {
				setStagedTree([]);
				setChangesTree([]);
				setStagedFileCount(0);
				setChangesFileCount(0);
				setLoading(false);
				return;
			}

			setLoading(true);
			try {
				const snapshot = await invoke<HeadDiffFileTreeSnapshot>(
					"get_head_diff_file_tree_snapshot",
					{ repoPath: rootPath },
				);
				if (requestId !== requestIdRef.current) return;
				if (
					requestedStatusVersion != null &&
					snapshot.version < requestedStatusVersion
				)
					return;
				setStagedTree(snapshot.staged_tree);
				setChangesTree(snapshot.changes_tree);
				setStagedFileCount(snapshot.staged_file_count);
				setChangesFileCount(snapshot.changes_file_count);
			} catch {
				if (requestId !== requestIdRef.current) return;
				setStagedTree([]);
				setChangesTree([]);
				setStagedFileCount(0);
				setChangesFileCount(0);
			} finally {
				if (requestId === requestIdRef.current) {
					setLoading(false);
				}
			}
		}
	}, [
		diffBase,
		branchDiffFiles,
		stagedFiles,
		changedFiles,
		rootPath,
		headStatusVersion,
	]);

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
