import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";

interface SourceControlContextMenuProps {
	children: React.ReactNode;
	variant: "unstaged" | "staged";
	onOpenChanges: () => void;
	onStage?: () => void;
	onUnstage?: () => void;
	onDiscard?: () => void;
	onCopyPath: () => void;
	onCopyRelativePath: () => void;
	onRevealInFinder: () => void;
}

export function SourceControlContextMenu({
	children,
	variant,
	onOpenChanges,
	onStage,
	onUnstage,
	onDiscard,
	onCopyPath,
	onCopyRelativePath,
	onRevealInFinder,
}: SourceControlContextMenuProps) {
	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				{/* biome-ignore lint/a11y/noStaticElementInteractions: wrapper to stop propagation to background context menu */}
				<div onContextMenu={(e) => e.stopPropagation()}>{children}</div>
			</ContextMenuTrigger>
			<ContextMenuContent className="w-56">
				<ContextMenuItem onClick={onOpenChanges}>変更を開く</ContextMenuItem>
				{variant === "unstaged" && onStage && (
					<ContextMenuItem onClick={onStage}>ステージ</ContextMenuItem>
				)}
				{variant === "staged" && onUnstage && (
					<ContextMenuItem onClick={onUnstage}>アンステージ</ContextMenuItem>
				)}
				{variant === "unstaged" && onDiscard && (
					<ContextMenuItem onClick={onDiscard} variant="destructive">
						変更を破棄
					</ContextMenuItem>
				)}
				<ContextMenuSeparator />
				<ContextMenuItem onClick={onCopyPath}>パスをコピー</ContextMenuItem>
				<ContextMenuItem onClick={onCopyRelativePath}>
					相対パスをコピー
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onClick={onRevealInFinder}>
					Finder で表示
				</ContextMenuItem>
			</ContextMenuContent>
		</ContextMenu>
	);
}
