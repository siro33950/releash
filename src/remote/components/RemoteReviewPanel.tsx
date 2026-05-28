import { CheckCircle2, MessageSquare, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import type { ReviewThread } from "@/types/protocol";

interface RemoteReviewPanelProps {
	threads: ReviewThread[];
	selectedThread: ReviewThread | null;
	selectedThreadId: string | null;
	loading: boolean;
	error: string | null;
	onSelectThread: (threadId: string) => void;
	onRefresh: () => void;
	onCreateThread: (content: string) => void;
	onAppendComment: (threadId: string, content: string) => void;
	onResolveThread: (threadId: string, summary: string) => void;
}

export function RemoteReviewPanel({
	threads,
	selectedThread,
	selectedThreadId,
	loading,
	error,
	onSelectThread,
	onRefresh,
	onCreateThread,
	onAppendComment,
	onResolveThread,
}: RemoteReviewPanelProps) {
	const [newThread, setNewThread] = useState("");
	const [reply, setReply] = useState("");
	const [resolveSummary, setResolveSummary] = useState("");
	const [pendingAction, setPendingAction] = useState<
		null | "create" | "reply" | "resolve"
	>(null);
	const responseKey = `${selectedThread?.id ?? ""}:${selectedThread?.version ?? ""}:${threads.length}:${error ?? ""}`;
	const previousResponseKeyRef = useRef(responseKey);

	useEffect(() => {
		if (previousResponseKeyRef.current === responseKey) return;
		previousResponseKeyRef.current = responseKey;
		setPendingAction(null);
	}, [responseKey]);

	return (
		<div className="h-full flex flex-col bg-background">
			<header className="h-11 px-3 border-b border-border flex items-center justify-between">
				<div className="flex items-center gap-2">
					<MessageSquare className="size-4" />
					<span className="text-sm font-medium">Threads</span>
				</div>
				<Button
					variant="ghost"
					size="icon-sm"
					aria-label="Refresh threads"
					onClick={onRefresh}
					disabled={loading}
				>
					<RefreshCw className="size-4" />
				</Button>
			</header>
			{error && (
				<div className="px-3 py-2 text-xs text-destructive border-b border-border">
					{error}
				</div>
			)}
			<div className="flex-1 min-h-0 grid grid-cols-[42%_58%]">
				<aside className="border-r border-border min-h-0 overflow-auto">
					<div className="p-2 border-b border-border">
						<Textarea
							value={newThread}
							onChange={(e) => setNewThread(e.target.value)}
							placeholder="Start a thread..."
							className="min-h-20 text-sm"
						/>
						<Button
							className="mt-2 h-8 w-full text-xs"
							disabled={newThread.trim() === "" || pendingAction === "create"}
							onClick={() => {
								setPendingAction("create");
								onCreateThread(newThread.trim());
								setNewThread("");
							}}
						>
							Create
						</Button>
					</div>
					{threads.map((thread) => (
						<button
							type="button"
							key={thread.id}
							onClick={() => onSelectThread(thread.id)}
							className={`w-full text-left px-3 py-2 border-b border-border ${
								thread.id === selectedThreadId
									? "bg-muted"
									: "hover:bg-muted/60"
							}`}
						>
							<div className="flex items-center gap-2">
								<span className="min-w-0 flex-1 text-xs font-medium truncate">
									{thread.comments[0]?.content ?? "(empty)"}
								</span>
								{thread.state === "resolved" && (
									<CheckCircle2 className="size-3.5 text-green-600" />
								)}
							</div>
							<div className="mt-1 text-[10px] text-muted-foreground">
								{thread.author.displayName} · {thread.comments.length} comments
							</div>
						</button>
					))}
				</aside>
				<section className="min-h-0 overflow-auto p-3">
					{selectedThread ? (
						<div className="space-y-3">
							<div className="space-y-2">
								{selectedThread.comments.map((comment) => (
									<div
										key={comment.id}
										className="rounded border border-border p-2 bg-card"
									>
										<div className="text-[10px] text-muted-foreground">
											{comment.author.kind} · {comment.author.displayName}
										</div>
										<p className="text-sm whitespace-pre-wrap break-words">
											{comment.content}
										</p>
									</div>
								))}
							</div>
							{selectedThread.resolve && (
								<div className="rounded border border-green-600/20 bg-green-600/10 px-2 py-1 text-[11px] text-green-700 dark:text-green-400">
									<div>
										{selectedThread.resolve.outcome} by{" "}
										{selectedThread.resolve.actor.kind} ·{" "}
										{selectedThread.resolve.actor.displayName}
									</div>
									<div className="break-words">
										{selectedThread.resolve.summary}
									</div>
								</div>
							)}
							{selectedThread.state === "open" && (
								<div className="space-y-2">
									<Textarea
										value={reply}
										onChange={(e) => setReply(e.target.value)}
										placeholder="Reply..."
										className="min-h-20 text-sm"
									/>
									<Button
										className="h-8 w-full text-xs"
										disabled={reply.trim() === "" || pendingAction === "reply"}
										onClick={() => {
											setPendingAction("reply");
											onAppendComment(selectedThread.id, reply.trim());
											setReply("");
										}}
									>
										Reply
									</Button>
								</div>
							)}
							{selectedThread.canResolve && selectedThread.state === "open" && (
								<div className="space-y-2">
									<input
										value={resolveSummary}
										onChange={(e) => setResolveSummary(e.target.value)}
										placeholder="Resolution summary"
										className="w-full h-8 px-2 rounded border border-input bg-background text-sm"
									/>
									<Button
										variant="outline"
										className="h-8 w-full text-xs"
										disabled={
											resolveSummary.trim() === "" ||
											pendingAction === "resolve"
										}
										onClick={() => {
											setPendingAction("resolve");
											onResolveThread(selectedThread.id, resolveSummary.trim());
											setResolveSummary("");
										}}
									>
										Resolve
									</Button>
								</div>
							)}
						</div>
					) : (
						<div className="h-full flex items-center justify-center text-sm text-muted-foreground">
							No thread selected
						</div>
					)}
				</section>
			</div>
		</div>
	);
}
