import {
	ArrowDown,
	ArrowUp,
	ChevronDown,
	ChevronRight,
	ChevronsDownUp,
	ChevronsUpDown,
	Folder,
	Minus,
	Plus,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import type { DiffTreeNode } from "@/hooks/useDiffFileTree";
import { cn } from "@/lib/utils";
import type { DiffBase, DiffSection } from "@/types/settings";

interface DiffFileTreeProps {
	rootPath: string;
	stagedTree: DiffTreeNode[];
	changesTree: DiffTreeNode[];
	branchBaseTree: DiffTreeNode[];
	stagedFileCount: number;
	changesFileCount: number;
	diffBase: DiffBase;
	selectedFile: string | null;
	selectedSection: DiffSection;
	onSelectFile: (path: string, section: DiffSection) => void;
	onStageFile?: (path: string) => void;
	onUnstageFile?: (path: string) => void;
	onStageAll?: () => void;
	onUnstageAll?: () => void;
}

interface TreeNodeProps {
	node: DiffTreeNode;
	depth: number;
	rootPath: string;
	selectedFile: string | null;
	selectedSection: DiffSection;
	section: DiffSection;
	onSelectFile: (path: string, section: DiffSection) => void;
	onFileAction?: (path: string) => void;
	fileActionIcon: "plus" | "minus";
	expanded: Set<string>;
	onToggle: (path: string) => void;
}

function formatFileName(name: string): { dir: string; baseName: string } {
	const lastSlash = name.lastIndexOf("/");
	if (lastSlash === -1) return { dir: "", baseName: name };
	return {
		dir: `${name.substring(0, lastSlash)}/`,
		baseName: name.substring(lastSlash + 1),
	};
}

function TreeNode({
	node,
	depth,
	rootPath,
	selectedFile,
	selectedSection,
	section,
	onSelectFile,
	onFileAction,
	fileActionIcon,
	expanded,
	onToggle,
}: TreeNodeProps) {
	const isFolder = node.node_type === "folder";
	const isExpanded = expanded.has(node.path);
	const isSelected =
		!isFolder && selectedFile === node.path && selectedSection === section;
	const paddingLeft = depth * 12 + 8;

	if (isFolder) {
		return (
			<>
				<button
					type="button"
					className="flex w-full items-center gap-1 py-0.5 text-xs hover:bg-foreground/5 transition-colors"
					style={{ paddingLeft }}
					onClick={() => onToggle(node.path)}
				>
					{isExpanded ? (
						<ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
					) : (
						<ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
					)}
					<Folder className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
					<span className="truncate text-muted-foreground">{node.name}</span>
				</button>
				{isExpanded &&
					node.children.map((child) => (
						<TreeNode
							key={child.id}
							node={child}
							depth={depth + 1}
							rootPath={rootPath}
							selectedFile={selectedFile}
							selectedSection={selectedSection}
							section={section}
							onSelectFile={onSelectFile}
							onFileAction={onFileAction}
							fileActionIcon={fileActionIcon}
							expanded={expanded}
							onToggle={onToggle}
						/>
					))}
			</>
		);
	}

	// File node
	const { baseName } = formatFileName(node.name);
	const ActionIcon = fileActionIcon === "plus" ? Plus : Minus;
	const actionColor =
		fileActionIcon === "plus"
			? "text-status-added/50 hover:text-status-added"
			: "text-status-modified/50 hover:text-status-modified";

	const handleCopyRelativePath = async () => {
		try {
			await navigator.clipboard.writeText(node.path);
		} catch {}
	};

	const handleCopyAbsolutePath = async () => {
		try {
			await navigator.clipboard.writeText(`${rootPath}/${node.path}`);
		} catch {}
	};

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<div
					className={cn(
						"group relative flex w-full items-center py-0.5 px-1 text-xs transition-colors",
						isSelected ? "bg-foreground/10" : "hover:bg-foreground/5",
					)}
					style={{ paddingLeft: paddingLeft + 16 }}
				>
					<button
						type="button"
						className="flex flex-1 items-center gap-1 min-w-0"
						onClick={() => onSelectFile(node.path, section)}
					>
						<span className="truncate">{baseName}</span>
						{node.additions != null && node.deletions != null && (
							<span className="ml-auto shrink-0 text-[10px] font-mono text-muted-foreground tabular-nums">
								<span className="text-status-untracked">+{node.additions}</span>{" "}
								<span className="text-status-deleted">-{node.deletions}</span>
							</span>
						)}
					</button>
					{onFileAction && (
						<button
							type="button"
							className={cn("ml-1 shrink-0 transition-colors", actionColor)}
							onClick={(e) => {
								e.stopPropagation();
								onFileAction(node.path);
							}}
							title={fileActionIcon === "plus" ? "Stage file" : "Unstage file"}
							aria-label={
								fileActionIcon === "plus" ? "Stage file" : "Unstage file"
							}
						>
							<ActionIcon className="h-3.5 w-3.5" />
						</button>
					)}
				</div>
			</ContextMenuTrigger>
			<ContextMenuContent>
				<ContextMenuItem onClick={handleCopyRelativePath}>
					Copy Relative Path
				</ContextMenuItem>
				<ContextMenuItem onClick={handleCopyAbsolutePath}>
					Copy Absolute Path
				</ContextMenuItem>
			</ContextMenuContent>
		</ContextMenu>
	);
}

