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
			<ContextMenuItem onClick={onNewFile}>New File</ContextMenuItem>
			<ContextMenuItem onClick={onNewFolder}>New Folder</ContextMenuItem>
			{clipboard && (
				<>
					<ContextMenuSeparator />
					<ContextMenuItem onClick={onPaste}>Paste</ContextMenuItem>
				</>
			)}
		</ContextMenuContent>
	);
}
