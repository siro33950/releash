import {
	ChevronsDownUp,
	FilePlus,
	FolderOpen,
	FolderPlus,
	RefreshCw,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useFileTree } from "@/hooks/useFileTree";
import { useGitStatus } from "@/hooks/useGitStatus";
import { applyStatusToTree } from "@/lib/applyStatusToTree";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import { FileTree } from "./FileTree";
import { SidebarRootContextMenu } from "./SidebarRootContextMenu";
import { useFileContextActions } from "./useFileContextActions";

export interface SidebarPanelProps {
	rootPath: string;
	onSelectFile?: (path: string) => void;
	onFileChange?: (path: string) => void;
	onRename?: (oldPath: string, newPath: string) => void;
	onDelete?: (path: string) => void;
	requestNewFolderKey?: number;
	activeTabPath?: string | null;
}

export function SidebarPanel({
	rootPath,
	onSelectFile,
	onFileChange,
	onRename,
	onDelete,
	requestNewFolderKey,
	activeTabPath,
}: SidebarPanelProps) {
	const [selectedPath, setSelectedPath] = useState<string | null>(null);

	const prevRootPathRef = useRef(rootPath);
	if (prevRootPathRef.current !== rootPath) {
		prevRootPathRef.current = rootPath;
		if (selectedPath !== null) setSelectedPath(null);
	}

	const lastClickedPathRef = useRef<string | null>(null);

	const {
		tree,
		expandedPaths,
		loading,
		error,
		toggleExpand,
		addExpandedPath,
		refresh,
		collapseAll,
		revealPath,
	} = useFileTree({
		rootPath,
		onFileChange: onFileChange
			? (event) => onFileChange(event.path)
			: undefined,
	});

	const { statusMap } = useGitStatus(rootPath);

	const {
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
	} = useFileContextActions({
		rootPath,
		tree,
		selectedPath,
		addExpandedPath,
		onRename,
		onDelete,
	});

	const handleSelectFile = useCallback(
		(path: string) => {
			lastClickedPathRef.current = path;
			setSelectedPath(path);
			onSelectFile?.(path);
		},
		[onSelectFile],
	);

	useEffect(() => {
		if (requestNewFolderKey && requestNewFolderKey > 0) {
			handleToolbarNewFolder();
		}
	}, [requestNewFolderKey, handleToolbarNewFolder]);

	useEffect(() => {
		if (!activeTabPath) return;
		if (lastClickedPathRef.current === activeTabPath) {
			lastClickedPathRef.current = null;
			return;
		}
		lastClickedPathRef.current = null;

		setSelectedPath(activeTabPath);

		revealPath(activeTabPath).then(() => {
			requestAnimationFrame(() => {
				const el = document.querySelector(
					`[data-filepath="${CSS.escape(activeTabPath)}"]`,
				);
				el?.scrollIntoView({ block: "nearest", behavior: "smooth" });
			});
		});
	}, [activeTabPath, revealPath]);

	const treeWithStatus = useMemo(
		() => applyStatusToTree(tree, statusMap),
		[tree, statusMap],
	);

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
				</div>
			</div>
			<ContextMenu>
				<ContextMenuTrigger asChild>
					<ScrollArea className="flex-1 min-h-0">
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

							{!loading && !error && treeWithStatus.length === 0 && (
								<EmptyState
									icon={FolderOpen}
									title="No files"
									description="Add files to the repository to get started"
								/>
							)}

							{!loading && !error && treeWithStatus.length > 0 && (
								<FileTree
									rootPath={rootPath}
									tree={treeWithStatus}
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
				<SidebarRootContextMenu
					rootPath={rootPath}
					clipboard={fileOps.clipboard}
					onNewFile={() => {
						setCreatingNode({ parentPath: rootPath, type: "file" });
					}}
					onNewFolder={() => {
						setCreatingNode({
							parentPath: rootPath,
							type: "folder",
						});
					}}
					onPaste={() => {
						fileOps.paste(rootPath);
					}}
				/>
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
