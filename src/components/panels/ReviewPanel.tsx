import { Send } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/utils";
import type { LineComment } from "@/types/comment";
import type { Theme } from "@/types/settings";
import { CommentList } from "./CommentList";
import { TerminalPanel } from "./TerminalPanel";

export interface ReviewPanelProps {
	comments: LineComment[];
	onCommentClick?: (filePath: string, lineNumber: number) => void;
	onSendToTerminal?: (comments: LineComment[]) => void;
	cwd?: string | null;
	theme?: Theme;
}

type ReviewTab = "terminal" | "comments";

export function ReviewPanel({
	comments,
	onCommentClick,
	onSendToTerminal,
	cwd,
	theme,
}: ReviewPanelProps) {
	const [activeTab, setActiveTab] = useState<ReviewTab>("terminal");
	const unsentComments = comments.filter((c) => c.status === "unsent");

	return (
		<div className="flex flex-col h-full border-t border-border">
			<div className="flex items-center justify-between border-b border-border bg-card">
				<div className="flex items-center" role="tablist">
					<button
						type="button"
						role="tab"
						aria-selected={activeTab === "terminal"}
						onClick={() => setActiveTab("terminal")}
						className={cn(
							"h-[28px] px-3 text-xs transition-colors",
							activeTab === "terminal"
								? "bg-background text-foreground"
								: "bg-sidebar text-muted-foreground hover:bg-sidebar-accent",
						)}
					>
						Terminal
					</button>
					<button
						type="button"
						role="tab"
						aria-selected={activeTab === "comments"}
						onClick={() => setActiveTab("comments")}
						className={cn(
							"h-[28px] px-3 text-xs transition-colors",
							activeTab === "comments"
								? "bg-background text-foreground"
								: "bg-sidebar text-muted-foreground hover:bg-sidebar-accent",
						)}
					>
						Comments
						{unsentComments.length > 0 && (
							<span className="ml-1 px-1 text-[10px] bg-primary/20 text-primary rounded">
								{unsentComments.length}
							</span>
						)}
					</button>
				</div>
				{activeTab === "comments" &&
					unsentComments.length > 0 &&
					onSendToTerminal && (
						<button
							type="button"
							onClick={() => onSendToTerminal(unsentComments)}
							className="flex items-center gap-1 px-2 py-0.5 mr-2 text-[10px] bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
							title="未送信コメントをターミナルに送信"
						>
							<Send className="h-3 w-3" />
							Send
						</button>
					)}
			</div>
			<div
				className="flex-1 overflow-hidden"
				style={{ display: activeTab === "terminal" ? "block" : "none" }}
			>
				<TerminalPanel
					cwd={cwd}
					theme={theme}
					sessionKey={cwd ? `${cwd}::user` : undefined}
				/>
			</div>
			<div
				className="flex-1 overflow-hidden"
				style={{ display: activeTab === "comments" ? "block" : "none" }}
			>
				<CommentList comments={comments} onCommentClick={onCommentClick} />
			</div>
		</div>
	);
}
