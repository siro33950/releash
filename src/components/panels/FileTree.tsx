import {
	DefaultFolderOpenedIcon,
	FileIcon,
	FolderIcon,
} from "@react-symbols/icons/utils";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { FileClipboard } from "@/hooks/useFileOperations";
import { cn } from "@/lib/utils";
import type { FileNode, FileStatus } from "@/types/file-tree";
import { FileTreeContextMenu } from "./FileTreeContextMenu";
import { InlineInput } from "./InlineInput";

function getStatusColor(status: FileStatus) {
	switch (status) {
		case "modified":
			return "text-status-modified";
		case "added":
			return "text-status-added";
		case "deleted":
			return "text-status-deleted";
		case "untracked":
			return "text-status-untracked";
		case "ignored":
			return "text-status-ignored";
		default:
			return "text-sidebar-foreground";
	}
}

function getStatusIndicator(status: FileStatus) {
	switch (status) {
		case "modified":
			return "M";
		case "added":
			return "A";
		case "deleted":
			return "D";
		case "untracked":
			return "U";
		case "ignored":
			return null;
		default:
			return null;
	}
}

interface FileTreeItemProps {
	node: FileNode;
	depth?: number;
	selectedPath: string | null;
	expandedPaths: Set<string>;
	onSelect: (path: string) => void;
	onToggleExpand: (path: string) => void;
	clipboard: FileClipboard | null;
	creatingNode: { parentPath: string; type: "file" | "folder" } | null;
	renamingPath: string | null;
	onContextNewFile: (nodePath: string, nodeType: "file" | "folder") => void;
	onContextNewFolder: (nodePath: string, nodeType: "file" | "folder") => void;
	onContextCut: (path: string, type: "file" | "folder") => void;
	onContextCopy: (path: string, type: "file" | "folder") => void;
	onContextPaste: (nodePath: string, nodeType: "file" | "folder") => void;
	onContextCopyPath: (path: string) => void;
	onContextCopyRelativePath: (path: string) => void;
	onContextRename: (path: string) => void;
	onContextDelete: (path: string) => void;
	onContextRevealInFinder: (path: string) => void;
	onCreateCommit: (name: string) => void;
	onCreateCancel: () => void;
	onRenameCommit: (oldPath: string, newName: string) => void;
	onRenameCancel: () => void;
}

function FileTreeItem({
	node,
	depth = 0,
	selectedPath,
	expandedPaths,
	onSelect,
	onToggleExpand,
	clipboard,
	creatingNode,
	renamingPath,
	onContextNewFile,
	onContextNewFolder,
	onContextCut,
	onContextCopy,
	onContextPaste,
	onContextCopyPath,
	onContextCopyRelativePath,
	onContextRename,
	onContextDelete,
	onContextRevealInFinder,
	onCreateCommit,
	onCreateCancel,
	onRenameCommit,
	onRenameCancel,
}: FileTreeItemProps) {
	const isExpanded = expandedPaths.has(node.path);
	const isSelected = selectedPath === node.path;
	const isRenaming = renamingPath === node.path;
	const showCreateInput =
		creatingNode &&
		creatingNode.parentPath === node.path &&
		node.type === "folder" &&
		isExpanded;

	const handleClick = () => {
		if (node.type === "folder") {
			onToggleExpand(node.path);
		} else {
			onSelect(node.path);
		}
	};

	const itemButton = (
		<button
			type="button"
			onClick={handleClick}
			className={cn(
				"flex w-full items-center gap-1 px-2 py-1 text-sm hover:bg-sidebar-accent transition-colors",
				isSelected && "bg-sidebar-accent",
				getStatusColor(node.status ?? null),
				node.status === "ignored" && "opacity-50",
			)}
			style={{ paddingLeft: `${depth * 12 + 8}px` }}
		>
			{node.type === "folder" ? (
				<>
					{isExpanded ? (
						<ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
					) : (
						<ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
					)}
					{isExpanded ? (
						<DefaultFolderOpenedIcon className="h-4 w-4 shrink-0" />
					) : (
						<FolderIcon folderName={node.name} className="h-4 w-4 shrink-0" />
					)}
				</>
			) : (
				<>
					<span className="w-4" />
					<FileIcon fileName={node.name} className="h-4 w-4 shrink-0" />
				</>
			)}
			{isRenaming ? (
				<InlineInput
					defaultValue={node.name}
					onCommit={(newName) => onRenameCommit(node.path, newName)}
					onCancel={onRenameCancel}
					className="flex-1"
				/>
			) : (
				<span className="truncate flex-1 text-left">{node.name}</span>
			)}
			{!isRenaming && node.status && getStatusIndicator(node.status) && (
				<span
					className={cn(
						"text-xs font-mono shrink-0",
						getStatusColor(node.status),
					)}
				>
					{getStatusIndicator(node.status)}
				</span>
			)}
		</button>
	);

	return (
		<div>
			<FileTreeContextMenu
				nodeType={node.type}
				clipboard={clipboard}
				onNewFile={() => onContextNewFile(node.path, node.type)}
				onNewFolder={() => onContextNewFolder(node.path, node.type)}
				onCut={() => onContextCut(node.path, node.type)}
				onCopy={() => onContextCopy(node.path, node.type)}
				onPaste={() => onContextPaste(node.path, node.type)}
				onCopyPath={() => onContextCopyPath(node.path)}
				onCopyRelativePath={() => onContextCopyRelativePath(node.path)}
				onRename={() => onContextRename(node.path)}
				onDelete={() => onContextDelete(node.path)}
				onRevealInFinder={() => onContextRevealInFinder(node.path)}
			>
				{itemButton}
			</FileTreeContextMenu>
			{node.type === "folder" && isExpanded && (
				<div>
					{showCreateInput && (
						<div
							className="flex items-center gap-1 px-2 py-1"
							style={{ paddingLeft: `${(depth + 1) * 12 + 8}px` }}
						>
							{creatingNode.type === "folder" ? (
								<FolderIcon folderName="" className="h-4 w-4 shrink-0" />
							) : (
								<FileIcon fileName="" className="h-4 w-4 shrink-0" />
							)}
							<InlineInput
								onCommit={onCreateCommit}
								onCancel={onCreateCancel}
								className="flex-1"
							/>
						</div>
					)}
					{node.children?.map((child) => (
						<FileTreeItem
							key={child.path}
							node={child}
							depth={depth + 1}
							selectedPath={selectedPath}
							expandedPaths={expandedPaths}
							onSelect={onSelect}
							onToggleExpand={onToggleExpand}
							clipboard={clipboard}
							creatingNode={creatingNode}
							renamingPath={renamingPath}
							onContextNewFile={onContextNewFile}
							onContextNewFolder={onContextNewFolder}
							onContextCut={onContextCut}
							onContextCopy={onContextCopy}
							onContextPaste={onContextPaste}
							onContextCopyPath={onContextCopyPath}
							onContextCopyRelativePath={onContextCopyRelativePath}
							onContextRename={onContextRename}
							onContextDelete={onContextDelete}
							onContextRevealInFinder={onContextRevealInFinder}
							onCreateCommit={onCreateCommit}
							onCreateCancel={onCreateCancel}
							onRenameCommit={onRenameCommit}
							onRenameCancel={onRenameCancel}
						/>
					))}
				</div>
			)}
		</div>
	);
}

