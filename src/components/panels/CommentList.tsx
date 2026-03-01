import {
	AlertTriangle,
	Check,
	Eye,
	EyeOff,
	Info,
	Lightbulb,
	MessageSquare,
	Trash2,
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
	onDeleteComment?: (id: string) => void;
	onResolveComment?: (id: string) => void;
	showResolvedComments?: boolean;
	onToggleShowResolved?: () => void;
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
	onDeleteComment,
	onResolveComment,
	showResolvedComments = false,
	onToggleShowResolved,
}: CommentListProps) {
	const resolvedCount = comments.filter((c) => c.resolved).length;

	const visibleComments = useMemo(
		() =>
			showResolvedComments ? comments : comments.filter((c) => !c.resolved),
		[comments, showResolvedComments],
	);

	if (visibleComments.length === 0 && resolvedCount === 0) {
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
		<ScrollArea className="h-full [&_[data-slot=scroll-area-viewport]>div]:!block">
			<div className="p-2">
				{resolvedCount > 0 && onToggleShowResolved && (
					<button
						type="button"
						onClick={onToggleShowResolved}
						className="flex items-center gap-1 w-full px-1.5 py-1 mb-1.5 text-[11px] text-muted-foreground rounded hover:bg-muted transition-colors"
						data-testid="toggle-resolved-comments"
					>
						{showResolvedComments ? (
							<EyeOff className="h-3 w-3" />
						) : (
							<Eye className="h-3 w-3" />
						)}
						Resolved ({resolvedCount})
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
											"group flex items-start gap-1.5 w-full px-1 py-1 text-[11px] rounded transition-colors",
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
												{(onResolveComment || onDeleteComment) && (
													<div className="hidden group-hover:flex items-center gap-0.5 ml-auto">
														{onResolveComment && !comment.resolved && (
															// biome-ignore lint/a11y/useSemanticElements: nested inside role="button", cannot use <button>
															<span
																role="button"
																tabIndex={0}
																className="p-0.5 rounded hover:bg-green-500/20 text-muted-foreground hover:text-green-500 transition-colors"
																aria-label="Resolve comment"
																onClick={(e) => {
																	e.stopPropagation();
																	onResolveComment(comment.id);
																}}
																onKeyDown={(e) => {
																	if (e.key === "Enter" || e.key === " ") {
																		e.preventDefault();
																		e.stopPropagation();
																		onResolveComment(comment.id);
																	}
																}}
															>
																<Check className="h-3 w-3" />
															</span>
														)}
														{onDeleteComment && (
															// biome-ignore lint/a11y/useSemanticElements: nested inside role="button", cannot use <button>
															<span
																role="button"
																tabIndex={0}
																className="p-0.5 rounded hover:bg-destructive/20 text-muted-foreground hover:text-destructive transition-colors"
																aria-label="Delete comment"
																onClick={(e) => {
																	e.stopPropagation();
																	onDeleteComment(comment.id);
																}}
																onKeyDown={(e) => {
																	if (e.key === "Enter" || e.key === " ") {
																		e.preventDefault();
																		e.stopPropagation();
																		onDeleteComment(comment.id);
																	}
																}}
															>
																<Trash2 className="h-3 w-3" />
															</span>
														)}
													</div>
												)}
											</div>
											<span className="line-clamp-2 text-foreground">
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
