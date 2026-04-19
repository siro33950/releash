import { ChevronDown, ChevronUp, Copy, Send } from "lucide-react";
import { useCallback, useMemo } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { CommentList } from "@/components/panels/CommentList";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Button } from "@/components/ui/button";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import type { Theme } from "@/types/settings";
import type { Thread } from "@/types/thread";

interface RightSidebarBottomProps {
	rootPath: string;
	theme?: Theme;
	threads: Thread[];
	onThreadClick?: (filePath: string, lineNumber: number) => void;
	onDeleteThread?: (id: string) => void;
	onResolveThread?: (id: string) => void;
	onSendToTerminal?: (threads: Thread[]) => void;
	showResolvedThreads?: boolean;
	onToggleShowResolved?: () => void;
	onToggleCollapse?: () => void;
	collapsed?: boolean;
}

export function RightSidebarBottom({
	rootPath,
	theme,
	threads,
	onThreadClick,
	onDeleteThread,
	onResolveThread,
	onSendToTerminal,
	showResolvedThreads,
	onToggleShowResolved,
	onToggleCollapse,
	collapsed,
}: RightSidebarBottomProps) {
	const unresolvedThreads = useMemo(
		() => threads.filter((t) => !t.resolved),
		[threads],
	);

	const handleCopyThreads = useCallback(() => {
		const text = formatCommentsForTerminal(unresolvedThreads);
		navigator.clipboard.writeText(text);
	}, [unresolvedThreads]);

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
						<div className="h-full flex flex-col overflow-hidden">
							<div className="flex-1 min-h-0 overflow-hidden">
								<CommentList
									threads={threads}
									onThreadClick={onThreadClick}
									onDeleteThread={onDeleteThread}
									onResolveThread={onResolveThread}
									showResolvedThreads={showResolvedThreads}
									onToggleShowResolved={onToggleShowResolved}
								/>
							</div>
							<div className="shrink-0 px-3 py-2 border-t border-border flex items-center gap-2">
								<Button
									variant="ghost"
									size="icon-xs"
									onClick={handleCopyThreads}
									disabled={unresolvedThreads.length === 0}
									aria-label="Copy comments to clipboard"
								>
									<Copy />
								</Button>
								{onSendToTerminal && (
									<Button
										variant="ghost"
										size="xs"
										onClick={() => onSendToTerminal(unresolvedThreads)}
										disabled={unresolvedThreads.length === 0}
									>
										<Send />
										Send
									</Button>
								)}
							</div>
						</div>
					</Panel>
				</Group>
			</div>
		</div>
	);
}
