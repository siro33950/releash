import {
	Clipboard,
	ExternalLink,
	FilePen,
	Minus,
	Plus,
	Undo2,
} from "lucide-react";
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
				<ContextMenuItem onClick={onOpenChanges}>
					<FilePen />
					Open Changes
				</ContextMenuItem>
				{variant === "unstaged" && onStage && (
					<ContextMenuItem onClick={onStage}>
						<Plus />
						Stage
					</ContextMenuItem>
				)}
				{variant === "staged" && onUnstage && (
					<ContextMenuItem onClick={onUnstage}>
						<Minus />
						アンStage
					</ContextMenuItem>
				)}
				{variant === "unstaged" && onDiscard && (
					<ContextMenuItem onClick={onDiscard} variant="destructive">
						<Undo2 />
						Discard
					</ContextMenuItem>
				)}
				<ContextMenuSeparator />
				<ContextMenuItem onClick={onCopyPath}>
					<Clipboard />
					Copy Path
				</ContextMenuItem>
				<ContextMenuItem onClick={onCopyRelativePath}>
					<Clipboard />
					相対Copy Path
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
