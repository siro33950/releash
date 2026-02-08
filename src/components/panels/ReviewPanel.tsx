import { Send } from "lucide-react";
import type { LineComment } from "@/types/comment";
import { CommentList } from "./CommentList";

export interface ReviewPanelProps {
	comments: LineComment[];
	onCommentClick?: (filePath: string, lineNumber: number) => void;
	onSendToTerminal?: (comments: LineComment[]) => void;
}

export function ReviewPanel({
	comments,
	onCommentClick,
	onSendToTerminal,
}: ReviewPanelProps) {
	const unsentComments = comments.filter((c) => c.status === "unsent");

	return (
		<div className="flex flex-col h-full border-t border-border">
			<div className="flex items-center justify-between px-2 py-1 border-b border-border bg-card">
				<span className="text-xs text-muted-foreground">
					Comments
					{unsentComments.length > 0 && (
						<span className="ml-1 px-1 text-[10px] bg-primary/20 text-primary rounded">
							{unsentComments.length}
						</span>
					)}
				</span>
				{unsentComments.length > 0 && onSendToTerminal && (
					<button
						type="button"
						onClick={() => onSendToTerminal(unsentComments)}
						className="flex items-center gap-1 px-2 py-0.5 text-[10px] bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
						title="未送信コメントをターミナルに送信"
					>
						<Send className="h-3 w-3" />
						Send
					</button>
				)}
			</div>
			<div className="flex-1 overflow-hidden">
				<CommentList comments={comments} onCommentClick={onCommentClick} />
			</div>
		</div>
	);
}
