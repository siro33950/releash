import { ChevronDown, ChevronUp } from "lucide-react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { DiffCommentList } from "@/components/panels/DiffCommentList";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { useDiffComments } from "@/hooks/useDiffComments";
import type { ThreadNavigationTarget } from "@/types/diffComment";
import type { Theme } from "@/types/settings";

interface RightSidebarBottomProps {
	rootPath: string;
	theme?: Theme;
	worktreeName: string;
	onThreadClick?: (target: ThreadNavigationTarget) => void;
	onToggleCollapse?: () => void;
	collapsed?: boolean;
}

export function RightSidebarBottom({
	rootPath,
	theme,
	worktreeName,
	onThreadClick,
	onToggleCollapse,
	collapsed,
}: RightSidebarBottomProps) {
	const { comments, deleteThread } = useDiffComments({ worktreeName });

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center gap-2 shrink-0 px-0 pt-0 bg-background border-y border-border">
				{onToggleCollapse && (
					<button
						type="button"
						onClick={onToggleCollapse}
						className="shrink-0 p-1 ml-2 text-muted-foreground hover:text-foreground transition-colors"
						aria-label={collapsed ? "Expand panel" : "Collapse panel"}
					>
						{collapsed ? (
							<ChevronUp className="size-3.5" />
						) : (
							<ChevronDown className="size-3.5" />
						)}
					</button>
				)}
			</div>
			<div className="flex-1 overflow-hidden">
				<Group orientation="horizontal">
					<Panel id="terminal" defaultSize="50%" minSize="20%">
						<div className="h-full overflow-hidden border-r border-border">
							<TerminalPanel cwd={rootPath} theme={theme} />
						</div>
					</Panel>
					<Separator />
					<Panel id="comments" defaultSize="50%" minSize="20%">
						<div className="h-full overflow-hidden">
							<DiffCommentList
								comments={comments}
								onThreadClick={onThreadClick ?? (() => {})}
								onDelete={deleteThread}
							/>
						</div>
					</Panel>
				</Group>
			</div>
		</div>
	);
}