export interface FileTreeProps {
	rootPath: string;
	tree: FileNode[];
	selectedPath: string | null;
	expandedPaths: Set<string>;
	onSelect: (path: string) => void;
	onToggleExpand: (path: string) => void;
	clipboard: FileClipboard | null;
	creatingNode: { parentPath: string; type: "file" | "folder" } | null;
	renamingPath: string | null;
	onContextNewFile: (nodePath: string, nodeType: "file" | "folder") => void;
	onContextNewFolder: (nodePath: string, nodeType: "file" | "folder") => void;
	onContextCut: (path: string, type: "file" | "folder") => void;
	onContextCopy: (path: string, type: "file" | "folder") => void;
	onContextPaste: (nodePath: string, nodeType: "file" | "folder") => void;
	onContextCopyPath: (path: string) => void;
	onContextCopyRelativePath: (path: string) => void;
	onContextRename: (path: string) => void;
	onContextDelete: (path: string) => void;
	onContextRevealInFinder: (path: string) => void;
	onCreateCommit: (name: string) => void;
	onCreateCancel: () => void;
	onRenameCommit: (oldPath: string, newName: string) => void;
	onRenameCancel: () => void;
}

export function FileTree({
	rootPath,
	tree,
	selectedPath,
	expandedPaths,
	onSelect,
	onToggleExpand,
	clipboard,
	creatingNode,
	renamingPath,
	onContextNewFile,
	onContextNewFolder,
	onContextCut,
	onContextCopy,
	onContextPaste,
	onContextCopyPath,
	onContextCopyRelativePath,
	onContextRename,
	onContextDelete,
	onContextRevealInFinder,
	onCreateCommit,
	onCreateCancel,
	onRenameCommit,
	onRenameCancel,
}: FileTreeProps) {
	return (
		<div className="py-1">
			{creatingNode && creatingNode.parentPath === rootPath && (
				<div
					className="flex items-center gap-1 px-2 py-1"
					style={{ paddingLeft: "8px" }}
				>
					{creatingNode.type === "folder" ? (
						<FolderIcon folderName="" className="h-4 w-4 shrink-0" />
					) : (
						<>
							<span className="w-4" />
							<FileIcon fileName="" className="h-4 w-4 shrink-0" />
						</>
					)}
					<InlineInput
						onCommit={onCreateCommit}
						onCancel={onCreateCancel}
						className="flex-1"
					/>
				</div>
			)}
			{tree.map((node) => (
				<FileTreeItem
					key={node.path}
					node={node}
					selectedPath={selectedPath}
					expandedPaths={expandedPaths}
					onSelect={onSelect}
					onToggleExpand={onToggleExpand}
					clipboard={clipboard}
					creatingNode={creatingNode}
					renamingPath={renamingPath}
					onContextNewFile={onContextNewFile}
					onContextNewFolder={onContextNewFolder}
					onContextCut={onContextCut}
					onContextCopy={onContextCopy}
					onContextPaste={onContextPaste}
					onContextCopyPath={onContextCopyPath}
					onContextCopyRelativePath={onContextCopyRelativePath}
					onContextRename={onContextRename}
					onContextDelete={onContextDelete}
					onContextRevealInFinder={onContextRevealInFinder}
					onCreateCommit={onCreateCommit}
					onCreateCancel={onCreateCancel}
					onRenameCommit={onRenameCommit}
					onRenameCancel={onRenameCancel}
				/>
			))}
		</div>
	);
}
