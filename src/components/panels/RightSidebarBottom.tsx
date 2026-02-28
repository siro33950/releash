import {
	ChevronDown,
	ChevronUp,
	MessageSquare,
	Play,
	Send,
	Terminal,
} from "lucide-react";
import { useState } from "react";
import { CommentList } from "@/components/panels/CommentList";
import { ReviewModal } from "@/components/panels/ReviewModal";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { LineComment } from "@/types/comment";
import type { AppSettings, Theme } from "@/types/settings";

type RightBottomTab = "terminal" | "review";

interface RightSidebarBottomProps {
	rootPath: string;
	theme?: Theme;
	settings: AppSettings;
	comments: LineComment[];
	onCommentClick?: (filePath: string, lineNumber: number) => void;
	onSendToTerminal?: (comments: LineComment[]) => void;
	showSentComments?: boolean;
	onToggleShowSent?: () => void;
	onToggleCollapse?: () => void;
	collapsed?: boolean;
}

export function RightSidebarBottom({
	rootPath,
	theme,
	settings,
	comments,
	onCommentClick,
	onSendToTerminal,
	showSentComments,
	onToggleShowSent,
	onToggleCollapse,
	collapsed,
}: RightSidebarBottomProps) {
	const [activeTab, setActiveTab] = useState<RightBottomTab>("terminal");
	const [reviewModalOpen, setReviewModalOpen] = useState(false);
	const unsentComments = comments.filter((c) => c.status === "unsent");

	return (
		<div className="flex flex-col h-full">
			<Tabs
				value={activeTab}
				onValueChange={(val) => setActiveTab(val as RightBottomTab)}
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
						<TabsTrigger value="review" aria-label="Review">
							<span className="inline-flex items-center gap-1.5">
								<MessageSquare className="size-3.5" />
								{unsentComments.length > 0 && (
									<span className="px-1 text-[10px] bg-primary/20 text-primary rounded">
										{unsentComments.length}
									</span>
								)}
							</span>
						</TabsTrigger>
					</TabsList>
					{activeTab === "review" &&
						unsentComments.length > 0 &&
						onSendToTerminal && (
							<button
								type="button"
								onClick={() => onSendToTerminal(unsentComments)}
								className="flex items-center gap-1 px-2 ml-auto text-[10px] bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
								title="Send unsent comments to terminal"
							>
								<Send className="h-3 w-3" />
								Send
							</button>
						)}
				</div>
				<TabsContent
					value="terminal"
					forceMount
					className="flex-1 overflow-hidden data-[state=inactive]:hidden"
				>
					<TerminalPanel cwd={rootPath} theme={theme} />
				</TabsContent>
				<TabsContent
					value="review"
					className="flex-1 overflow-hidden flex flex-col"
				>
					<div className="flex-1 min-h-0 overflow-hidden">
						<CommentList
							comments={comments}
							onCommentClick={onCommentClick}
							showSentComments={showSentComments}
							onToggleShowSent={onToggleShowSent}
						/>
					</div>
					{settings.reviewAgent !== "none" && (
						<div className="shrink-0 px-3 py-2 border-t border-border">
							<button
								type="button"
								onClick={() => setReviewModalOpen(true)}
								className="w-full flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md bg-primary/10 text-primary hover:bg-primary/20 transition-colors"
							>
								<Play className="h-3.5 w-3.5" />
								AI Review
							</button>
						</div>
					)}
				</TabsContent>
			</Tabs>
			<ReviewModal
				open={reviewModalOpen}
				onOpenChange={setReviewModalOpen}
				rootPath={rootPath}
				comments={comments}
				settings={settings}
			/>
		</div>
	);
}
