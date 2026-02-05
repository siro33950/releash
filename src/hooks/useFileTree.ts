import { readDir } from "@tauri-apps/plugin-fs";
import { useCallback, useEffect, useState } from "react";
import { type FileChangeEvent, useFileWatcher } from "@/hooks/useFileWatcher";
import { normalizePath } from "@/lib/normalizePath";
import type { FileNode } from "@/types/file-tree";

interface UseFileTreeOptions {
	rootPath: string | null;
	showHidden?: boolean;
	onFileChange?: (event: FileChangeEvent) => void;
}

interface UseFileTreeReturn {
	tree: FileNode[];
	expandedPaths: Set<string>;
	loading: boolean;
	error: string | null;
	toggleExpand: (path: string) => Promise<void>;
	addExpandedPath: (path: string) => void;
	refresh: () => Promise<void>;
	collapseAll: () => void;
}

async function loadChildren(
	path: string,
	showHidden: boolean,
): Promise<FileNode[]> {
	const normalizedPath = normalizePath(path);
	const entries = await readDir(normalizedPath);

	const nodes: FileNode[] = entries
		.filter((entry) => showHidden || !entry.name.startsWith("."))
		.map((entry) => ({
			name: entry.name,
			path: `${normalizedPath}/${entry.name}`,
			type: entry.isDirectory ? ("folder" as const) : ("file" as const),
		}));

	return sortNodes(nodes);
}

function sortNodes(nodes: FileNode[]): FileNode[] {
	return nodes.sort((a, b) => {
		if (a.type === "folder" && b.type === "file") return -1;
		if (a.type === "file" && b.type === "folder") return 1;
		return a.name.localeCompare(b.name);
	});
}

function updateNodeChildren(
	nodes: FileNode[],
	targetPath: string,
	children: FileNode[] | undefined,
): FileNode[] {
	return nodes.map((node) => {
		if (node.path === targetPath) {
			return { ...node, children };
		}
		if (
			node.children &&
			(targetPath === node.path || targetPath.startsWith(`${node.path}/`))
		) {
			return {
				...node,
				children: updateNodeChildren(node.children, targetPath, children),
			};
		}
		return node;
	});
}

export function useFileTree(options: UseFileTreeOptions): UseFileTreeReturn {
	const {
		rootPath,
		showHidden = false,
		onFileChange: onFileChangeExternal,
	} = options;

	const [tree, setTree] = useState<FileNode[]>([]);
	const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const handleFileChange = useCallback(
		async (event: FileChangeEvent) => {
			onFileChangeExternal?.(event);

			const changedPath = normalizePath(event.path);
			const lastSlashIndex = changedPath.lastIndexOf("/");
			if (lastSlashIndex === -1) return;
			const parentDir = changedPath.substring(0, lastSlashIndex);

			const normalizedRootPath = rootPath ? normalizePath(rootPath) : null;

			if (parentDir === normalizedRootPath) {
				try {
					const children = await loadChildren(normalizedRootPath, showHidden);
					setTree(children);
				} catch (e) {
					console.error("Failed to reload root tree:", e);
				}
				return;
			}

			if (expandedPaths.has(parentDir)) {
				try {
					const children = await loadChildren(parentDir, showHidden);
					setTree((prev) => updateNodeChildren(prev, parentDir, children));
				} catch (e) {
					console.error("Failed to reload directory:", e);
				}
			}
		},
		[rootPath, showHidden, expandedPaths, onFileChangeExternal],
	);

	useFileWatcher({
		rootPath,
		onFileChange: handleFileChange,
	});

	const loadRootTree = useCallback(async () => {
		if (!rootPath) {
			setTree([]);
			return;
		}

		setLoading(true);
		setError(null);

		try {
			const normalizedRoot = normalizePath(rootPath);
			const children = await loadChildren(normalizedRoot, showHidden);
			setTree(children);
			setExpandedPaths(new Set());
		} catch (e) {
			setError(e instanceof Error ? e.message : "Failed to load directory");
			setTree([]);
		} finally {
			setLoading(false);
		}
	}, [rootPath, showHidden]);

	useEffect(() => {
		loadRootTree();
	}, [loadRootTree]);

	const toggleExpand = useCallback(
		async (path: string) => {
			const normalized = normalizePath(path);
			const isExpanded = expandedPaths.has(normalized);

			if (isExpanded) {
				setExpandedPaths((prev) => {
					const next = new Set(prev);
					next.delete(normalized);
					return next;
				});
			} else {
				try {
					const children = await loadChildren(normalized, showHidden);
					setTree((prev) => updateNodeChildren(prev, normalized, children));
					setExpandedPaths((prev) => new Set(prev).add(normalized));
				} catch (e) {
					setError(e instanceof Error ? e.message : "Failed to load directory");
				}
			}
		},
		[expandedPaths, showHidden],
	);

	const refresh = useCallback(async () => {
		await loadRootTree();
	}, [loadRootTree]);

	const addExpandedPath = useCallback((path: string) => {
		setExpandedPaths((prev) => new Set(prev).add(normalizePath(path)));
	}, []);

	const collapseAll = useCallback(() => {
		setExpandedPaths(new Set());
	}, []);

	return {
		tree,
		expandedPaths,
		loading,
		error,
		toggleExpand,
		addExpandedPath,
		refresh,
		collapseAll,
	};
}
