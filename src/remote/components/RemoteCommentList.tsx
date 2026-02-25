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
import type { LineComment } from "@/types/comment";

interface RemoteCommentListProps {
	comments: LineComment[];
	onSendToTerminal?: (comments: LineComment[]) => void;
	onDeleteComment?: (id: string) => void;
	onUpdateComment?: (id: string, content: string) => void;
	onSendComment?: (comment: LineComment) => void;
	onCopyComment?: (comment: LineComment) => void;
}

export function RemoteCommentList({
	comments,
	onSendToTerminal,
	onDeleteComment,
	onUpdateComment,
	onSendComment,
	onCopyComment,
}: RemoteCommentListProps) {
	const [showSentComments, setShowSentComments] = useState(false);
	const unsentComments = comments.filter((c) => c.status === "unsent");
	const sentCount = comments.filter((c) => c.status === "sent").length;
	const visibleComments = useMemo(
		() =>
			showSentComments ? comments : comments.filter((c) => c.status !== "sent"),
		[comments, showSentComments],
	);
	const [editingId, setEditingId] = useState<string | null>(null);
	const [editContent, setEditContent] = useState("");

	const startEditing = useCallback((comment: LineComment) => {
		setEditingId(comment.id);
		setEditContent(comment.content);
	}, []);

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

	if (visibleComments.length === 0 && sentCount === 0) {
		return (
			<div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground px-6">
				<MessageSquare className="h-8 w-8" />
				<span className="text-sm font-medium">コメントなし</span>
				<p className="text-xs text-center leading-relaxed">
					Diff画面のコメントボタンからコメントを追加できます
				</p>
			</div>
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
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-between px-3 py-2 border-b border-border bg-card shrink-0">
				<div className="flex items-center gap-2">
					<span className="text-xs text-muted-foreground">
						Comments
						{unsentComments.length > 0 && (
							<span className="ml-1.5 px-1.5 py-0.5 text-[10px] bg-primary/20 text-primary rounded">
								{unsentComments.length}
							</span>
						)}
					</span>
					{sentCount > 0 && (
						<button
							type="button"
							onClick={() => setShowSentComments((prev) => !prev)}
							className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] text-muted-foreground rounded hover:bg-muted transition-colors"
							data-testid="toggle-sent-comments"
						>
							{showSentComments ? (
								<EyeOff className="h-3 w-3" />
							) : (
								<Eye className="h-3 w-3" />
							)}
							送信済み ({sentCount})
						</button>
					)}
				</div>
				{unsentComments.length > 0 && onSendToTerminal && (
					<button
						type="button"
						onClick={() => onSendToTerminal(unsentComments)}
						className="flex items-center gap-1.5 px-3 py-1.5 text-xs bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors min-h-[32px]"
					>
						<Send className="h-3.5 w-3.5" />
						送信
					</button>
				)}
			</div>
			<div className="flex-1 overflow-y-auto p-2">
				{[...grouped.entries()].map(([filePath, fileComments]) => {
					const fileName = filePath.split("/").pop() ?? filePath;
					return (
						<div key={filePath} className="mb-3">
							<div className="text-xs font-medium px-2 py-1 text-secondary-foreground truncate">
								{fileName}
							</div>
							{fileComments
								.sort((a, b) => a.lineNumber - b.lineNumber)
								.map((comment) => (
									<div
										key={comment.id}
										className="group flex items-start gap-2 px-2 py-2 text-sm rounded hover:bg-muted/50 transition-colors"
									>
										<MessageSquare className="h-4 w-4 shrink-0 mt-0.5 text-muted-foreground" />
										<div className="min-w-0 flex-1">
											<div className="flex items-center gap-1.5">
												<span className="text-muted-foreground font-mono text-xs">
													L{comment.lineNumber}
													{comment.endLine != null ? `-${comment.endLine}` : ""}
												</span>
												<span
													className={`text-[10px] px-1 rounded ${
														comment.status === "sent"
															? "bg-success/20 text-success"
															: "bg-muted text-muted-foreground"
													}`}
												>
													{comment.status === "sent" ? "sent" : "unsent"}
												</span>
											</div>
											{editingId === comment.id ? (
												<div className="mt-1">
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
														className="w-full px-2 py-1 text-sm bg-input border border-border rounded resize-none focus:outline-none focus:ring-1 focus:ring-ring text-foreground"
														rows={2}
													/>
													<div className="flex gap-2 mt-1">
														<button
															type="button"
															onClick={submitEdit}
															className="flex items-center gap-1 px-2 py-1 text-xs bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
														>
															<Check className="h-3 w-3" />
															保存
														</button>
														<button
															type="button"
															onClick={cancelEditing}
															className="flex items-center gap-1 px-2 py-1 text-xs bg-secondary text-secondary-foreground rounded hover:bg-secondary/80 transition-colors"
														>
															<X className="h-3 w-3" />
															キャンセル
														</button>
													</div>
												</div>
											) : (
												<div className="text-foreground mt-0.5 break-words">
													{comment.content}
												</div>
											)}
										</div>
										{editingId !== comment.id && (
											<div className="flex gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
												{onSendComment && comment.status === "unsent" && (
													<button
														type="button"
														onClick={() => onSendComment(comment)}
														className="p-1 rounded hover:bg-primary/20 text-muted-foreground hover:text-primary transition-colors"
														title="送信"
													>
														<Send className="h-3.5 w-3.5" />
													</button>
												)}
												{onCopyComment && (
													<button
														type="button"
														onClick={() => onCopyComment(comment)}
														className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-secondary-foreground transition-colors"
														title="コピー"
													>
														<Copy className="h-3.5 w-3.5" />
													</button>
												)}
												{onUpdateComment && (
													<button
														type="button"
														onClick={() => startEditing(comment)}
														className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-secondary-foreground transition-colors"
														title="編集"
													>
														<Pencil className="h-3.5 w-3.5" />
													</button>
												)}
												{onDeleteComment && (
													<button
														type="button"
														onClick={() => onDeleteComment(comment.id)}
														className="p-1 rounded hover:bg-destructive/20 text-muted-foreground hover:text-destructive transition-colors"
														title="削除"
													>
														<Trash2 className="h-3.5 w-3.5" />
													</button>
												)}
											</div>
										)}
									</div>
								))}
						</div>
					);
				})}
			</div>
		</div>
	);
}
