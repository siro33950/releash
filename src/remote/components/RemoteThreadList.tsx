import {
	Check,
	Copy,
	Eye,
	EyeOff,
	MessageSquare,
	Send,
	Trash2,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { Thread } from "@/types/thread";

interface RemoteThreadListProps {
	threads: Thread[];
	onSendToTerminal?: (threads: Thread[]) => void;
	onDeleteThread?: (threadId: string) => void;
	onResolveThread?: (threadId: string) => void;
	onAddEntry?: (threadId: string, content: string) => void;
	onCopyThread?: (thread: Thread) => void;
}

export function RemoteThreadList({
	threads,
	onSendToTerminal,
	onDeleteThread,
	onResolveThread,
	onAddEntry,
	onCopyThread,
}: RemoteThreadListProps) {
	const [showResolved, setShowResolved] = useState(false);
	const resolvedCount = threads.filter((t) => t.resolved).length;
	const unresolvedThreads = useMemo(
		() => threads.filter((t) => !t.resolved),
		[threads],
	);
	const visibleThreads = showResolved ? threads : unresolvedThreads;

	const [replyingId, setReplyingId] = useState<string | null>(null);
	const [replyContent, setReplyContent] = useState("");

	const submitReply = useCallback(() => {
		if (!replyingId || !replyContent.trim()) return;
		onAddEntry?.(replyingId, replyContent.trim());
		setReplyingId(null);
		setReplyContent("");
	}, [replyingId, replyContent, onAddEntry]);

	if (threads.length === 0) {
		return (
			<div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground px-6">
				<MessageSquare className="h-8 w-8" />
				<span className="text-sm font-medium">No threads</span>
				<p className="text-xs text-center leading-relaxed">
					Threads from the desktop app will appear here.
				</p>
			</div>
		);
	}

	const grouped = new Map<string, Thread[]>();
	for (const thread of visibleThreads) {
		const existing = grouped.get(thread.filePath);
		if (existing) {
			existing.push(thread);
		} else {
			grouped.set(thread.filePath, [thread]);
		}
	}

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-between px-3 py-2 border-b border-border bg-card shrink-0">
				<div className="flex items-center gap-2">
					<span className="text-xs text-muted-foreground">
						Threads
						{unresolvedThreads.length > 0 && (
							<span className="ml-1.5 px-1.5 py-0.5 text-[10px] bg-primary/20 text-primary rounded">
								{unresolvedThreads.length}
							</span>
						)}
					</span>
					{resolvedCount > 0 && (
						<button
							type="button"
							onClick={() => setShowResolved((prev) => !prev)}
							className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] text-muted-foreground rounded hover:bg-muted transition-colors"
						>
							{showResolved ? (
								<EyeOff className="h-3 w-3" />
							) : (
								<Eye className="h-3 w-3" />
							)}
							Resolved ({resolvedCount})
						</button>
					)}
				</div>
				{unresolvedThreads.length > 0 && onSendToTerminal && (
					<button
						type="button"
						onClick={() => onSendToTerminal(unresolvedThreads)}
						className="flex items-center gap-1.5 px-3 py-1.5 text-xs bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors min-h-[32px]"
					>
						<Send className="h-3.5 w-3.5" />
						Send
					</button>
				)}
			</div>
			<div className="flex-1 overflow-y-auto p-2">
				{[...grouped.entries()].map(([filePath, fileThreads]) => {
					const fileName = filePath.split("/").pop() ?? filePath;
					return (
						<div key={filePath} className="mb-3">
							<div className="text-xs font-medium px-2 py-1 text-secondary-foreground truncate">
								{fileName}
							</div>
							{fileThreads
								.sort((a, b) => a.lineNumber - b.lineNumber)
								.map((thread) => (
									<div
										key={thread.id}
										className="group px-2 py-2 rounded hover:bg-muted/50 transition-colors"
									>
										<div className="flex items-start gap-2">
											<MessageSquare className="h-4 w-4 shrink-0 mt-0.5 text-muted-foreground" />
											<div className="min-w-0 flex-1">
												<div className="flex items-center gap-1.5">
													<span className="text-muted-foreground font-mono text-xs">
														L{thread.lineNumber}
														{thread.endLine != null ? `-${thread.endLine}` : ""}
													</span>
													{thread.resolved && (
														<span className="text-[10px] px-1 rounded bg-success/20 text-success">
															resolved
														</span>
													)}
													{thread.entries.length > 1 && (
														<span className="text-[10px] px-1 rounded bg-muted text-muted-foreground">
															{thread.entries.length - 1}{" "}
															{thread.entries.length - 1 === 1
																? "reply"
																: "replies"}
														</span>
													)}
												</div>
												{thread.entries.map((entry) => (
													<div
														key={entry.id}
														className="mt-1 text-sm break-words"
													>
														{entry.authorName && (
															<span className="text-xs font-medium text-muted-foreground mr-1">
																{entry.authorName}:
															</span>
														)}
														<span
															className={
																entry.isAi ? "text-info" : "text-foreground"
															}
														>
															{entry.content}
														</span>
													</div>
												))}
												{replyingId === thread.id && (
													<div className="mt-2">
														<textarea
															ref={(el) => el?.focus()}
															value={replyContent}
															onChange={(e) => setReplyContent(e.target.value)}
															onKeyDown={(e) => {
																if (e.key === "Enter" && !e.shiftKey) {
																	e.preventDefault();
																	submitReply();
																}
																if (e.key === "Escape") {
																	setReplyingId(null);
																	setReplyContent("");
																}
															}}
															className="w-full px-2 py-1 text-sm bg-input border border-border rounded resize-none focus:outline-none focus:ring-1 focus:ring-ring text-foreground"
															rows={2}
															placeholder="Reply..."
														/>
														<div className="flex gap-2 mt-1">
															<button
																type="button"
																onClick={submitReply}
																className="px-2 py-1 text-xs bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
															>
																Reply
															</button>
															<button
																type="button"
																onClick={() => {
																	setReplyingId(null);
																	setReplyContent("");
																}}
																className="px-2 py-1 text-xs bg-secondary text-secondary-foreground rounded hover:bg-secondary/80 transition-colors"
															>
																Cancel
															</button>
														</div>
													</div>
												)}
											</div>
											{replyingId !== thread.id && (
												<div className="flex gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
													{onAddEntry && !thread.resolved && (
														<button
															type="button"
															onClick={() => {
																setReplyingId(thread.id);
																setReplyContent("");
															}}
															className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-secondary-foreground transition-colors"
															title="Reply"
														>
															<MessageSquare className="h-3.5 w-3.5" />
														</button>
													)}
													{onCopyThread && (
														<button
															type="button"
															onClick={() => onCopyThread(thread)}
															className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-secondary-foreground transition-colors"
															title="Copy"
														>
															<Copy className="h-3.5 w-3.5" />
														</button>
													)}
													{onResolveThread && !thread.resolved && (
														<button
															type="button"
															onClick={() => onResolveThread(thread.id)}
															className="p-1 rounded hover:bg-green-500/20 text-muted-foreground hover:text-green-500 transition-colors"
															title="Resolve"
														>
															<Check className="h-3.5 w-3.5" />
														</button>
													)}
													{onDeleteThread && (
														<button
															type="button"
															onClick={() => onDeleteThread(thread.id)}
															className="p-1 rounded hover:bg-destructive/20 text-muted-foreground hover:text-destructive transition-colors"
															title="Delete"
														>
															<Trash2 className="h-3.5 w-3.5" />
														</button>
													)}
												</div>
											)}
										</div>
									</div>
								))}
						</div>
					);
				})}
			</div>
		</div>
	);
}
