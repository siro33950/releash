import { useCallback, useRef, useState } from "react";
import { useFileOperations } from "@/hooks/useFileOperations";
import type { FileNode } from "@/types/file-tree";

function getTargetDir(nodePath: string, nodeType: "file" | "folder") {
	return nodeType === "folder"
		? nodePath
		: nodePath.substring(0, nodePath.lastIndexOf("/"));
}

function findNodeByPath(
	nodes: FileNode[],
	targetPath: string,
): FileNode | undefined {
	for (const node of nodes) {
		if (node.path === targetPath) return node;
		if (node.children) {
			const found = findNodeByPath(node.children, targetPath);
			if (found) return found;
		}
	}
	return undefined;
}

interface UseFileContextActionsParams {
	rootPath: string;
	tree: FileNode[];
	selectedPath: string | null;
	addExpandedPath: (path: string) => void;
	onRename?: (oldPath: string, newPath: string) => void;
	onDelete?: (path: string) => void;
}

export function useFileContextActions({
	rootPath,
	tree,
	selectedPath,
	addExpandedPath,
	onRename,
	onDelete,
}: UseFileContextActionsParams) {
	const [creatingNode, setCreatingNode] = useState<{
		parentPath: string;
		type: "file" | "folder";
	} | null>(null);
	const [renamingPath, setRenamingPath] = useState<string | null>(null);
	const [deletingPath, setDeletingPath] = useState<string | null>(null);

	const fileOps = useFileOperations();

	const prevRootPathRef = useRef(rootPath);
	if (prevRootPathRef.current !== rootPath) {
		prevRootPathRef.current = rootPath;
		if (creatingNode !== null) setCreatingNode(null);
		if (renamingPath !== null) setRenamingPath(null);
		if (deletingPath !== null) setDeletingPath(null);
	}

	const handleContextNewFile = useCallback(
		(nodePath: string, nodeType: "file" | "folder") => {
			const parentPath = getTargetDir(nodePath, nodeType);
			if (nodeType === "folder") {
				addExpandedPath(nodePath);
			}
			setCreatingNode({ parentPath, type: "file" });
		},
		[addExpandedPath],
	);

	const handleContextNewFolder = useCallback(
		(nodePath: string, nodeType: "file" | "folder") => {
			const parentPath = getTargetDir(nodePath, nodeType);
			if (nodeType === "folder") {
				addExpandedPath(nodePath);
			}
			setCreatingNode({ parentPath, type: "folder" });
		},
		[addExpandedPath],
	);

	const handleCreateCommit = useCallback(
		async (name: string) => {
			if (!creatingNode) return;
			const fullPath = `${creatingNode.parentPath}/${name}`;
			try {
				if (creatingNode.type === "folder") {
					await fileOps.createFolder(fullPath);
				} else {
					await fileOps.createFile(fullPath);
				}
			} catch (e) {
				console.error("Failed to create:", e);
			}
			setCreatingNode(null);
		},
		[creatingNode, fileOps],
	);

	const handleCreateCancel = useCallback(() => {
		setCreatingNode(null);
	}, []);

	const handleRenameCommit = useCallback(
		async (oldPath: string, newName: string) => {
			const parentDir = oldPath.substring(0, oldPath.lastIndexOf("/"));
			const newPath = `${parentDir}/${newName}`;
			try {
				await fileOps.renameItem(oldPath, newPath);
				onRename?.(oldPath, newPath);
			} catch (e) {
				console.error("Failed to rename:", e);
			}
			setRenamingPath(null);
		},
		[fileOps, onRename],
	);

	const handleRenameCancel = useCallback(() => {
		setRenamingPath(null);
	}, []);

	const handleDeleteConfirm = useCallback(async () => {
		if (!deletingPath) return;
		try {
			await fileOps.deleteItem(deletingPath);
			onDelete?.(deletingPath);
		} catch (e) {
			console.error("Failed to delete:", e);
		}
		setDeletingPath(null);
	}, [deletingPath, fileOps, onDelete]);

	const handleDeleteCancel = useCallback(() => {
		setDeletingPath(null);
	}, []);

	const handleContextPaste = useCallback(
		async (nodePath: string, nodeType: "file" | "folder") => {
			const destDir = getTargetDir(nodePath, nodeType);
			try {
				await fileOps.paste(destDir);
			} catch (e) {
				console.error("Failed to paste:", e);
			}
		},
		[fileOps],
	);

	const handleContextCopyRelativePath = useCallback(
		(path: string) => {
			if (rootPath) {
				fileOps.copyRelativePath(path, rootPath);
			}
		},
		[fileOps, rootPath],
	);

	const handleToolbarNewFile = useCallback(() => {
		if (!rootPath) return;
		let parentPath = rootPath;
		if (selectedPath) {
			const node = findNodeByPath(tree, selectedPath);
			const isDir = node?.type === "folder";
			parentPath = isDir
				? selectedPath
				: selectedPath.substring(0, selectedPath.lastIndexOf("/"));
		}
		setCreatingNode({ parentPath, type: "file" });
	}, [rootPath, selectedPath, tree]);

	const handleToolbarNewFolder = useCallback(() => {
		if (!rootPath) return;
		let parentPath = rootPath;
		if (selectedPath) {
			const node = findNodeByPath(tree, selectedPath);
			const isDir = node?.type === "folder";
			parentPath = isDir
				? selectedPath
				: selectedPath.substring(0, selectedPath.lastIndexOf("/"));
		}
		setCreatingNode({ parentPath, type: "folder" });
	}, [rootPath, selectedPath, tree]);

	return {
		creatingNode,
		setCreatingNode,
		renamingPath,
		setRenamingPath,
		deletingPath,
		setDeletingPath,
		fileOps,
		handleContextNewFile,
		handleContextNewFolder,
		handleCreateCommit,
		handleCreateCancel,
		handleRenameCommit,
		handleRenameCancel,
		handleDeleteConfirm,
		handleDeleteCancel,
		handleContextPaste,
		handleContextCopyRelativePath,
		handleToolbarNewFile,
		handleToolbarNewFolder,
	};
}
