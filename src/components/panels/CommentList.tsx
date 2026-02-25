import {
	Check,
	Copy,
	Eye,
	EyeOff,
	MessageSquare,
	Pencil,
	Send,
	Trash2,
	X,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { EmptyState } from "@/components/ui/empty-state";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import type { LineComment } from "@/types/comment";

export interface CommentListProps {
	comments: LineComment[];
	onCommentClick?: (filePath: string, lineNumber: number) => void;
	onDeleteComment?: (id: string) => void;
	onUpdateComment?: (id: string, content: string) => void;
	onSendComment?: (comment: LineComment) => void;
	onCopyComment?: (comment: LineComment) => void;
	showSentComments?: boolean;
	onToggleShowSent?: () => void;
}

export function CommentList({
	comments,
	onCommentClick,
	onDeleteComment,
	onUpdateComment,
	onSendComment,
	onCopyComment,
	showSentComments = false,
	onToggleShowSent,
}: CommentListProps) {
	const [editingId, setEditingId] = useState<string | null>(null);
	const [editContent, setEditContent] = useState("");

	const startEditing = useCallback(
		(e: React.MouseEvent, comment: LineComment) => {
			e.stopPropagation();
			setEditingId(comment.id);
			setEditContent(comment.content);
		},
		[],
	);

	const cancelEditing = useCallback(() => {
		setEditingId(null);
		setEditContent("");
	}, []);

	const submitEdit = useCallback(() => {
		if (!editingId) return;
		const trimmed = editContent.trim();
		if (!trimmed) return;
		onUpdateComment?.(editingId, trimmed);
		setEditingId(null);
		setEditContent("");
	}, [editingId, editContent, onUpdateComment]);

	const handleDelete = useCallback(
		(e: React.MouseEvent, id: string) => {
			e.stopPropagation();
			onDeleteComment?.(id);
		},
		[onDeleteComment],
	);

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
								.map((comment) => {
									return (
										// biome-ignore lint/a11y/useSemanticElements: button cannot contain interactive descendants
										<div
											role="button"
											tabIndex={0}
											key={comment.id}
											onClick={() => {
												if (editingId !== comment.id) {
													onCommentClick?.(
														comment.filePath,
														comment.lineNumber,
													);
												}
											}}
											onKeyDown={(e) => {
												if (
													(e.key === "Enter" || e.key === " ") &&
													editingId !== comment.id &&
													e.target === e.currentTarget
												) {
													e.preventDefault();
													onCommentClick?.(
														comment.filePath,
														comment.lineNumber,
													);
												}
											}}
											className={cn(
												"group flex items-start gap-1.5 w-full px-1 py-1 text-[11px] rounded transition-colors",
												"hover:bg-muted text-left",
											)}
										>
											<MessageSquare className="h-3 w-3 shrink-0 mt-0.5 text-muted-foreground" />
											<div className="min-w-0 flex-1">
												<div className="flex items-center gap-1">
													<span className="text-muted-foreground font-mono">
														L{comment.lineNumber}
														{comment.endLine != null
															? `-${comment.endLine}`
															: ""}
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
												{editingId === comment.id ? (
													<form
														className="mt-0.5"
														onSubmit={(e) => {
															e.preventDefault();
															submitEdit();
														}}
													>
														<textarea
															ref={(el) => {
																el?.focus();
															}}
															value={editContent}
															onChange={(e) => setEditContent(e.target.value)}
															onKeyDown={(e) => {
																if (e.key === "Enter" && !e.shiftKey) {
																	e.preventDefault();
																	submitEdit();
																}
																if (e.key === "Escape") {
																	cancelEditing();
																}
															}}
															className="w-full px-1 py-0.5 text-[11px] bg-background border border-border rounded resize-none focus:outline-none focus:ring-1 focus:ring-primary"
															rows={2}
														/>
														<div className="flex gap-1 mt-0.5">
															<button
																type="submit"
																className="p-0.5 rounded hover:bg-status-added/20 text-status-added"
																title="Save"
															>
																<Check className="h-3 w-3" />
															</button>
															<button
																type="button"
																onClick={cancelEditing}
																className="p-0.5 rounded hover:bg-muted text-muted-foreground"
																title="Cancel"
															>
																<X className="h-3 w-3" />
															</button>
														</div>
													</form>
												) : (
													<span className="block truncate text-foreground">
														{comment.content}
													</span>
												)}
											</div>
											{editingId !== comment.id && (
												<div className="flex gap-0.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
													{onSendComment && comment.status === "unsent" && (
														<button
															type="button"
															onClick={(e) => {
																e.stopPropagation();
																onSendComment(comment);
															}}
															className="p-0.5 rounded hover:bg-primary/20 text-muted-foreground hover:text-primary"
															title="Send"
														>
															<Send className="h-3 w-3" />
														</button>
													)}
													{onCopyComment && (
														<button
															type="button"
															onClick={(e) => {
																e.stopPropagation();
																onCopyComment(comment);
															}}
															className="p-0.5 rounded hover:bg-muted text-muted-foreground hover:text-foreground"
															title="Copy"
														>
															<Copy className="h-3 w-3" />
														</button>
													)}
													{onUpdateComment && (
														<button
															type="button"
															onClick={(e) => startEditing(e, comment)}
															className="p-0.5 rounded hover:bg-muted text-muted-foreground hover:text-foreground"
															title="Edit"
														>
															<Pencil className="h-3 w-3" />
														</button>
													)}
													{onDeleteComment && (
														<button
															type="button"
															onClick={(e) => handleDelete(e, comment.id)}
															className="p-0.5 rounded hover:bg-destructive/20 text-muted-foreground hover:text-destructive"
															title="Delete"
														>
															<Trash2 className="h-3 w-3" />
														</button>
													)}
												</div>
											)}
										</div>
									);
								})}
						</div>
					);
				})}
			</div>
		</ScrollArea>
	);
}
