import {
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
} from "@/components/ui/context-menu";
import type { FileClipboard } from "@/hooks/useFileOperations";

export interface SidebarRootContextMenuProps {
	rootPath: string;
	clipboard: FileClipboard | null;
	onNewFile: () => void;
	onNewFolder: () => void;
	onPaste: () => void;
}

export function SidebarRootContextMenu({
	rootPath,
	clipboard,
	onNewFile,
	onNewFolder,
	onPaste,
}: SidebarRootContextMenuProps) {
	if (!rootPath) return null;

	return (
		<ContextMenuContent className="w-56">
			<ContextMenuItem onClick={onNewFile}>新規ファイル</ContextMenuItem>
			<ContextMenuItem onClick={onNewFolder}>新規フォルダ</ContextMenuItem>
			{clipboard && (
				<>
					<ContextMenuSeparator />
					<ContextMenuItem onClick={onPaste}>貼り付け</ContextMenuItem>
				</>
			)}
		</ContextMenuContent>
	);
}
