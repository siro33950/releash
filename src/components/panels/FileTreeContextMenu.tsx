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
						<ContextMenuItem onClick={onNewFile}>新規ファイル</ContextMenuItem>
						<ContextMenuItem onClick={onNewFolder}>
							新規フォルダ
						</ContextMenuItem>
						<ContextMenuSeparator />
					</>
				)}
				<ContextMenuItem onClick={onCut}>切り取り</ContextMenuItem>
				<ContextMenuItem onClick={onCopy}>コピー</ContextMenuItem>
				{clipboard && (
					<ContextMenuItem onClick={onPaste}>貼り付け</ContextMenuItem>
				)}
				<ContextMenuSeparator />
				<ContextMenuItem onClick={onCopyPath}>パスをコピー</ContextMenuItem>
				<ContextMenuItem onClick={onCopyRelativePath}>
					相対パスをコピー
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onClick={onRename}>名前の変更</ContextMenuItem>
				<ContextMenuItem onClick={onDelete} variant="destructive">
					削除
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onClick={onRevealInFinder}>
					Finder で表示
				</ContextMenuItem>
			</ContextMenuContent>
		</ContextMenu>
	);
}
