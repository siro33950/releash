import { MessageSquare, Send, Terminal } from "lucide-react";
import { useState } from "react";
import { CommentList } from "@/components/panels/CommentList";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { LineComment } from "@/types/comment";
import type { Theme } from "@/types/settings";

type RightBottomTab = "terminal" | "comments";

interface RightSidebarBottomProps {
	rootPath: string;
	theme?: Theme;
	comments: LineComment[];
	onCommentClick?: (filePath: string, lineNumber: number) => void;
	onDeleteComment?: (id: string) => void;
	onUpdateComment?: (id: string, content: string) => void;
	onSendToTerminal?: (comments: LineComment[]) => void;
	onSendComment?: (comment: LineComment) => void;
	onCopyComment?: (comment: LineComment) => void;
	showSentComments?: boolean;
	onToggleShowSent?: () => void;
}

export function RightSidebarBottom({
	rootPath,
	theme,
	comments,
	onCommentClick,
	onDeleteComment,
	onUpdateComment,
	onSendToTerminal,
	onSendComment,
	onCopyComment,
	showSentComments,
	onToggleShowSent,
}: RightSidebarBottomProps) {
	const [activeTab, setActiveTab] = useState<RightBottomTab>("comments");
	const unsentComments = comments.filter((c) => c.status === "unsent");

	return (
		<div className="flex flex-col h-full">
			<Tabs
				value={activeTab}
				onValueChange={(val) => setActiveTab(val as RightBottomTab)}
				className="flex flex-col h-full gap-0"
			>
				<div className="flex items-center gap-2 shrink-0 px-2 pt-1 bg-background">
					<TabsList variant="line" aria-label="Bottom sidebar tabs">
						<TabsTrigger value="comments" aria-label="Comments">
							<MessageSquare className="size-3.5" />
							{unsentComments.length > 0 && (
								<span className="px-1 text-[10px] bg-primary/20 text-primary rounded">
									{unsentComments.length}
								</span>
							)}
						</TabsTrigger>
						<TabsTrigger value="terminal" aria-label="Terminal">
							<Terminal className="size-3.5" />
						</TabsTrigger>
					</TabsList>
					{activeTab === "comments" &&
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
					<TerminalPanel
						cwd={rootPath}
						theme={theme}
						sessionKey={`${rootPath}::user`}
					/>
				</TabsContent>
				<TabsContent value="comments" className="flex-1 overflow-hidden">
					<CommentList
						comments={comments}
						onCommentClick={onCommentClick}
						onDeleteComment={onDeleteComment}
						onUpdateComment={onUpdateComment}
						onSendComment={onSendComment}
						onCopyComment={onCopyComment}
						showSentComments={showSentComments}
						onToggleShowSent={onToggleShowSent}
					/>
				</TabsContent>
			</Tabs>
		</div>
	);
}
