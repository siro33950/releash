import {
	ChevronDown,
	ChevronUp,
	Copy,
	MessageSquare,
	Send,
	Terminal,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { CommentList } from "@/components/panels/CommentList";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import type { Theme } from "@/types/settings";
import type { Thread } from "@/types/thread";

export type RightBottomTab = "terminal" | "comments";

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
	initialActiveTab?: RightBottomTab;
	onActiveTabChange?: (tab: RightBottomTab) => void;
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
	initialActiveTab,
	onActiveTabChange,
}: RightSidebarBottomProps) {
	const [activeTab, setActiveTab] = useState<RightBottomTab>(
		initialActiveTab ?? "terminal",
	);

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
			<Tabs
				value={activeTab}
				onValueChange={(val) => {
					const tab = val as RightBottomTab;
					setActiveTab(tab);
					onActiveTabChange?.(tab);
				}}
				className="flex flex-col h-full"
			>
				<div className="flex items-center gap-2 shrink-0 px-0 pt-0 bg-background">
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
					<TabsList variant="line" aria-label="Bottom sidebar tabs">
						<TabsTrigger value="terminal" aria-label="Terminal">
							<span className="inline-flex items-center">
								<Terminal className="size-3.5" />
							</span>
						</TabsTrigger>
						<TabsTrigger value="comments" aria-label="Comments">
							<span className="inline-flex items-center gap-1.5">
								<MessageSquare className="size-3.5" />
								{unresolvedThreads.length > 0 && (
									<span className="px-1 text-[10px] bg-primary/20 text-primary rounded">
										{unresolvedThreads.length}
									</span>
								)}
							</span>
						</TabsTrigger>
					</TabsList>
				</div>
				<TabsContent
					value="terminal"
					forceMount
					className="flex-1 overflow-hidden data-[state=inactive]:hidden"
				>
					<TerminalPanel cwd={rootPath} theme={theme} />
				</TabsContent>
				<TabsContent
					value="comments"
					className="flex-1 overflow-hidden flex flex-col"
				>
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
				</TabsContent>
			</Tabs>
		</div>
	);
}
