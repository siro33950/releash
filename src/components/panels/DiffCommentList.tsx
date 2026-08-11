import { CheckCircle2, MessageSquare, Send, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import {
	ReviewThreadHandoffFeedbackMessage,
	useReviewThreadHandoff,
} from "@/contexts/ReviewThreadHandoffContext";
import {
	getThreadEndLine,
	getThreadFilePath,
	getThreadInitialContent,
	getThreadLineNumber,
	type ReviewDiscussionThread,
	type ThreadNavigationTarget,
	toThreadNavigationTarget,
} from "@/types/diffComment";

export interface DiffCommentListProps {
	comments: ReviewDiscussionThread[];
	onThreadClick: (target: ThreadNavigationTarget) => void;
	onDelete?: (threadId: string) => Promise<void>;
}

function formatLineLabel(comment: ReviewDiscussionThread): string {
	const lineNumber = getThreadLineNumber(comment);
	const endLine = getThreadEndLine(comment);
	if (lineNumber == null) return "file";
	if (endLine != null && endLine !== lineNumber) {
		return `L${lineNumber}-${endLine}`;
	}
	return `L${lineNumber}`;
}

function getFileName(filePath: string): string {
	const parts = filePath.split("/");
	return parts[parts.length - 1] ?? filePath;
}

function SendThreadToAgentButton({ threadId }: { threadId: string }) {
	const { canCopy, copyThreadForAgent, feedback } = useReviewThreadHandoff();
	const [busy, setBusy] = useState(false);
	const title = canCopy
		? "Copy Diff Thread for Agent"
		: "Agent instruction copy unavailable";
	return (
		<>
			<Button
				type="button"
				variant="ghost"
				size="icon-xs"
				aria-label={title}
				title={title}
				disabled={busy || !canCopy}
				onClick={(e) => {
					e.stopPropagation();
					(async () => {
						setBusy(true);
						try {
							await copyThreadForAgent(threadId);
						} finally {
							setBusy(false);
						}
					})();
				}}
				className="shrink-0 opacity-0 group-hover/item:opacity-100 focus:opacity-100"
			>
				<Send className="size-3.5" />
			</Button>
			<ReviewThreadHandoffFeedbackMessage
				feedback={feedback}
				threadId={threadId}
			/>
		</>
	);
}

function DeleteThreadButton({
	threadId,
	onDelete,
}: {
	threadId: string;
	onDelete: (threadId: string) => Promise<void>;
}) {
	const [open, setOpen] = useState(false);
	const [busy, setBusy] = useState(false);

	return (
		<AlertDialog open={open} onOpenChange={setOpen}>
			<AlertDialogTrigger asChild>
				<Button
					type="button"
					variant="ghost"
					size="icon-xs"
					aria-label="Delete thread"
					title="Delete thread"
					disabled={busy}
					onClick={(e) => {
						e.stopPropagation();
					}}
					className="shrink-0 opacity-0 group-hover/item:opacity-100 focus:opacity-100"
				>
					<Trash2 className="size-3.5" />
				</Button>
			</AlertDialogTrigger>
			<AlertDialogContent
				onClick={(e) => {
					e.stopPropagation();
				}}
			>
				<AlertDialogHeader>
					<AlertDialogTitle>Delete this thread?</AlertDialogTitle>
					<AlertDialogDescription>
						This will remove the thread from the review list. The deletion is
						recorded in the thread history and cannot be undone from the UI.
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
					<AlertDialogAction
						disabled={busy}
						onClick={(e) => {
							e.preventDefault();
							e.stopPropagation();
							(async () => {
								setBusy(true);
								try {
									await onDelete(threadId);
									setOpen(false);
								} catch (error) {
									console.error("Failed to delete thread:", error);
								} finally {
									setBusy(false);
								}
							})();
						}}
					>
						Delete
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

export function DiffCommentList({
	comments,
	onThreadClick,
	onDelete,
}: DiffCommentListProps) {
	const groupedByFile = useMemo(() => {
		const map = new Map<string, ReviewDiscussionThread[]>();
		for (const comment of comments) {
			const filePath = getThreadFilePath(comment);
			const existing = map.get(filePath);
			if (existing) {
				existing.push(comment);
			} else {
				map.set(filePath, [comment]);
			}
		}
		return map;
	}, [comments]);

	if (comments.length === 0) {
		return (
			<div className="h-full flex flex-col">
				<div className="flex items-center justify-between px-3 py-1.5 border-b border-border shrink-0">
					<span className="text-xs font-medium text-foreground">Threads</span>
				</div>
				<div className="flex-1 flex items-center justify-center">
					<EmptyState
						icon={MessageSquare}
						title="No threads yet"
						description="Add comments on diff lines"
					/>
				</div>
			</div>
		);
	}

	return (
		<div className="h-full flex flex-col">
			<div className="flex items-center justify-between px-3 py-1.5 border-b border-border shrink-0">
				<span className="text-xs font-medium text-foreground">Threads</span>
			</div>
			<div className="flex-1 min-h-0 overflow-auto">
				<div className="px-2 py-1">
					{[...groupedByFile.entries()].map(([filePath, fileComments]) => (
						<div key={filePath || "general"} className="mb-2">
							<Tooltip>
								<TooltipTrigger asChild>
									<button
										type="button"
										onClick={() => {
											const first = fileComments[0];
											if (first) {
												onThreadClick(toThreadNavigationTarget(first));
											}
										}}
										className="w-full text-left px-1 py-0.5 text-[11px] font-medium text-muted-foreground hover:text-foreground truncate"
									>
										{filePath ? getFileName(filePath) : "General"}
									</button>
								</TooltipTrigger>
								<TooltipContent side="top" className="text-xs">
									{filePath || "General"}
								</TooltipContent>
							</Tooltip>
							<div className="space-y-0.5">
								{fileComments.map((comment) => (
									<div
										key={comment.id}
										className="group/item flex items-start gap-1 px-1.5 py-1 rounded hover:bg-muted/50 transition-colors"
									>
										<button
											type="button"
											onClick={() =>
												onThreadClick(toThreadNavigationTarget(comment))
											}
											className="flex-1 min-w-0 text-left flex items-start gap-1.5"
										>
											<span className="shrink-0 text-[10px] font-mono text-muted-foreground mt-0.5 min-w-[36px]">
												{formatLineLabel(comment)}
											</span>
											<span className="flex-1 text-xs text-foreground truncate">
												{getThreadInitialContent(comment)}
											</span>
											<span className="shrink-0 text-[10px] text-muted-foreground">
												{comment.comments.length}
											</span>
											{comment.state === "resolved" && (
												<span className="shrink-0 inline-flex items-center gap-0.5 text-[9px] bg-green-600/15 text-green-600 px-1 py-0.5 rounded-full">
													<CheckCircle2 className="h-2.5 w-2.5" />
													resolved
												</span>
											)}
										</button>
										<SendThreadToAgentButton threadId={comment.id} />
										{onDelete && (
											<DeleteThreadButton
												threadId={comment.id}
												onDelete={onDelete}
											/>
										)}
									</div>
								))}
							</div>
						</div>
					))}
				</div>
			</div>
		</div>
	);
}
