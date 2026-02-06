import { MessageSquare } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import type { LineComment } from "@/types/comment";

export interface CommentListProps {
	comments: LineComment[];
	onCommentClick?: (filePath: string, lineNumber: number) => void;
}

export function CommentList({ comments, onCommentClick }: CommentListProps) {
	if (comments.length === 0) {
		return (
			<div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground px-4">
				<MessageSquare className="h-6 w-6" />
				<span className="text-xs font-medium">コメントなし</span>
				<div className="text-[11px] text-center leading-relaxed">
					<p>行番号の左マージンをクリック、またはドラッグで範囲選択</p>
					<p className="mt-0.5">
						<kbd className="px-1 py-0.5 bg-muted rounded text-[10px] font-mono">
							⌘K
						</kbd>{" "}
						でカーソル行にも追加できます
					</p>
				</div>
			</div>
		);
	}

	const grouped = new Map<string, LineComment[]>();
	for (const comment of comments) {
		const existing = grouped.get(comment.filePath);
		if (existing) {
			existing.push(comment);
		} else {
			grouped.set(comment.filePath, [comment]);
		}
	}

	return (
		<ScrollArea className="h-full">
			<div className="p-2">
				{[...grouped.entries()].map(([filePath, fileComments]) => {
					const fileName = filePath.split("/").pop() ?? filePath;
					return (
						<div key={filePath} className="mb-2">
							<div className="text-xs font-medium px-1 py-0.5 truncate">
								{fileName}
							</div>
							{fileComments
								.sort((a, b) => a.lineNumber - b.lineNumber)
								.map((comment) => (
									<button
										type="button"
										key={comment.id}
										onClick={() =>
											onCommentClick?.(comment.filePath, comment.lineNumber)
										}
										className={cn(
											"flex items-start gap-1.5 w-full px-1 py-1 text-[11px] rounded transition-colors",
											"hover:bg-muted text-left",
										)}
									>
										<MessageSquare className="h-3 w-3 shrink-0 mt-0.5 text-muted-foreground" />
										<div className="min-w-0 flex-1">
											<div className="flex items-center gap-1">
												<span className="text-muted-foreground font-mono">
													L{comment.lineNumber}
													{comment.endLine != null ? `-${comment.endLine}` : ""}
												</span>
												<span
													className={cn(
														"text-[10px] px-1 rounded",
														comment.status === "sent"
															? "bg-status-added/20 text-status-added"
															: "bg-muted text-muted-foreground",
													)}
												>
													{comment.status === "sent" ? "sent" : "unsent"}
												</span>
											</div>
											<div className="truncate text-foreground">
												{comment.content}
											</div>
										</div>
									</button>
								))}
						</div>
					);
				})}
			</div>
		</ScrollArea>
	);
}