function collectAllFolderPaths(nodes: DiffTreeNode[]): Set<string> {
	const paths = new Set<string>();
	const collect = (ns: DiffTreeNode[]) => {
		for (const node of ns) {
			if (node.node_type === "folder") {
				paths.add(node.path);
				collect(node.children);
			}
		}
	};
	collect(nodes);
	return paths;
}

function ExpandCollapseButtons({
	onExpandAll,
	onCollapseAll,
}: {
	onExpandAll: () => void;
	onCollapseAll: () => void;
}) {
	return (
		<>
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon-xs"
						onClick={onExpandAll}
						aria-label="Expand All"
						className="h-5 w-5 text-muted-foreground hover:text-foreground"
					>
						<ChevronsUpDown className="h-3 w-3" />
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom" className="text-xs">
					Expand All
				</TooltipContent>
			</Tooltip>
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon-xs"
						onClick={onCollapseAll}
						aria-label="Collapse All"
						className="h-5 w-5 text-muted-foreground hover:text-foreground"
					>
						<ChevronsDownUp className="h-3 w-3" />
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom" className="text-xs">
					Collapse All
				</TooltipContent>
			</Tooltip>
		</>
	);
}

function SectionHeader({
	label,
	count,
	isExpanded,
	onToggle,
	actionLabel,
	actionIcon: ActionIcon,
	onAction,
	hasFolders,
	onExpandAll,
	onCollapseAll,
}: {
	label: string;
	count: number;
	isExpanded: boolean;
	onToggle: () => void;
	actionLabel?: string;
	actionIcon?: React.ComponentType<{ className?: string }>;
	onAction?: () => void;
	hasFolders?: boolean;
	onExpandAll?: () => void;
	onCollapseAll?: () => void;
}) {
	return (
		<div className="flex items-center justify-between px-1.5 py-1 border-b border-border bg-muted/50">
			<button
				type="button"
				className="flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors"
				onClick={onToggle}
			>
				{isExpanded ? (
					<ChevronDown className="h-3 w-3 shrink-0" />
				) : (
					<ChevronRight className="h-3 w-3 shrink-0" />
				)}
				{label}
				<span className="text-[10px] font-normal">({count})</span>
			</button>
			<div className="flex items-center gap-0.5">
				{hasFolders && onExpandAll && onCollapseAll && (
					<ExpandCollapseButtons
						onExpandAll={onExpandAll}
						onCollapseAll={onCollapseAll}
					/>
				)}
				{onAction && actionLabel && ActionIcon && count > 0 && (
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon-xs"
								onClick={onAction}
								aria-label={actionLabel}
								className="h-5 w-5 text-muted-foreground hover:text-foreground"
							>
								<ActionIcon className="h-3 w-3" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="text-xs">
							{actionLabel}
						</TooltipContent>
					</Tooltip>
				)}
			</div>
		</div>
	);
}

function useTreeExpand(tree: DiffTreeNode[]) {
	const [expanded, setExpanded] = useState<Set<string>>(() =>
		collectAllFolderPaths(tree),
	);

	useEffect(() => {
		setExpanded(collectAllFolderPaths(tree));
	}, [tree]);

	const handleToggle = useCallback((path: string) => {
		setExpanded((prev) => {
			const next = new Set(prev);
			if (next.has(path)) {
				next.delete(path);
			} else {
				next.add(path);
			}
			return next;
		});
	}, []);

	const expandAll = useCallback(() => {
		setExpanded(collectAllFolderPaths(tree));
	}, [tree]);

	const collapseAll = useCallback(() => {
		setExpanded(new Set());
	}, []);

	const hasFolders = tree.some((n) => n.node_type === "folder");

	return { expanded, handleToggle, expandAll, collapseAll, hasFolders };
}

function TreeSection({
	tree,
	section,
	rootPath,
	selectedFile,
	selectedSection,
	onSelectFile,
	onFileAction,
	fileActionIcon,
	expandState,
}: {
	tree: DiffTreeNode[];
	section: DiffSection;
	rootPath: string;
	selectedFile: string | null;
	selectedSection: DiffSection;
	onSelectFile: (path: string, section: DiffSection) => void;
	onFileAction?: (path: string) => void;
	fileActionIcon: "plus" | "minus";
	expandState: { expanded: Set<string>; handleToggle: (path: string) => void };
}) {
	return (
		<>
			{tree.map((node) => (
				<TreeNode
					key={node.id}
					node={node}
					depth={0}
					rootPath={rootPath}
					selectedFile={selectedFile}
					selectedSection={selectedSection}
					section={section}
					onSelectFile={onSelectFile}
					onFileAction={onFileAction}
					fileActionIcon={fileActionIcon}
					expanded={expandState.expanded}
					onToggle={expandState.handleToggle}
				/>
			))}
		</>
	);
}

