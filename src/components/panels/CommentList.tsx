import {
	AlertTriangle,
	Eye,
	EyeOff,
	Info,
	Lightbulb,
	MessageSquare,
	XCircle,
} from "lucide-react";
import { useMemo } from "react";
import { EmptyState } from "@/components/ui/empty-state";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import type { CommentSeverity, LineComment } from "@/types/comment";

export interface CommentListProps {
	comments: LineComment[];
	onCommentClick?: (filePath: string, lineNumber: number) => void;
	showSentComments?: boolean;
	onToggleShowSent?: () => void;
}

function SeverityIcon({ severity }: { severity?: CommentSeverity }) {
	switch (severity) {
		case "error":
			return <XCircle className="h-3 w-3 shrink-0 mt-0.5 text-destructive" />;
		case "warning":
			return (
				<AlertTriangle className="h-3 w-3 shrink-0 mt-0.5 text-yellow-500" />
			);
		case "info":
			return <Info className="h-3 w-3 shrink-0 mt-0.5 text-blue-400" />;
		case "suggestion":
			return <Lightbulb className="h-3 w-3 shrink-0 mt-0.5 text-green-400" />;
		default:
			return (
				<MessageSquare className="h-3 w-3 shrink-0 mt-0.5 text-muted-foreground" />
			);
	}
}

export function CommentList({
	comments,
	onCommentClick,
	showSentComments = false,
	onToggleShowSent,
}: CommentListProps) {
	const sentCount = comments.filter((c) => c.status === "sent").length;

	const visibleComments = useMemo(
		() =>
			showSentComments ? comments : comments.filter((c) => c.status !== "sent"),
		[comments, showSentComments],
	);

	if (visibleComments.length === 0 && sentCount === 0) {
		return (
			<EmptyState
				icon={MessageSquare}
				title="No comments"
				description={
					<>
						<p>
							Click the left margin of a line number, or drag to select a range
						</p>
						<p className="mt-0.5">
							<kbd className="px-1 py-0.5 bg-muted rounded text-[10px] font-mono">
								⌘K
							</kbd>{" "}
							to add a comment on the current cursor line
						</p>
					</>
				}
			/>
		);
	}

	const grouped = new Map<string, LineComment[]>();
	for (const comment of visibleComments) {
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
				{sentCount > 0 && onToggleShowSent && (
					<button
						type="button"
						onClick={onToggleShowSent}
						className="flex items-center gap-1 w-full px-1.5 py-1 mb-1.5 text-[11px] text-muted-foreground rounded hover:bg-muted transition-colors"
						data-testid="toggle-sent-comments"
					>
						{showSentComments ? (
							<EyeOff className="h-3 w-3" />
						) : (
							<Eye className="h-3 w-3" />
						)}
						Sent ({sentCount})
					</button>
				)}
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
									// biome-ignore lint/a11y/useSemanticElements: custom styled clickable row
									<div
										role="button"
										tabIndex={0}
										key={comment.id}
										onClick={() =>
											onCommentClick?.(comment.filePath, comment.lineNumber)
										}
										onKeyDown={(e) => {
											if (
												(e.key === "Enter" || e.key === " ") &&
												e.target === e.currentTarget
											) {
												e.preventDefault();
												onCommentClick?.(comment.filePath, comment.lineNumber);
											}
										}}
										className={cn(
											"flex items-start gap-1.5 w-full px-1 py-1 text-[11px] rounded transition-colors",
											"hover:bg-muted text-left",
											comment.resolved && "opacity-50",
										)}
									>
										<SeverityIcon severity={comment.severity} />
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
											<span className="block truncate text-foreground">
												{comment.content}
											</span>
										</div>
									</div>
								))}
						</div>
					);
				})}
			</div>
		</ScrollArea>
	);
}
