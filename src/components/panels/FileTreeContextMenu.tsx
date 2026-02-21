import {
	Clipboard,
	ClipboardPaste,
	Copy,
	ExternalLink,
	FilePlus,
	FolderPlus,
	Pencil,
	Scissors,
	Trash2,
} from "lucide-react";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import type { FileClipboard } from "@/hooks/useFileOperations";

interface FileTreeContextMenuProps {
	children: React.ReactNode;
	nodeType: "file" | "folder";
	clipboard: FileClipboard | null;
	onNewFile: () => void;
	onNewFolder: () => void;
	onCut: () => void;
	onCopy: () => void;
	onPaste: () => void;
	onCopyPath: () => void;
	onCopyRelativePath: () => void;
	onRename: () => void;
	onDelete: () => void;
	onRevealInFinder: () => void;
}

export function FileTreeContextMenu({
	children,
	nodeType,
	clipboard,
	onNewFile,
	onNewFolder,
	onCut,
	onCopy,
	onPaste,
	onCopyPath,
	onCopyRelativePath,
	onRename,
	onDelete,
	onRevealInFinder,
}: FileTreeContextMenuProps) {
	const isFolder = nodeType === "folder";

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				{/* biome-ignore lint/a11y/noStaticElementInteractions: wrapper to stop propagation to background context menu */}
				<div onContextMenu={(e) => e.stopPropagation()}>{children}</div>
			</ContextMenuTrigger>
			<ContextMenuContent className="w-56">
				{isFolder && (
					<>
						<ContextMenuItem onClick={onNewFile}>
							<FilePlus />
							New File
						</ContextMenuItem>
						<ContextMenuItem onClick={onNewFolder}>
							<FolderPlus />
							New Folder
						</ContextMenuItem>
						<ContextMenuSeparator />
					</>
				)}
				<ContextMenuItem onClick={onCut}>
					<Scissors />
					Cut
				</ContextMenuItem>
				<ContextMenuItem onClick={onCopy}>
					<Copy />
					Copy
				</ContextMenuItem>
				{clipboard && (
					<ContextMenuItem onClick={onPaste}>
						<ClipboardPaste />
						Paste
					</ContextMenuItem>
				)}
				<ContextMenuSeparator />
				<ContextMenuItem onClick={onCopyPath}>
					<Clipboard />
					Copy Path
				</ContextMenuItem>
				<ContextMenuItem onClick={onCopyRelativePath}>
					<Clipboard />
					Copy Relative Path
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onClick={onRename}>
					<Pencil />
					Rename
				</ContextMenuItem>
				<ContextMenuItem onClick={onDelete} variant="destructive">
					<Trash2 />
					Delete
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onClick={onRevealInFinder}>
					<ExternalLink />
					Reveal in Finder
				</ContextMenuItem>
			</ContextMenuContent>
		</ContextMenu>
	);
}