export function DiffFileTree({
	rootPath,
	stagedTree,
	changesTree,
	branchBaseTree,
	stagedFileCount,
	changesFileCount,
	diffBase,
	selectedFile,
	selectedSection,
	onSelectFile,
	onStageFile,
	onUnstageFile,
	onStageAll,
	onUnstageAll,
}: DiffFileTreeProps) {
	const [stagedSectionOpen, setStagedSectionOpen] = useState(true);
	const [changesSectionOpen, setChangesSectionOpen] = useState(true);
	const changesExpand = useTreeExpand(changesTree);
	const stagedExpand = useTreeExpand(stagedTree);

	// Branch Base mode: flat tree with no sections
	if (diffBase === "branch-base") {
		return (
			<BranchBaseTree
				rootPath={rootPath}
				tree={branchBaseTree}
				selectedFile={selectedFile}
				onSelectFile={onSelectFile}
			/>
		);
	}

	// HEAD mode: two sections — Unstaged on top, Staged on bottom
	return (
		<div
			className="flex flex-col h-full select-none"
			data-testid="diff-file-tree"
		>
			{/* Unstaged section (changesFiles → Stage operations) */}
			<div className="flex flex-col flex-1 min-h-0">
				<SectionHeader
					label="Unstaged"
					count={changesFileCount}
					isExpanded={changesSectionOpen}
					onToggle={() => setChangesSectionOpen((v) => !v)}
					actionLabel="Stage All"
					actionIcon={ArrowDown}
					onAction={onStageAll}
					hasFolders={changesExpand.hasFolders}
					onExpandAll={changesExpand.expandAll}
					onCollapseAll={changesExpand.collapseAll}
				/>
				{changesSectionOpen && changesFileCount > 0 && (
					<div className="flex-1 min-h-0 overflow-hidden">
						<ScrollArea className="h-full">
							<TreeSection
								tree={changesTree}
								section="changes"
								rootPath={rootPath}
								selectedFile={selectedFile}
								selectedSection={selectedSection}
								onSelectFile={onSelectFile}
								onFileAction={onStageFile}
								fileActionIcon="plus"
								expandState={changesExpand}
							/>
						</ScrollArea>
					</div>
				)}
			</div>

			{/* Staged section (stagedFiles → Unstage operations) */}
			<div
				className={cn(
					"flex flex-col",
					stagedSectionOpen ? "flex-1 min-h-0" : "shrink-0",
				)}
			>
				<SectionHeader
					label="Staged"
					count={stagedFileCount}
					isExpanded={stagedSectionOpen}
					onToggle={() => setStagedSectionOpen((v) => !v)}
					actionLabel="Unstage All"
					actionIcon={ArrowUp}
					onAction={onUnstageAll}
					hasFolders={stagedExpand.hasFolders}
					onExpandAll={stagedExpand.expandAll}
					onCollapseAll={stagedExpand.collapseAll}
				/>
				{stagedSectionOpen && stagedFileCount > 0 && (
					<div className="flex-1 min-h-0 overflow-hidden">
						<ScrollArea className="h-full">
							<TreeSection
								tree={stagedTree}
								section="staged"
								rootPath={rootPath}
								selectedFile={selectedFile}
								selectedSection={selectedSection}
								onSelectFile={onSelectFile}
								onFileAction={onUnstageFile}
								fileActionIcon="minus"
								expandState={stagedExpand}
							/>
						</ScrollArea>
					</div>
				)}
			</div>
		</div>
	);
}

/** Branch Base mode uses a simple flat tree without sections */
function BranchBaseTree({
	rootPath,
	tree,
	selectedFile,
	onSelectFile,
}: {
	rootPath: string;
	tree: DiffTreeNode[];
	selectedFile: string | null;
	onSelectFile: (path: string, section: DiffSection) => void;
}) {
	const { expanded, handleToggle, expandAll, collapseAll, hasFolders } =
		useTreeExpand(tree);

	return (
		<ScrollArea className="select-none h-full" data-testid="diff-file-tree">
			{hasFolders && (
				<div className="flex items-center justify-end gap-0.5 px-1 py-0.5 border-b border-border">
					<ExpandCollapseButtons
						onExpandAll={expandAll}
						onCollapseAll={collapseAll}
					/>
				</div>
			)}
			{tree.map((node) => (
				<TreeNode
					key={node.id}
					node={node}
					depth={0}
					rootPath={rootPath}
					selectedFile={selectedFile}
					selectedSection="changes"
					section="changes"
					onSelectFile={onSelectFile}
					fileActionIcon="plus"
					expanded={expanded}
					onToggle={handleToggle}
				/>
			))}
		</ScrollArea>
	);
}
