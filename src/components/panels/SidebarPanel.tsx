import { open } from "@tauri-apps/plugin-dialog";
import {
	ChevronsDownUp,
	FilePlus,
	FolderOpen,
	FolderPlus,
	RefreshCw,
} from "lucide-react";
import { useCallback, useState } from "react";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useFileOperations } from "@/hooks/useFileOperations";
import { useFileTree } from "@/hooks/useFileTree";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import { FileTree } from "./FileTree";

function getTargetDir(nodePath: string, nodeType: "file" | "folder") {
	return nodeType === "folder"
		? nodePath
		: nodePath.substring(0, nodePath.lastIndexOf("/"));
}

export interface SidebarPanelProps {
	onSelectFile?: (path: string) => void;
	onFileChange?: (path: string) => void;
	onRename?: (oldPath: string, newPath: string) => void;
	onDelete?: (path: string) => void;
}

export function SidebarPanel({
	onSelectFile,
	onFileChange,
	onRename,
	onDelete,
}: SidebarPanelProps) {
	const [rootPath, setRootPath] = useState<string | null>(null);
	const [selectedPath, setSelectedPath] = useState<string | null>(null);

	const [creatingNode, setCreatingNode] = useState<{
		parentPath: string;
		type: "file" | "folder";
	} | null>(null);
	const [renamingPath, setRenamingPath] = useState<string | null>(null);
	const [deletingPath, setDeletingPath] = useState<string | null>(null);

	const {
		tree,
		expandedPaths,
		loading,
		error,
		toggleExpand,
		addExpandedPath,
		refresh,
		collapseAll,
	} = useFileTree({
		rootPath,
		onFileChange: onFileChange
			? (event) => onFileChange(event.path)
			: undefined,
	});

	const fileOps = useFileOperations();

	const handleOpenFolder = async () => {
		const selected = await open({ directory: true });
		if (selected) {
			setRootPath(selected);
			setSelectedPath(null);
		}
	};

	const handleSelectFile = (path: string) => {
		setSelectedPath(path);
		onSelectFile?.(path);
	};

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
			const isDir = expandedPaths.has(selectedPath);
			parentPath = isDir
				? selectedPath
				: selectedPath.substring(0, selectedPath.lastIndexOf("/"));
		}
		setCreatingNode({ parentPath, type: "file" });
	}, [rootPath, selectedPath, expandedPaths]);

	const handleToolbarNewFolder = useCallback(() => {
		if (!rootPath) return;
		let parentPath = rootPath;
		if (selectedPath) {
			const isDir = expandedPaths.has(selectedPath);
			parentPath = isDir
				? selectedPath
				: selectedPath.substring(0, selectedPath.lastIndexOf("/"));
		}
		setCreatingNode({ parentPath, type: "folder" });
	}, [rootPath, selectedPath, expandedPaths]);

	const deletingName = deletingPath?.split("/").pop() ?? "";

	return (
		<div className="h-full flex flex-col bg-sidebar">
			<div className="flex items-center justify-end h-[30px] px-3 border-b border-border">
				<div className="flex items-center gap-1">
					<button
						type="button"
						onClick={handleToolbarNewFile}
						className="p-1 hover:bg-sidebar-accent rounded transition-colors"
						title="New File"
						aria-label="New File"
						disabled={!rootPath}
					>
						<FilePlus className="h-3.5 w-3.5 text-muted-foreground" />
					</button>
					<button
						type="button"
						onClick={handleToolbarNewFolder}
						className="p-1 hover:bg-sidebar-accent rounded transition-colors"
						title="New Folder"
						aria-label="New Folder"
						disabled={!rootPath}
					>
						<FolderPlus className="h-3.5 w-3.5 text-muted-foreground" />
					</button>
					<button
						type="button"
						onClick={refresh}
						className="p-1 hover:bg-sidebar-accent rounded transition-colors"
						title="Refresh"
						aria-label="Refresh"
						disabled={!rootPath}
					>
						<RefreshCw className="h-3.5 w-3.5 text-muted-foreground" />
					</button>
					<button
						type="button"
						onClick={collapseAll}
						className="p-1 hover:bg-sidebar-accent rounded transition-colors"
						title="Collapse All"
						aria-label="Collapse All"
						disabled={!rootPath}
					>
						<ChevronsDownUp className="h-3.5 w-3.5 text-muted-foreground" />
					</button>
					<button
						type="button"
						onClick={handleOpenFolder}
						className="p-1 hover:bg-sidebar-accent rounded transition-colors"
						title="Open Folder"
						aria-label="Open Folder"
					>
						<FolderOpen className="h-3.5 w-3.5 text-muted-foreground" />
					</button>
				</div>
			</div>
			<ContextMenu>
				<ContextMenuTrigger asChild>
					<ScrollArea className="flex-1">
						<div className="p-2">
							{loading && (
								<div className="px-2 py-4 text-sm text-muted-foreground">
									Loading...
								</div>
							)}

							{error && (
								<div className="px-2 py-4 text-sm text-destructive">
									{error}
								</div>
							)}

							{!rootPath && !loading && (
								<div className="px-2 py-4 text-center">
									<button
										type="button"
										onClick={handleOpenFolder}
										className="inline-flex items-center gap-2 px-4 py-2 text-sm bg-sidebar-accent hover:bg-sidebar-accent/80 rounded transition-colors"
									>
										<FolderOpen className="h-4 w-4" />
										Open Folder
									</button>
								</div>
							)}

							{rootPath && !loading && !error && (
								<FileTree
									tree={tree}
									selectedPath={selectedPath}
									expandedPaths={expandedPaths}
									onSelect={handleSelectFile}
									onToggleExpand={toggleExpand}
									clipboard={fileOps.clipboard}
									creatingNode={creatingNode}
									renamingPath={renamingPath}
									onContextNewFile={handleContextNewFile}
									onContextNewFolder={handleContextNewFolder}
									onContextCut={(path, type) => fileOps.cut(path, type)}
									onContextCopy={(path, type) => fileOps.copy(path, type)}
									onContextPaste={handleContextPaste}
									onContextCopyPath={(path) => fileOps.copyPath(path)}
									onContextCopyRelativePath={handleContextCopyRelativePath}
									onContextRename={(path) => setRenamingPath(path)}
									onContextDelete={(path) => setDeletingPath(path)}
									onContextRevealInFinder={(path) =>
										fileOps.revealInFinder(path)
									}
									onCreateCommit={handleCreateCommit}
									onCreateCancel={handleCreateCancel}
									onRenameCommit={handleRenameCommit}
									onRenameCancel={handleRenameCancel}
								/>
							)}
						</div>
					</ScrollArea>
				</ContextMenuTrigger>
				{rootPath && (
					<ContextMenuContent className="w-56">
						<ContextMenuItem
							onClick={() => {
								setCreatingNode({ parentPath: rootPath, type: "file" });
							}}
						>
							新規ファイル
						</ContextMenuItem>
						<ContextMenuItem
							onClick={() => {
								setCreatingNode({
									parentPath: rootPath,
									type: "folder",
								});
							}}
						>
							新規フォルダ
						</ContextMenuItem>
						{fileOps.clipboard && (
							<>
								<ContextMenuSeparator />
								<ContextMenuItem
									onClick={() => {
										fileOps.paste(rootPath);
									}}
								>
									貼り付け
								</ContextMenuItem>
							</>
						)}
					</ContextMenuContent>
				)}
			</ContextMenu>
			<DeleteConfirmDialog
				open={!!deletingPath}
				itemName={deletingName}
				onConfirm={handleDeleteConfirm}
				onCancel={handleDeleteCancel}
			/>
		</div>
	);
}
