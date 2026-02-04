import {
	ChevronDown,
	ChevronRight,
	File,
	Folder,
	FolderOpen,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { FileNode, FileStatus } from "@/types/file-tree";

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
}

function FileTreeItem({
	node,
	depth = 0,
	selectedPath,
	expandedPaths,
	onSelect,
	onToggleExpand,
}: FileTreeItemProps) {
	const isExpanded = expandedPaths.has(node.path);
	const isSelected = selectedPath === node.path;

	const handleClick = () => {
		if (node.type === "folder") {
			onToggleExpand(node.path);
		} else {
			onSelect(node.path);
		}
	};

	return (
		<div>
			<button
				type="button"
				onClick={handleClick}
				className={cn(
					"flex w-full items-center gap-1 px-2 py-1 text-sm hover:bg-sidebar-accent transition-colors",
					isSelected && "bg-sidebar-accent",
					getStatusColor(node.status ?? null),
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
							<FolderOpen className="h-4 w-4 shrink-0 text-status-modified" />
						) : (
							<Folder className="h-4 w-4 shrink-0 text-status-modified" />
						)}
					</>
				) : (
					<>
						<span className="w-4" />
						<File className="h-4 w-4 shrink-0 text-muted-foreground" />
					</>
				)}
				<span className="truncate flex-1 text-left">{node.name}</span>
				{node.status && (
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
			{node.type === "folder" && isExpanded && node.children && (
				<div>
					{node.children.map((child) => (
						<FileTreeItem
							key={child.path}
							node={child}
							depth={depth + 1}
							selectedPath={selectedPath}
							expandedPaths={expandedPaths}
							onSelect={onSelect}
							onToggleExpand={onToggleExpand}
						/>
					))}
				</div>
			)}
		</div>
	);
}

export interface FileTreeProps {
	tree: FileNode[];
	selectedPath: string | null;
	expandedPaths: Set<string>;
	onSelect: (path: string) => void;
	onToggleExpand: (path: string) => void;
}

export function FileTree({
	tree,
	selectedPath,
	expandedPaths,
	onSelect,
	onToggleExpand,
}: FileTreeProps) {
	return (
		<div className="py-1">
			{tree.map((node) => (
				<FileTreeItem
					key={node.path}
					node={node}
					selectedPath={selectedPath}
					expandedPaths={expandedPaths}
					onSelect={onSelect}
					onToggleExpand={onToggleExpand}
				/>
			))}
		</div>
	);
}
